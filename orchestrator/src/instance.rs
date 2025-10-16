// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use axum::extract::ws::Message;
use opentalk_orchestrator_shared::{Event, Register, RegisterType};
use tokio::{
    sync::Mutex,
    time::{Instant, Interval},
};

use crate::{
    Address, AppState, recorder::RecorderInstance, roomserver::RoomServerInstance,
    transcription::TranscriptionInstance,
};

const HEARTBEAT_TIMEOUT_SECONDS: u64 = 5;

pub(crate) struct Instance {
    pub(crate) socket: axum::extract::ws::WebSocket,
    pub(crate) address: Address,
    pub(crate) service_type: ServiceType,
}

pub(crate) enum ServiceType {
    Recorder(Arc<Mutex<HashMap<Address, RecorderInstance>>>),
    RoomServer(Arc<Mutex<HashMap<Address, RoomServerInstance>>>),
    Transcription(Arc<Mutex<HashMap<Address, TranscriptionInstance>>>),
}

impl Instance {
    pub(crate) async fn handle_socket(mut socket: axum::extract::ws::WebSocket, state: AppState) {
        let Some(Ok(register_message)) = socket.recv().await else {
            // TODO
            log::error!("");
            return;
        };
        let parse_message: Result<Register, _> = match register_message {
            Message::Text(utf8_bytes) => serde_json::from_str(&utf8_bytes),
            Message::Binary(bytes) => serde_json::from_slice(&bytes),
            _ => {
                // TODO
                log::error!("");
                return;
            }
        };
        let register = match parse_message {
            Ok(register) => register,
            Err(err) => {
                log::error!("unable to parse register message: {err:?}");
                return;
            }
        };
        log::debug!("received register message: {register:?}");

        let address = register.address;
        let mut this = match register.register_type {
            // TODO
            RegisterType::Recorder(_register_instance) => Instance {
                socket,
                address: address.clone(),
                service_type: ServiceType::Recorder(state.recorder_services),
            },
            RegisterType::RoomServer(register_instance) => {
                {
                    let mut guard = state.roomserver_services.lock().await;
                    let instance = guard.entry(address.clone()).or_default();

                    instance.metrics = register.metrics;
                    instance.rooms = register_instance.rooms;
                }

                Instance {
                    socket,
                    address: address.clone(),
                    service_type: ServiceType::RoomServer(state.roomserver_services),
                }
            }
            // TODO
            RegisterType::Transcription(_register_instance) => Instance {
                socket,
                address: address.clone(),
                service_type: ServiceType::Transcription(state.transcription_services),
            },
        };

        if let Err(err) = this.run().await {
            log::error!("run websocket connection for address '{address}' failed: {err:?}");
        }

        this.close().await;
    }

    async fn run(&mut self) -> Result<()> {
        let mut heartbeat = tokio::time::interval_at(
            Instant::now() + Duration::from_secs(HEARTBEAT_TIMEOUT_SECONDS),
            Duration::from_secs(5),
        );

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    log::error!("heartbeat timeout triggered, there was no heartbeat message within '{HEARTBEAT_TIMEOUT_SECONDS}' seconds");
                    break;
                }
                msg = self.socket.recv() => self.handle_message(msg, &mut heartbeat).await?
            }
        }

        Ok(())
    }

    async fn handle_message(
        &mut self,
        msg: Option<Result<Message, axum::Error>>,
        heartbeat: &mut Interval,
    ) -> Result<()> {
        let Some(msg) = msg else {
            bail!(
                "websocket for connetion '{}' closed unexpectedly",
                &self.address
            )
        };
        let msg = match msg {
            Ok(msg) => msg,
            Err(err) => bail!("received websocket error for connection '{}': {err:?}", {
                &self.address
            }),
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

        if let Event::Metrics(ref metrics) = event {
            log::trace!(
                "set metrics '{metrics:?}' for connection '{}'",
                self.address
            );

            log::trace!("reset heartbeat for connection '{}'", self.address);
            heartbeat.reset();

            match &mut self.service_type {
                ServiceType::Recorder(instances) => {
                    let mut guard = instances.lock().await;
                    let instance = guard.entry(self.address.clone()).or_default();
                    instance.metrics = metrics.clone();
                }
                ServiceType::RoomServer(instances) => {
                    let mut guard = instances.lock().await;
                    let instance = guard.entry(self.address.clone()).or_default();
                    instance.metrics = metrics.clone();
                }
                ServiceType::Transcription(instances) => {
                    let mut guard = instances.lock().await;
                    let instance = guard.entry(self.address.clone()).or_default();
                    instance.metrics = metrics.clone();
                }
            }
        }

        match &mut self.service_type {
            ServiceType::Recorder(instances) => {
                let mut guard = instances.lock().await;
                let instance = guard.entry(self.address.clone()).or_default();
                if let Event::Recorder(event) = &event {
                    instance.handle_event(event).await
                }
            }
            ServiceType::RoomServer(instances) => {
                let mut guard = instances.lock().await;
                let instance = guard.entry(self.address.clone()).or_default();
                if let Event::RoomServer(event) = &event {
                    instance.handle_event(event).await
                }
            }
            ServiceType::Transcription(instances) => {
                let mut guard = instances.lock().await;
                let instance = guard.entry(self.address.clone()).or_default();
                if let Event::Transcription(event) = &event {
                    instance.handle_event(event).await
                }
            }
        }

        Ok(())
    }

    async fn close(&mut self) {
        match &mut self.service_type {
            ServiceType::Recorder(instances) => {
                instances.lock().await.remove(&self.address);
            }
            ServiceType::RoomServer(instances) => {
                instances.lock().await.remove(&self.address);
            }
            ServiceType::Transcription(instances) => {
                instances.lock().await.remove(&self.address);
            }
        }
    }
}
