// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use instance::Instance;
use opentalk_roomserver_web_api::v1::rooms;
use reqwest::Client;
use roomserver::RoomServerInstance;
use tokio::sync::Mutex;
use transcription::TranscriptionInstance;

use crate::recorder::RecorderInstance;

mod instance;
mod recorder;
mod roomserver;
mod transcription;

pub type Address = String;

#[derive(Debug, Clone, Default)]
pub(crate) struct AppState {
    client: Client,
    recorder_services: Arc<Mutex<HashMap<Address, RecorderInstance>>>,
    roomserver_services: Arc<Mutex<HashMap<Address, RoomServerInstance>>>,
    transcription_services: Arc<Mutex<HashMap<Address, TranscriptionInstance>>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let state = AppState::default();

    let app = Router::new()
        .route("/metrics", get(metrics))
        .route("/register", any(register))
        .nest("/roomserver", rooms::routes())
        .with_state(state);

    let address = "127.0.0.1:11222";
    log::info!("Listen on address {address}");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let recorder_services = state.recorder_services.lock().await.clone();
    let roomserver_services = state.roomserver_services.lock().await.clone();
    let transcription_services = state.transcription_services.lock().await.clone();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "recorders": recorder_services,
            "roomservers": roomserver_services,
            "transcriptions": transcription_services,
        })),
    )
}

async fn register(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.protocols(["opentalk-orchestrator-json-v1.0"])
        .on_upgrade(|socket| Instance::handle_socket(socket, state))
}
