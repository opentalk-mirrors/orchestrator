// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use serde::{Deserialize, Serialize};
pub use url::Url;

#[cfg(feature = "recording-service")]
pub use crate::services::recorder::{RecorderEvent, RecorderResource, RegisterRecorder};
#[cfg(feature = "roomserver-service")]
pub use crate::services::roomserver::{RegisterRoomserver, RoomserverEvent};
#[cfg(feature = "transcription-service")]
pub use crate::services::transcription::{RegisterTranscription, TranscriptionEvent};
pub use crate::{
    event::Event,
    register::{Register, RegisterData, RegisterResponse, RegisterType, ServiceAddress},
};

pub mod error;
mod event;
mod register;
mod services;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metrics {
    pub load: u8,
    pub accepting_jobs: bool,
}
