// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use opentalk_service_auth::ApiKeyId;
use serde::{Deserialize, Serialize};

use crate::error::RegistrationError;
#[cfg(feature = "recording-service")]
pub use crate::recorder::{RecorderEvent, RecorderResource, RegisterRecorder};
#[cfg(feature = "roomserver-service")]
pub use crate::roomserver::{RegisterRoomServer, RoomServerEvent};
#[cfg(feature = "transcription-service")]
pub use crate::transcription::{RegisterTranscription, TranscriptionEvent};

pub mod error;

#[cfg(feature = "recording-service")]
mod recorder;
#[cfg(feature = "roomserver-service")]
mod roomserver;
#[cfg(feature = "transcription-service")]
mod transcription;

/// The server response to the [`Register`] request
#[derive(Debug, Serialize, Deserialize)]
pub enum RegisterResponse {
    Success,
    Error(RegistrationError),
}

impl From<RegistrationError> for RegisterResponse {
    fn from(error: RegistrationError) -> Self {
        Self::Error(error)
    }
}

/// Request to register at the orchestrator
#[derive(Debug, Serialize, Deserialize)]
pub struct Register {
    pub register_data: RegisterData,
    /// The type of service
    pub register_type: RegisterType,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RegisterData {
    /// The address of the service
    pub address: String,
    /// A list of api key ids to that authorize requests to the service
    pub api_key_ids: Vec<ApiKeyId>,
    /// The initial metrics
    pub metrics: Metrics,
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
