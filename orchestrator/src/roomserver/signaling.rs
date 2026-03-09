// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use anyhow::Context;
use axum::{
    Router,
    extract::{Path, State, WebSocketUpgrade, ws::WebSocket},
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use opentalk_types_api_common::error::ApiError;
use opentalk_types_common::roomserver::Token;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type RoomserverSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/signaling/{token}", get(roomserver_signaling))
}

pub async fn roomserver_signaling(
    Path(token): Path<Token>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let mut token_store = state.roomserver_tokens.lock().await;
    let Some(room) = token_store.consume_token(&token) else {
        return Err(ApiError::forbidden().with_message("invalid token"));
    };
    drop(token_store);

    let roomserver_instances = state.roomserver_services.read().await;
    let (url, _) = roomserver_instances
        .iter()
        .find(|(_, instance)| instance.rooms.contains(&room))
        .ok_or_else(|| {
            tracing::error!(
                "Received valid roomserver token {token} but could not find associated roomserver for room {room}"
            );

            ApiError::internal()
        })?;

    let signaling_path = format!("/v1/signaling/{token}");
    let mut signaling_url = match url.join(&signaling_path) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!("failed to build roomserver signaling url: {e}");
            return Err(
                ApiError::internal().with_message("failed to build roomserver signaling url")
            );
        }
    };
    drop(roomserver_instances);

    match signaling_url.scheme() {
        "https" => signaling_url
            .set_scheme("wss")
            .expect("wss is a valid scheme"),
        "http" => signaling_url
            .set_scheme("ws")
            .expect("ws is a valid scheme"),
        scheme => {
            tracing::error!("Invalid scheme in roomserver signaling url, {scheme:?}");
            return Err(ApiError::internal());
        }
    }

    let (roomserver, _) = tokio_tungstenite::connect_async(signaling_url.as_str())
        .await
        .map_err(|e| {
            tracing::error!("failed to connect to roomserver signaling: {e}");

            ApiError::internal()
                .with_message(format!("failed to connect to roomserver instance: {e}"))
        })?;

    let response = ws.on_upgrade(|client| proxy_signaling_task(client, roomserver));

    Ok(response)
}

async fn proxy_signaling_task(client: WebSocket, roomserver: RoomserverSocket) {
    if let Err(e) = forward_signaling_messages(client, roomserver).await {
        tracing::debug!("Proxy task exited with error: {e:?}");
    }
}

async fn forward_signaling_messages(
    mut client: WebSocket,
    mut roomserver: RoomserverSocket,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            msg = client.recv() => {
                let msg = msg.context("Client closed connection")?.context("Error on client connection")?;

                let Some(msg) = translation::axum_to_tungstenite(msg) else {
                    continue;
                };

                roomserver.send(msg).await?;
            }
            msg = roomserver.next() => {
                let msg = msg.context("Roomserver closed connection")?.context("Error on roomserver connection")?;

                let Some(msg) = translation::tungstenite_to_axum(msg) else {
                    continue;
                };

                client.send(msg).await?;
            }
        }
    }
}

mod translation {
    use axum::extract::ws::{
        CloseFrame as AxumCloseFrame, Message as AxumMessage, Utf8Bytes as AxumUtf8Bytes,
    };
    use bytes::Bytes;
    use tokio_tungstenite::tungstenite::{
        Message as TungsteniteMessage,
        protocol::frame::{
            CloseFrame as TungsteniteCloseFrame, Utf8Bytes as TungsteniteUtf8Bytes,
            coding::CloseCode as TungsteniteCloseCode,
        },
    };

    pub fn axum_to_tungstenite(msg: AxumMessage) -> Option<TungsteniteMessage> {
        match msg {
            AxumMessage::Text(text) => Some(TungsteniteMessage::Text(
                // Safety: UTF8 invariant is uphold because the source type is guaranteed to be
                // UTF8
                unsafe { TungsteniteUtf8Bytes::from_bytes_unchecked(Bytes::from(text)) },
            )),
            AxumMessage::Binary(bin) => Some(TungsteniteMessage::Binary(bin)),
            AxumMessage::Close(close_frame) => {
                Some(TungsteniteMessage::Close(close_frame.map(|axum_close| {
                    TungsteniteCloseFrame {
                        code: TungsteniteCloseCode::from(axum_close.code),
                        reason: TungsteniteUtf8Bytes::from(axum_close.reason.as_str()),
                    }
                })))
            }
            AxumMessage::Ping(ping) => Some(TungsteniteMessage::Ping(ping)),
            // tungstenite automatically responds to pings, forwarding pongs would result in two
            // pongs on the receiver
            AxumMessage::Pong(_) => None,
        }
    }

    pub fn tungstenite_to_axum(msg: TungsteniteMessage) -> Option<AxumMessage> {
        match msg {
            TungsteniteMessage::Text(text) => {
                Some(AxumMessage::Text(
                    // Safety: UTF8 invariant is uphold because the source type is guaranteed to be
                    // UTF8
                    AxumUtf8Bytes::try_from(Bytes::from(text)).expect("valid UTF8 bytes"),
                ))
            }
            TungsteniteMessage::Binary(bin) => Some(AxumMessage::Binary(bin)),
            TungsteniteMessage::Close(close_frame) => {
                Some(AxumMessage::Close(close_frame.map(|tungstenite_close| {
                    AxumCloseFrame {
                        code: axum::extract::ws::CloseCode::from(tungstenite_close.code),
                        reason: AxumUtf8Bytes::from(tungstenite_close.reason.as_str()),
                    }
                })))
            }
            TungsteniteMessage::Ping(ping) => Some(AxumMessage::Ping(ping)),
            // tungstenite automatically responds to pings, forwarding pongs would result in two
            // pongs on the receiver
            TungsteniteMessage::Pong(_) => None,
            TungsteniteMessage::Frame(_) => {
                tracing::warn!(
                    "Received raw tungstenite frame in websocket proxy, this was unexpected"
                );
                None
            }
        }
    }
}
