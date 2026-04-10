// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::{
    collections::{HashSet, hash_map::Entry},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use opentalk_orchestrator_shared::{
    Metrics, RecorderResource, Register, RegisterResponse, RegisterType, ServiceAddress,
    error::RegistrationError,
};
use opentalk_service_auth::ApiKeyId;
use opentalk_types_common::rooms::RoomId;
use tokio::time::timeout;
use url::Url;

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

pub(crate) async fn handle_socket(mut socket: WebSocket, socket_addr: SocketAddr, state: AppState) {
    let Register {
        register_data,
        register_type,
    } = match receive_registration_message(&mut socket).await {
        Ok(msg) => msg,
        Err(err) => {
            tracing::error!("failed to register new service: {err}");

            if let Error::Registration(registration_error) = err {
                send_registration_response(&mut socket, registration_error).await;
            }

            close_socket(socket, close_code::NORMAL, "registration failed").await;

            return;
        }
    };

    let address = match register_data.service_address.clone() {
        ServiceAddress::Url(url) => url,
        ServiceAddress::Port(port) => {
            match Url::parse(&format!("http://{}:{}", socket_addr.ip(), port)) {
                Ok(url) => url,
                Err(err) => {
                    tracing::error!(
                        "failed to build service address from socket address and port: {err}"
                    );
                    send_registration_response(
                        &mut socket,
                        RegistrationError::InvalidServiceAddress,
                    )
                    .await;
                    close_socket(
                        socket,
                        close_code::NORMAL,
                        "failed to build service address",
                    )
                    .await;
                    return;
                }
            }
        }
    };

    tracing::debug!("Received registration request from {address}");

    let service_registration = ServiceRegistration {
        address,
        api_key_ids: register_data.api_key_ids,
        metrics: register_data.metrics,
    };

    match register_type {
        RegisterType::Recorder(service_data) => {
            state
                .run_recorder_instance(socket, service_registration, service_data.rooms)
                .await;
        }
        RegisterType::RoomServer(service_data) => {
            state
                .run_roomserver_instance(socket, service_registration, service_data.rooms)
                .await;
        }
        RegisterType::Transcription(_service_data) => {
            todo!()
        }
    };
}

/// Receive and parse the [`Register`] message on the given socket
async fn receive_registration_message(socket: &mut WebSocket) -> Result<Register, Error> {
    let Ok(message) = timeout(REGISTRATION_TIMEOUT, socket.recv()).await else {
        tracing::error!("did not receive registration message for {REGISTRATION_TIMEOUT:?}");
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
            tracing::debug!("failed to parse registration message: {err}");
            Err(RegistrationError::InvalidJson.into())
        }
    }
}

/// The data received from a service instance during registration
pub struct ServiceRegistration {
    /// The address of the service instance
    address: Url,
    /// The api key ids provided by the service instance
    api_key_ids: Vec<ApiKeyId>,
    /// The initial metrics provided by the service instance
    metrics: Metrics,
}

impl AppState {
    /// Register and run a new roomserver instance
    async fn run_roomserver_instance(
        &self,
        socket: WebSocket,
        registration: ServiceRegistration,
        rooms: HashSet<RoomId>,
    ) {
        let address = registration.address.clone();

        let runner = match self
            .create_instance_runner(
                socket,
                registration,
                rooms,
                Arc::clone(&self.roomserver_services),
            )
            .await
        {
            Ok(runner) => runner,
            Err(err) => {
                tracing::error!("failed to register roomserver ({address}): {err:?}");
                return;
            }
        };

        tracing::info!("successfully registered roomserver ({address})");

        match runner.run().await {
            Ok(()) => {
                tracing::info!(
                    "disconnected from roomserver ({address}), connection closed by service"
                );
            }
            Err(err) => {
                tracing::error!("unexpected disconnect from roomserver ({address}): {err:?}");
            }
        }
    }

    /// Register and run a new recorder instance
    async fn run_recorder_instance(
        &self,
        socket: WebSocket,
        registration: ServiceRegistration,
        rooms: HashSet<RecorderResource>,
    ) {
        let address = registration.address.clone();

        let runner = match self
            .create_instance_runner(
                socket,
                registration,
                rooms,
                Arc::clone(&self.recorder_services),
            )
            .await
        {
            Ok(runner) => runner,
            Err(err) => {
                tracing::error!("failed to register recorder ({address}): {err:?}");
                return;
            }
        };

        tracing::info!("successfully registered recorder ({address})");

        match runner.run().await {
            Ok(()) => {
                tracing::info!(
                    "disconnected from recorder ({address}), connection closed by service"
                );
            }
            Err(err) => {
                tracing::error!("unexpected disconnect from recorder ({address}): {err:?}");
            }
        }
    }

    /// Create the instance runner for the provided websocket
    async fn create_instance_runner<T: ServiceInstance>(
        &self,
        mut socket: WebSocket,
        registration: ServiceRegistration,
        managed_data: HashSet<T::ManagedResource>,
        instances: InstanceCollection<T>,
    ) -> Result<InstanceRunner<T>> {
        let address = registration.address.clone();

        if let Err(registration_error) = self
            .register_instance(registration, managed_data, Arc::clone(&instances))
            .await
        {
            let error_msg = registration_error.to_string();
            send_registration_response(&mut socket, registration_error).await;
            close_socket(socket, close_code::NORMAL, "registration failed").await;
            bail!(error_msg);
        }

        send_registration_response(&mut socket, RegisterResponse::Success).await;

        Ok(InstanceRunner::new(socket, address, instances))
    }

    /// Try to register a new service instance
    async fn register_instance<T: ServiceInstance>(
        &self,
        registration: ServiceRegistration,
        managed_data: HashSet<T::ManagedResource>,
        instances: InstanceCollection<T>,
    ) -> Result<(), RegistrationError> {
        if !self.knows_any_of(&registration.api_key_ids) {
            return Err(RegistrationError::UnknownApiKeyIds);
        }

        let mut guard = instances.write().await;
        let instance = match guard.entry(registration.address) {
            Entry::Occupied(_) => {
                return Err(RegistrationError::AddressAlreadyInUse);
            }
            Entry::Vacant(vacant_entry) => vacant_entry.insert(T::new(managed_data)),
        };

        let instance_data = instance.instance_data_mut();

        instance_data.metrics = registration.metrics;
        instance_data.api_key_ids = registration.api_key_ids;

        Ok(())
    }
}

async fn send_registration_response(socket: &mut WebSocket, response: impl Into<RegisterResponse>) {
    let response = response.into();

    match serde_json::to_string(&response) {
        Ok(error_message) => {
            if let Err(err) = socket.send(Message::Text(error_message.into())).await {
                tracing::error!("failed to send registration response: {err} ")
            }
        }
        Err(err) => {
            tracing::error!("failed to serialize registration response: {err}");
        }
    };
}

async fn close_socket<S: AsRef<str>>(mut socket: WebSocket, code: u16, reason: S) {
    if let Err(err) = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.as_ref().into(),
        })))
        .await
    {
        tracing::debug!("Failed to close websocket connection: {err}");
    };
}
