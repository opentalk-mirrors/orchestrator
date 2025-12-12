// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::time::Duration;

use async_trait::async_trait;
use opentalk_orchestrator_shared::{Event, Metrics, Register, RegisterData, RegisterType};
use opentalk_service_auth::ApiKeyId;
use tokio::{sync::mpsc, time::Instant};

use crate::{config::OrchestratorConfig, signaling::Signaling};

const METRIC_INTERVAL: Duration = Duration::from_secs(1);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);

#[async_trait]
pub trait StateProvider {
    async fn register_type(&self) -> RegisterType;
    async fn metrics(&self) -> Metrics;
}

/// The client to connect to the orchestrator service
pub struct OrchestratorClient {
    /// List of key ids that can be used to authorize requests to implementing service
    key_ids: Vec<ApiKeyId>,
    config: OrchestratorConfig,
    event_receiver: mpsc::Receiver<Event>,
}

/// Handle to send events to the orchestrator client task
#[derive(Debug, Clone)]
pub struct OrchestratorHandle(pub mpsc::Sender<Event>);

impl OrchestratorHandle {
    pub async fn send_event<E: Into<Event> + Send + 'static>(
        &self,
        event: E,
    ) -> anyhow::Result<()> {
        self.0
            .send(event.into())
            .await
            .map_err(|_| anyhow::anyhow!("Orchestrator client exited"))
    }
}

impl OrchestratorClient {
    /// Creates a new [`OrchestratorClient`]
    pub async fn create<K: IntoIterator<Item = ApiKeyId>>(
        config: OrchestratorConfig,
        key_ids: K,
    ) -> (Self, OrchestratorHandle) {
        let (event_sender, event_receiver) = mpsc::channel::<Event>(32);

        (
            Self {
                key_ids: key_ids.into_iter().collect(),
                config,
                event_receiver,
            },
            OrchestratorHandle(event_sender),
        )
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
    pub fn connect<P>(self, client_address: String, state_provider: P)
    where
        P: StateProvider + Send + 'static,
    {
        tokio::spawn(self.run(client_address, state_provider));
    }

    /// Run the reconnect and event loop until the associated client sender is dropped
    async fn run<P>(mut self, client_address: String, mut state_provider: P)
    where
        P: StateProvider + Send + 'static,
    {
        loop {
            // Clear the channel to avoid pushing outdated events when reconnected
            loop {
                match self.event_receiver.try_recv() {
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
                self.config.url
            );

            let signaling = match Signaling::connect(
                &self.config,
                Register {
                    register_data: RegisterData {
                        address: client_address.clone(),
                        api_key_ids: self.key_ids.clone(),
                        metrics: state_provider.metrics().await,
                    },
                    register_type: state_provider.register_type().await,
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

            match self.event_loop(&mut state_provider, signaling).await {
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

    /// Forward the clients events to the orchestrator
    async fn event_loop<P>(
        &mut self,
        state_provider: &mut P,
        mut signaling: Signaling,
    ) -> anyhow::Result<SenderDropped>
    where
        P: StateProvider + Send + 'static,
    {
        let mut interval = tokio::time::interval_at(Instant::now(), METRIC_INTERVAL);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    signaling.send(state_provider.metrics().await).await?;
                }
                data = self.event_receiver.recv() => {
                    match data {
                        Some(event) => {
                            signaling.send::<Event>(event).await?;
                        },
                        None => {
                            signaling.close().await;
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
}

/// Indicates that the clients sender got dropped
struct SenderDropped;
