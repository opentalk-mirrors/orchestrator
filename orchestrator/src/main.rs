// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use clap::Parser;
use opentalk_roomserver_web_api::v1::rooms;
use opentalk_service_auth::{ApiKey, ApiKeyId, service::ApiKeys};
use reqwest::Client;
use roomserver::RoomserverInstance;
use service_probe::{ServiceState, start_probe, stop_probe};
use tokio::{
    select,
    signal::{
        self,
        unix::{SignalKind, signal},
    },
};
use transcription::TranscriptionInstance;

use crate::{
    recorder::RecorderInstance,
    service_instance::{registration::handle_socket, runner::InstanceCollection},
    settings::{Settings, monitoring::Monitoring},
    tasks::{ShutdownReceiver, Tasks},
};

mod cli;
mod recorder;
mod roomserver;
mod service_instance;
mod settings;
mod tasks;
mod transcription;

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
    roomserver_services: InstanceCollection<RoomserverInstance>,
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
    let args = cli::Args::parse();

    tracing_subscriber::fmt::init();

    if let Some(cmd) = args.cmd {
        cli::handle_subcommand(cmd, args.config).await?;

        return Ok(());
    }

    let settings = Settings::load(args.config.as_deref())?;

    let mut tasks = Tasks::new();

    tasks.spawn("shutdown_signal_handler", |shutdown| async move {
        shutdown_signal_handler(shutdown).await;
        Ok(())
    });

    let settings_clone = settings.clone();
    tasks.spawn("webserver", |shutdown| {
        run_webserver(settings_clone, shutdown)
    });

    match settings.monitoring {
        Some(monitoring) => {
            tasks.spawn("service_probe", |shutdown| {
                start_service_probe(monitoring, shutdown)
            });
        }
        None => {
            log::info!(
                "Monitoring is not configured, not serving /health, /ready or /startup endpoints"
            );
        }
    }

    tasks.wait_for_shutdown().await?;

    Ok(())
}

async fn run_webserver(settings: Settings, mut shutdown: ShutdownReceiver) -> Result<()> {
    let state = AppState::new(settings.services.keys);

    let app = Router::new()
        .route("/metrics", get(metrics))
        .route("/register", any(register))
        .layer(settings.http.api_keys.auth_middleware()?)
        .nest("/roomserver/v1", rooms::routes())
        .with_state(state);

    let address = format!("{}:{}", settings.http.address, settings.http.port);

    log::info!("Listening on address {address}");
    let listener = tokio::net::TcpListener::bind(address).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.wait_for_shutdown().await })
        .await?;

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
        .on_upgrade(|socket| handle_socket(socket, state))
}

pub async fn start_service_probe(
    monitoring: Monitoring,
    mut shutdown: ShutdownReceiver,
) -> Result<()> {
    start_probe(monitoring.address, monitoring.port, ServiceState::Ready)
        .await
        .context("Failed to start monitotoring endpoint")?;

    shutdown.wait_for_shutdown().await;

    stop_probe().await;
    Ok(())
}

pub async fn shutdown_signal_handler(mut shutdown_signal: ShutdownReceiver) {
    let mut sig_term = signal(SignalKind::terminate()).expect("cannot setup SIGTERM handler");
    select! {
        _ = signal::ctrl_c() => { log::debug!("received Ctrl-C"); }
        _ = sig_term.recv() => { log::debug!("received SIGTERM"); }
        _ = shutdown_signal.wait_for_shutdown() => {
            log::trace!("Shutdown handler received shutdown signal from application state");
            return;
        }
    }

    log::info!("Received shutdown signal...");
}
