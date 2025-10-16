// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use serde::{Deserialize, Serialize};
#[cfg(any(
    feature = "recording-service",
    feature = "roomserver-service",
    feature = "transcription-service"
))]
use {opentalk_types::common::rooms::RoomId, std::collections::HashSet};

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

#[cfg(feature = "recording-service")]
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRecorder {
    pub rooms: HashSet<RoomId>,
}

#[cfg(feature = "roomserver-service")]
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRoomServer {
    pub rooms: HashSet<RoomId>,
}

#[cfg(feature = "transcription-service")]
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterTranscription {
    pub rooms: HashSet<RoomId>,
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

#[cfg(feature = "recording-service")]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RecorderEvent {
    RemoveRoom(RoomId),
}

#[cfg(feature = "recording-service")]
impl From<RecorderEvent> for Event {
    fn from(event: RecorderEvent) -> Event {
        Event::Recorder(event)
    }
}

#[cfg(feature = "roomserver-service")]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RoomServerEvent {
    RemoveRoom(RoomId),
}

#[cfg(feature = "roomserver-service")]
impl From<RoomServerEvent> for Event {
    fn from(event: RoomServerEvent) -> Event {
        Event::RoomServer(event)
    }
}

#[cfg(feature = "transcription-service")]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TranscriptionEvent {
    RemoveRoom(RoomId),
}

#[cfg(feature = "transcription-service")]
impl From<TranscriptionEvent> for Event {
    fn from(event: TranscriptionEvent) -> Event {
        Event::Transcription(event)
    }
}
