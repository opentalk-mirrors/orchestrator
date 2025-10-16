// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::time::Duration;

use anyhow::Result;
use opentalk_orchestrator_shared::{Event, Metrics, Register, RegisterType};
use tokio::{sync::mpsc, time::Instant};

use crate::{config::OrchestratorConfig, signaling::Signaling};

const METRIC_INTERVAL: Duration = Duration::from_secs(1);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

pub trait StateProvider {
    fn register_type(&self) -> RegisterType;
    fn metrics(&self) -> Metrics;
}

/// The client to connect to the orchestrator service
pub struct OrchestratorClient {
    config: OrchestratorConfig,
}

impl OrchestratorClient {
    /// Creates a new [`OrchestratorClient`]
    pub async fn create(config: OrchestratorConfig) -> Result<Self> {
        Ok(Self { config })
    }

    /// Connect to the given orchestrator address to continuously sync service metrics
    ///
    /// Once connected, the client task will send an initial dump of available metrics. While
    /// connected, the metrics are sent in a fixed interval.
    ///
    /// When disconnected from the orchestrator, the underlying task will indefinitely attempt to
    /// reconnect every few seconds.
    ///
    /// The client exits gracefully when the returned sender is dropped.
    pub async fn connect<P, E>(&self, client_address: String, state_provider: P) -> mpsc::Sender<E>
    where
        P: StateProvider + Send + 'static,
        E: Into<Event> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<E>(32);

        tokio::spawn(run_client_task(
            self.config.clone(),
            client_address,
            state_provider,
            rx,
        ));

        tx
    }
}

/// Run the reconnect and event loop until the associated client sender is dropped
async fn run_client_task<P, E>(
    config: OrchestratorConfig,
    client_address: String,
    mut state_provider: P,
    mut rx: mpsc::Receiver<E>,
) where
    P: StateProvider + Send + 'static,
    E: Into<Event> + Send + 'static,
{
    loop {
        // Clear the channel to avoid pushing outdated events when reconnected
        loop {
            match rx.try_recv() {
                Ok(_) => continue,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    log::debug!("sender dropped, exiting connect task");
                    return;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
            }
        }

        log::info!(
            "trying to connect to orchestrator at {}...",
            config.endpoint.0
        );

        let signaling = match Signaling::connect(
            &config,
            Register {
                address: client_address.clone(),
                metrics: state_provider.metrics(),
                register_type: state_provider.register_type(),
            },
        )
        .await
        {
            Ok(signaling) => signaling,
            Err(err) => {
                log::error!(
                    "failed to connect to orchestrator, retrying in {RECONNECT_INTERVAL:?}: {err:?}"
                );
                tokio::time::sleep(RECONNECT_INTERVAL).await;
                continue;
            }
        };

        log::info!("connection to orchestrator established");

        match event_loop(&mut state_provider, &mut rx, signaling).await {
            Ok(SenderDropped) => {
                log::debug!("sender dropped, disconnecting from orchestrator");
                return;
            }
            Err(e) => {
                log::error!("disconnected from orchestrator: {e}");
                continue;
            }
        }
    }
}

/// Indicates that the clients sender got dropped
struct SenderDropped;

/// Forward the clients events to the orchestrator
async fn event_loop<P, E>(
    state_provider: &mut P,
    rx: &mut mpsc::Receiver<E>,
    mut signaling: Signaling,
) -> anyhow::Result<SenderDropped>
where
    P: StateProvider + Send + 'static,
    E: Into<Event> + Send + 'static,
{
    let mut interval = tokio::time::interval_at(Instant::now(), METRIC_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                signaling.send(state_provider.metrics()).await?;
            }
            data = rx.recv() => {
                match data {
                    Some(event) => {
                        signaling.send(event.into()).await?;
                    },
                    None => {
                        signaling.close().await?;
                        return Ok(SenderDropped)
                    },
                }
            }
            result = signaling.recv() => {
                let _: () = result?;
            }
        }
    }
}
