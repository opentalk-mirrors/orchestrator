// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{ConnectInfo, OriginalUri, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use clap::Parser;
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
    sync::Mutex,
};
use transcription::TranscriptionInstance;
use url::Url;

use crate::{
    recorder::RecorderInstance,
    roomserver::token_store::TokenStore,
    service_instance::{registration::handle_socket, runner::InstanceCollection},
    settings::{Settings, monitoring::Monitoring},
    tasks::{ShutdownReceiver, Tasks},
};

mod cli;
mod livekit;
mod logging;
mod proxy;
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

#[derive(Debug, Clone)]
pub(crate) struct AppState {
    client: Client,
    public_url: Url,
    service_keys: ApiKeys,
    recorder_services: InstanceCollection<RecorderInstance>,
    roomserver_services: InstanceCollection<RoomserverInstance>,
    roomserver_tokens: Arc<Mutex<TokenStore>>,
    transcription_services: InstanceCollection<TranscriptionInstance>,
}

impl AppState {
    fn new(service_keys: ApiKeys, public_url: Url) -> Self {
        Self {
            client: Default::default(),
            public_url,
            service_keys,
            recorder_services: Default::default(),
            roomserver_services: Default::default(),
            roomserver_tokens: Arc::new(Mutex::new(TokenStore::new())),
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
    ensure_crypto_provider();
    let args = cli::Args::parse();

    // Initialize an early primitive logger until the settings are loaded
    let early_logger = logging::early_logging();

    if let Some(cmd) = args.cmd {
        cli::handle_subcommand(cmd, args.config).await?;

        return Ok(());
    }

    let settings = Settings::load(args.config.as_deref())?;

    drop(early_logger);
    logging::init_logging(settings.logging.as_ref());

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
            tracing::info!(
                "Monitoring is not configured, not serving /health, /ready or /startup endpoints"
            );
        }
    }

    tasks.wait_for_shutdown().await?;

    Ok(())
}

async fn run_webserver(settings: Settings, mut shutdown: ShutdownReceiver) -> Result<()> {
    let state = AppState::new(settings.services.keys, settings.http.public_url);

    let app = Router::new()
        .route("/metrics", get(metrics))
        .route("/register", any(register))
        .layer(settings.http.api_keys.auth_middleware()?)
        .nest("/livekit", livekit::routes())
        .nest("/roomserver/v1", roomserver::routes())
        .nest("/recording", opentalk_recorder_web_api::v1::routes())
        .with_state(state)
        .fallback(not_found_handler)
        .into_make_service_with_connect_info::<SocketAddr>();

    let address = format!("{}:{}", settings.http.address, settings.http.port);

    tracing::info!("Listening on address {address}");
    let listener = tokio::net::TcpListener::bind(address).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.wait_for_shutdown().await })
        .await?;

    Ok(())
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let recorder_services = state.recorder_services.read().await;
    let roomserver_services = state.roomserver_services.read().await;
    let transcription_services = state.transcription_services.read().await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "recorders": *recorder_services,
            "roomservers": *roomserver_services,
            "transcriptions": *transcription_services,
        })),
    )
}

async fn register(
    ws: WebSocketUpgrade,
    ConnectInfo(socket_addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> Response {
    ws.protocols(["opentalk-orchestrator-json-v1.0"])
        .on_upgrade(move |socket| handle_socket(socket, socket_addr, state))
}

async fn not_found_handler(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    tracing::debug!("Received request for unknown route: {}", uri);

    (StatusCode::NOT_FOUND, "requested route was not found")
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
        _ = signal::ctrl_c() => { tracing::debug!("received Ctrl-C"); }
        _ = sig_term.recv() => { tracing::debug!("received SIGTERM"); }
        _ = shutdown_signal.wait_for_shutdown() => {
            tracing::trace!("Shutdown handler received shutdown signal from application state");
            return;
        }
    }

    tracing::info!("Received shutdown signal...");
}

/// `rustls` and `jsonwebtoken` depend on a `CryptoProvider` being configured.
/// If no provider was explicitly configured, a provider will be derived from
/// the enabled features. Since there are many crates that depend on rustls and
/// `jsonwebtoken`, we don't have complete control over the enabled features.
/// If the configuration via feature is ambiguous these crates will panic.
///
/// Here we ensure that these crates are explicitly configured.
fn ensure_crypto_provider() {
    rustls::crypto::CryptoProvider::install_default(rustls::crypto::aws_lc_rs::default_provider())
        .expect("valid default crypto provider expected");

    jsonwebtoken::crypto::CryptoProvider::install_default(
        &jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER,
    )
    .expect("valid default crypto provider expected");
}
