// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use axum::extract::ws::Message;
use opentalk_orchestrator_shared::Event;
use tokio::{
    sync::Mutex,
    time::{Instant, Interval},
};

use crate::{Address, instance::ServiceInstance};

const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) type InstanceCollection<T> = Arc<Mutex<HashMap<Address, T>>>;

/// The runner for a service instance
pub(crate) struct InstanceRunner<T: ServiceInstance> {
    /// The websocket connection to the instance
    pub(crate) socket: axum::extract::ws::WebSocket,
    /// The HTTP address of the instance
    pub(crate) address: Address,
    /// A reference counter to the list of global service instances of kind `T`
    ///
    /// When the runner exits, the associated service instance gets removed from this collection.
    pub(crate) instances: InstanceCollection<T>,
}

impl<T: ServiceInstance> InstanceRunner<T> {
    pub(crate) fn new(
        socket: axum::extract::ws::WebSocket,
        address: String,
        instances: InstanceCollection<T>,
    ) -> Self {
        Self {
            socket,
            address,
            instances,
        }
    }

    /// Run the event loop until the connection is closed by the service
    pub(crate) async fn run(mut self) -> Result<()> {
        let result = self.inner_run().await;

        self.remove_associated_instance().await;

        result
    }

    async fn inner_run(&mut self) -> Result<()> {
        let mut heartbeat =
            tokio::time::interval_at(Instant::now() + HEARTBEAT_TIMEOUT, HEARTBEAT_TIMEOUT);

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    log::error!("heartbeat timeout({HEARTBEAT_TIMEOUT:?}) triggered");
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
    ) -> Result<()> {
        let Some(msg) = msg.transpose().context("error on websocket connection")? else {
            bail!("socket unexpectedly closed by client",)
        };

        let event: Event = match msg {
            Message::Ping(bytes) => {
                self.socket.send(Message::Pong(bytes)).await?;
                return Ok(());
            }
            Message::Pong(_bytes) => return Ok(()),
            Message::Close(close_frame) => {
                log::debug!(
                    "received close frame, close the websocket connection to {}",
                    self.address
                );
                self.socket.send(Message::Close(close_frame)).await?;
                return Ok(());
            }
            Message::Text(utf8_bytes) => serde_json::from_str(utf8_bytes.as_str())
                .with_context(|| format!("parse message for connection '{}'", self.address))?,
            Message::Binary(bytes) => serde_json::from_slice(bytes.iter().as_slice())
                .with_context(|| format!("parse message for connection '{}'", self.address))?,
        };

        let mut guard = self.instances.lock().await;

        let Some(instance) = guard.get_mut(&self.address) else {
            bail!(
                "Failed to get service instance for connected service ({})",
                self.address
            )
        };

        match event {
            Event::Metrics(metrics) => {
                log::trace!(
                    "set metrics '{metrics:?}' for connection '{}'",
                    self.address
                );

                log::trace!("reset heartbeat for connection '{}'", self.address);
                heartbeat.reset();

                instance.instance_data_mut().metrics = metrics;
            }
            event => {
                let Ok(service_event) = event.try_into() else {
                    // event mismatch
                    todo!()
                };

                instance.handle_event(service_event).await
            }
        }
        Ok(())
    }

    /// Remove the associated service instance from the global [`InstanceCollection`]
    async fn remove_associated_instance(&mut self) {
        self.instances.lock().await.remove(&self.address);
    }
}
