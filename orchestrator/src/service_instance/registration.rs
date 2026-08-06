// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail};
use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use opentalk_orchestrator_shared::{
    Metrics, Register, RegisterResponse, RegisterType, ServiceAddress, ServiceKind,
    error::RegistrationError,
};
use opentalk_service_auth::ApiKeyId;
use tokio::time::timeout;
use url::Url;

use crate::{AppState, service_instance::runner::InstanceRunner, storage::AddInstanceError};

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

    let service_kind = ServiceKind::from(&register_type);
    tracing::debug!("Received {service_kind} registration request from {address}");

    let service_registration = ServiceRegistration {
        address: address.clone(),
        register_type,
        api_key_ids: register_data.api_key_ids,
        metrics: register_data.metrics,
    };

    let runner = match state
        .create_instance_runner(socket, service_registration)
        .await
    {
        Ok(runner) => runner,
        Err(err) => {
            tracing::error!(
                "failed to create instance runner for {service_kind} ({address}): {err}"
            );
            return;
        }
    };

    match runner.run().await {
        Ok(()) => {
            tracing::info!(
                "disconnected from {service_kind} ({address}), connection closed by service"
            );
        }
        Err(err) => {
            tracing::error!("unexpected disconnect from {service_kind} ({address}): {err:?}");
        }
    }
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

#[derive(Debug, Clone)]
/// The data received from a service instance during registration
pub struct ServiceRegistration {
    /// The address of the service instance
    pub address: Url,
    /// The type of the service instance
    pub register_type: RegisterType,
    /// The api key ids provided by the service instance
    pub api_key_ids: Vec<ApiKeyId>,
    /// The initial metrics provided by the service instance
    pub metrics: Metrics,
}

impl AppState {
    /// Create the instance runner for the provided websocket
    async fn create_instance_runner(
        &self,
        mut socket: WebSocket,
        registration: ServiceRegistration,
    ) -> Result<InstanceRunner> {
        let address = registration.address.clone();
        let kind = ServiceKind::from(&registration.register_type);

        if let Err(registration_error) = self.register_instance(registration).await {
            let error_msg = registration_error.to_string();
            send_registration_response(&mut socket, registration_error).await;
            close_socket(socket, close_code::NORMAL, "registration failed").await;
            bail!(error_msg);
        }

        send_registration_response(&mut socket, RegisterResponse::Success).await;

        Ok(InstanceRunner::new(
            socket,
            address,
            kind,
            self.storage.clone(),
        ))
    }

    /// Try to register a new service instance
    async fn register_instance(
        &self,
        registration: ServiceRegistration,
    ) -> Result<(), RegistrationError> {
        if !self.knows_any_of(&registration.api_key_ids) {
            return Err(RegistrationError::UnknownApiKeyIds);
        }

        if let Err(e) = self.storage.add_instance(registration).await {
            return match e {
                AddInstanceError::RegistrationError(registration_error) => Err(registration_error),
                AddInstanceError::Internal(error) => {
                    tracing::error!("Failed to register service: {error}");
                    Err(RegistrationError::Internal)
                }
            };
        }
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
