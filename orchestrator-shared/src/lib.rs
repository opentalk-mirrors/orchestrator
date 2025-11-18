// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use serde::{Deserialize, Serialize};

#[cfg(feature = "recording-service")]
pub use crate::recorder::{RecorderEvent, RegisterRecorder};
#[cfg(feature = "roomserver-service")]
pub use crate::roomserver::{RegisterRoomServer, RoomServerEvent};
#[cfg(feature = "transcription-service")]
pub use crate::transcription::{RegisterTranscription, TranscriptionEvent};

#[cfg(feature = "recording-service")]
mod recorder;
#[cfg(feature = "roomserver-service")]
mod roomserver;
#[cfg(feature = "transcription-service")]
mod transcription;

#[derive(Debug, Serialize, Deserialize)]
pub struct Register {
    pub address: String,
    pub metrics: Metrics,
    pub register_type: RegisterType,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub load: u8,
    pub accepting_jobs: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RegisterType {
    #[cfg(feature = "recording-service")]
    Recorder(RegisterRecorder),
    #[cfg(feature = "roomserver-service")]
    RoomServer(RegisterRoomServer),
    #[cfg(feature = "transcription-service")]
    Transcription(RegisterTranscription),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    Metrics(Metrics),
    #[cfg(feature = "recording-service")]
    Recorder(RecorderEvent),
    #[cfg(feature = "roomserver-service")]
    RoomServer(RoomServerEvent),
    #[cfg(feature = "transcription-service")]
    Transcription(TranscriptionEvent),
}
