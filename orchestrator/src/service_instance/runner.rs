// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::{collections::HashMap, fmt::Debug, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use axum::extract::ws::Message;
use opentalk_orchestrator_shared::Event;
use tokio::{
    sync::RwLock,
    time::{Instant, Interval},
};
use url::Url;

use crate::service_instance::ServiceInstance;

const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) type InstanceCollection<T> = Arc<RwLock<HashMap<Url, T>>>;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Connection has been closed by the client")]
    ClosedByClient,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The runner for a service instance
pub(crate) struct InstanceRunner<T: ServiceInstance> {
    /// The websocket connection to the instance
    pub(crate) socket: axum::extract::ws::WebSocket,
    /// The HTTP address of the instance
    pub(crate) client_address: Url,
    /// A reference counter to the list of global service instances of kind `T`
    ///
    /// When a runner exits, the associated service instance gets removed from this collection.
    pub(crate) instances: InstanceCollection<T>,
}

impl<T: ServiceInstance> InstanceRunner<T> {
    pub(crate) fn new(
        socket: axum::extract::ws::WebSocket,
        client_address: Url,
        instances: InstanceCollection<T>,
    ) -> Self {
        Self {
            socket,
            client_address,
            instances,
        }
    }

    /// Run the event loop until the connection is closed by the service
    pub(crate) async fn run(mut self) -> Result<()> {
        let result = self.inner_run().await;

        self.remove_associated_instance().await;

        if let Err(e) = result {
            match e {
                Error::ClosedByClient => return Ok(()),
                Error::Other(error) => return Err(error),
            }
        }

        Ok(())
    }

    async fn inner_run(&mut self) -> Result<(), Error> {
        let mut heartbeat =
            tokio::time::interval_at(Instant::now() + HEARTBEAT_TIMEOUT, HEARTBEAT_TIMEOUT);

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    tracing::error!("heartbeat timeout({HEARTBEAT_TIMEOUT:?}) triggered");
                    break;
                }
                msg = self.socket.recv() => self.handle_message(msg, &mut heartbeat).await?
            }
        }

        Ok(())
    }

    /// Handle an incoming websocket message
    async fn handle_message(
        &mut self,
        msg: Option<Result<Message, axum::Error>>,
        heartbeat: &mut Interval,
    ) -> Result<(), Error> {
        let msg = msg
            .context("socket unexpectedly closed by client")?
            .context("error on websocket connection")?;

        let event: Event = match msg {
            Message::Close(close_frame) => {
                tracing::debug!(
                    "received close message {close_frame:?} for websocket connection ({})",
                    self.client_address
                );
                return Err(Error::ClosedByClient);
            }
            Message::Text(utf8_bytes) => {
                serde_json::from_str(utf8_bytes.as_str()).with_context(|| {
                    format!(
                        "failed to parse message for connection '{}':\n{utf8_bytes}",
                        self.client_address
                    )
                })?
            }
            Message::Binary(bytes) => serde_json::from_slice(bytes.iter().as_slice())
                .with_context(|| {
                    format!(
                        "failed parse to message for connection '{}'",
                        self.client_address
                    )
                })?,

            _ => {
                return Ok(());
            }
        };

        let mut guard = self.instances.write().await;

        let Some(instance) = guard.get_mut(&self.client_address) else {
            return Err(anyhow!(
                "Failed to get service instance for connected service ({})",
                self.client_address
            )
            .into());
        };

        match event {
            Event::Metrics(metrics) => {
                tracing::trace!(
                    "set metrics '{metrics:?}' for connection '{}'",
                    self.client_address
                );

                tracing::trace!("reset heartbeat for connection '{}'", self.client_address);
                heartbeat.reset();

                instance.instance_data_mut().metrics = metrics;
            }
            event => {
                let service_event = match event.try_into() {
                    Ok(event) => event,
                    Err(_) => {
                        return Err(
                            anyhow!("Received unexpected event variant from service").into()
                        );
                    }
                };

                instance.handle_event(service_event).await
            }
        }

        Ok(())
    }

    /// Remove the associated service instance from the global [`InstanceCollection`]
    async fn remove_associated_instance(&mut self) {
        self.instances.write().await.remove(&self.client_address);
    }
}
