// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

pub mod client;
pub mod config;
pub mod signaling;

pub use client::OrchestratorClient;
pub use config::{Endpoint, OrchestratorConfig};
pub use opentalk_orchestrator_shared::{Metrics, RegisterType};
#[cfg(feature = "recording-service")]
pub use opentalk_orchestrator_shared::{RecorderEvent, RegisterRecorder};
#[cfg(feature = "roomserver-service")]
pub use opentalk_orchestrator_shared::{RegisterRoomServer, RoomServerEvent};
#[cfg(feature = "transcription-service")]
pub use opentalk_orchestrator_shared::{RegisterTranscription, TranscriptionEvent};

#[cfg(not(any(
    feature = "recording-service",
    feature = "roomserver-service",
    feature = "transcription-service"
)))]
compile_error!(
    "At least one service feature needs to be enabled (e.g. recording-service, roomserver-service, transcription-service)"
);
