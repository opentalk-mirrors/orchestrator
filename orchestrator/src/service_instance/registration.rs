// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::{
    collections::{HashSet, hash_map::Entry},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use opentalk_orchestrator_shared::{
    Register, RegisterData, RegisterResponse, RegisterType, error::RegistrationError,
};
use opentalk_types_common::rooms::RoomId;
use tokio::time::timeout;

use crate::{
    AppState,
    service_instance::{
        ServiceInstance,
        runner::{InstanceCollection, InstanceRunner},
    },
};

pub(crate) const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Local error type for the registration process
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Registration(#[from] RegistrationError),
    #[error("connection dropped by client")]
    ConnectionDropped,
    #[error("websocket error {0}")]
    WebSocket(#[from] axum::Error),
}

pub(crate) async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let Register {
        register_data,
        register_type,
    } = match receive_register_message(&mut socket).await {
        Ok(msg) => msg,
        Err(err) => {
            log::error!("failed to register new service: {err:?}");

            if let Error::Registration(registration_error) = err {
                send_registration_error(&mut socket, registration_error).await;
                close_socket(socket, close_code::NORMAL, "registration failed").await;
            }

            return;
        }
    };

    let address = register_data.address.clone();

    log::debug!("Received registration request from {address}");

    match register_type {
        RegisterType::Recorder(_service_data) => {
            todo!()
        }
        RegisterType::RoomServer(service_data) => {
            state
                .run_roomserver_instance(socket, register_data, service_data.rooms)
                .await;
        }
        RegisterType::Transcription(_service_data) => {
            todo!()
        }
    };
}

/// Receive and parse the [`Register`] message on the given socket
async fn receive_register_message(socket: &mut WebSocket) -> Result<Register, Error> {
    let Ok(message) = timeout(REGISTRATION_TIMEOUT, socket.recv()).await else {
        log::error!("did not receive registration message for {REGISTRATION_TIMEOUT:?}");
        return Err(RegistrationError::Timeout.into());
    };

    let Some(message) = message.transpose()? else {
        return Err(Error::ConnectionDropped);
    };

    let parse_message: Result<Register, _> = match message {
        Message::Text(utf8_bytes) => serde_json::from_str(&utf8_bytes),
        Message::Binary(bytes) => serde_json::from_slice(&bytes),
        _ => return Err(RegistrationError::InvalidMessageType.into()),
    };

    match parse_message.context("unable to parse register message") {
        Ok(message) => Ok(message),
        Err(err) => {
            log::debug!("failed to parse registration message: {err}");
            Err(RegistrationError::InvalidJson.into())
        }
    }
}

impl AppState {
    /// Register and run a new roomserver instance
    async fn run_roomserver_instance(
        &self,
        socket: WebSocket,
        register: RegisterData,
        rooms: HashSet<RoomId>,
    ) {
        let address = register.address.clone();

        let runner = match self
            .create_instance_runner(
                socket,
                register,
                rooms,
                Arc::clone(&self.roomserver_services),
            )
            .await
        {
            Ok(runner) => runner,
            Err(err) => {
                log::error!("failed to register roomserver({address}): {err:?}");
                return;
            }
        };

        log::info!("successfully registered roomserver({address})");

        match runner.run().await {
            Ok(()) => {
                log::info!("disconnected from roomserver({address}), connection closed by service");
            }
            Err(err) => {
                log::error!("unexpected disconnect from roomserver({address}): {err:?}");
            }
        }
    }

    /// Create the instance runner for the provided websocket
    async fn create_instance_runner<T: ServiceInstance>(
        &self,
        mut socket: WebSocket,
        register: RegisterData,
        managed_data: HashSet<T::ManagedResource>,
        instances: InstanceCollection<T>,
    ) -> Result<InstanceRunner<T>> {
        let address = register.address.clone();

        if let Err(registration_error) = self
            .register_instance(register, managed_data, Arc::clone(&instances))
            .await
        {
            let error_msg = registration_error.to_string();
            send_registration_error(&mut socket, registration_error).await;
            close_socket(socket, close_code::NORMAL, "registration failed").await;
            bail!(error_msg);
        }

        Ok(InstanceRunner::new(socket, address, instances))
    }

    /// Try to register a new service instance
    async fn register_instance<T: ServiceInstance>(
        &self,
        register: RegisterData,
        managed_data: HashSet<T::ManagedResource>,
        instances: InstanceCollection<T>,
    ) -> Result<(), RegistrationError> {
        if !self.knows_any_of(&register.api_key_ids) {
            return Err(RegistrationError::UnknownApiKeyIds);
        }

        let mut guard = instances.lock().await;

        let instance = match guard.entry(register.address.clone()) {
            Entry::Occupied(_) => {
                return Err(RegistrationError::AddressAlreadyInUse);
            }
            Entry::Vacant(vacant_entry) => vacant_entry.insert(T::new(managed_data)),
        };

        let instance_data = instance.instance_data_mut();

        instance_data.metrics = register.metrics;
        instance_data.api_key_ids = register.api_key_ids;

        Ok(())
    }
}

async fn send_registration_error(socket: &mut WebSocket, error: RegistrationError) {
    match serde_json::to_string(&RegisterResponse::Error(error)) {
        Ok(error_message) => {
            if let Err(err) = socket.send(Message::Text(error_message.into())).await {
                log::error!("failed to send registration error response: {err:?} ")
            }
        }
        Err(err) => {
            log::error!("failed to serialize registration response: {err:?}");
        }
    };
}

async fn close_socket<S: AsRef<str>>(mut socket: WebSocket, code: u16, reason: S) {
    const SOCKET_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

    if let Err(err) = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.as_ref().into(),
        })))
        .await
    {
        log::debug!("Failed to close websocket connection: {err:?}");
        return;
    };

    // Wait a few seconds for the websocket close response
    tokio::spawn(async move {
        let mut timeout = std::pin::pin!(tokio::time::sleep(SOCKET_CLOSE_TIMEOUT));

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    return;
                },
                Some(Ok(msg)) = socket.recv() => {
                    if let Message::Close(_) = msg { return }
                }
            }
        }
    });
}
