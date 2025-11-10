// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use core::fmt::Debug;
use std::collections::VecDeque;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use opentalk_orchestrator_shared::Register;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{
        Message as TtMessage, Utf8Bytes,
        client::IntoClientRequest,
        http::header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL},
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

use crate::config::OrchestratorConfig;

#[derive(Debug)]
pub(crate) struct Signaling {
    websocket: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    pending_pings: VecDeque<Bytes>,
}

impl Signaling {
    pub(crate) async fn connect(config: &OrchestratorConfig, register: Register) -> Result<Self> {
        let mut websocket_request = config
            .url
            .register_endpoint()?
            .to_string()
            .into_client_request()?;

        websocket_request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", config.api_key.generate_jwt()?).try_into()?,
        );

        websocket_request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            "opentalk-orchestrator-json-v1.0".to_string().try_into()?,
        );

        let (websocket, _) = tokio_tungstenite::connect_async(websocket_request)
            .await
            .context("failed create websocket connection")?;

        let mut this = Self {
            websocket: Some(websocket),
            pending_pings: VecDeque::new(),
        };

        this.send(register).await?;

        Ok(this)
    }

    pub(crate) async fn recv(&mut self) -> Result<()> {
        while let Some(data) = self.pending_pings.front() {
            self.websocket
                .as_mut()
                .unwrap()
                .send(TtMessage::Pong(data.clone()))
                .await?;
            self.pending_pings.pop_front();
        }

        loop {
            let Some(payload) = self.recv_websocket_message().await? else {
                return Ok(());
            };

            // TODO: trying to parse unit type, oof
            match serde_json::from_str(&payload) {
                Ok(payload) => return Ok(payload),
                Err(e) => log::debug!("Failed to parse event: {e:?}"),
            }
        }
    }

    // Receive any text data from the websocket and handle other events silently
    async fn recv_websocket_message(&mut self) -> Result<Option<Utf8Bytes>> {
        loop {
            match self
                .websocket
                .as_mut()
                .unwrap()
                .next()
                .await
                .transpose()
                .context("Failed to receive data from websocket")?
            {
                Some(TtMessage::Ping(data)) => {
                    self.pending_pings.push_back(data);
                }
                Some(TtMessage::Text(data)) => {
                    return Ok(Some(data));
                }
                Some(TtMessage::Close(c)) => {
                    log::debug!("Received close frame: {c:?}");
                    return Ok(None);
                }
                Some(_) => {}
                None => bail!("websocket connection closed"),
            }
        }
    }

    pub(crate) async fn send<P>(&mut self, payload: P) -> Result<()>
    where
        P: Into<outgoing::Command>,
    {
        self.websocket
            .as_mut()
            .unwrap()
            .send(TtMessage::Text(
                serde_json::to_string(&payload.into())?.into(),
            ))
            .await?;

        Ok(())
    }

    pub(crate) async fn close(&mut self) -> Result<()> {
        let mut websocket = self.websocket.take().unwrap();

        websocket
            .close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: Utf8Bytes::default(),
            }))
            .await
            .context("Failed to send websocket close frame")
    }
}

impl Drop for Signaling {
    fn drop(&mut self) {
        if let Some(mut websocket) = self.websocket.take() {
            tokio::spawn(async move {
                websocket.close(None).await.unwrap();
            });
        }
    }
}

pub(crate) mod outgoing {
    use opentalk_orchestrator_shared::{Event, Metrics, Register};
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    #[serde(untagged)]
    pub(crate) enum Command {
        Event(Event),
        Register(Register),
    }

    impl From<Event> for Command {
        fn from(event: Event) -> Self {
            Self::Event(event)
        }
    }

    impl From<Metrics> for Command {
        fn from(metrics: Metrics) -> Self {
            Self::Event(Event::Metrics(metrics))
        }
    }

    impl From<Register> for Command {
        fn from(register: Register) -> Self {
            Self::Register(register)
        }
    }
}
