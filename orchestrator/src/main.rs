// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use instance_runner::InstanceRunner;
use opentalk_roomserver_web_api::v1::rooms;
use opentalk_service_auth::{ApiKey, ApiKeyId, service::ApiKeys};
use reqwest::Client;
use roomserver::RoomServerInstance;
use transcription::TranscriptionInstance;

use crate::{instance_runner::InstanceCollection, recorder::RecorderInstance};

mod instance_runner;
mod instance_selector;
mod recorder;
mod roomserver;
mod transcription;

pub type Address = String;

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ServiceType {
    Recorder,
    Roomserver,
    Transcription,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AppState {
    client: Client,
    service_keys: ApiKeys,
    recorder_services: InstanceCollection<RecorderInstance>,
    roomserver_services: InstanceCollection<RoomServerInstance>,
    transcription_services: InstanceCollection<TranscriptionInstance>,
}

impl AppState {
    fn new(service_keys: ApiKeys) -> Self {
        Self {
            client: Default::default(),
            service_keys,
            recorder_services: Default::default(),
            roomserver_services: Default::default(),
            transcription_services: Default::default(),
        }
    }

    /// Returns true if any of the provided service key ids is known to the orchestrator
    fn knows_any_of(&self, service_key_ids: &[ApiKeyId]) -> bool {
        for key_id in service_key_ids {
            let is_known = self
                .service_keys
                .inner()
                .iter()
                .any(|cred| &cred.id == key_id);

            if is_known {
                return true;
            }
        }

        false
    }

    /// Returns the first viable API key
    fn get_api_key_for_key_ids(&self, service_key_ids: &[ApiKeyId]) -> Option<ApiKey> {
        for key_id in service_key_ids {
            if let Some(key) = self
                .service_keys
                .inner()
                .iter()
                .find(|cred| &cred.id == key_id)
            {
                return Some(key.clone());
            }
        }

        None
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // TODO: provide this with a config
    let orchestrator_keys = ApiKeys::new(vec![ApiKey::new("orchestrator", "secret")]);
    let service_keys = ApiKeys::new(vec![ApiKey::new("roomserver", "secret")]);

    let state = AppState::new(service_keys);

    let app = Router::new()
        .route("/metrics", get(metrics))
        .route("/register", any(register))
        .layer(orchestrator_keys.auth_middleware()?)
        .nest("/roomserver", rooms::routes())
        .with_state(state);

    let address = "127.0.0.1:11222";
    log::info!("Listening on address {address}");
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
        .on_upgrade(|socket| InstanceRunner::handle_socket(socket, state))
}
