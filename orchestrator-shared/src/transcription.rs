// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use opentalk_types::common::rooms::RoomId;
use serde::{Deserialize, Serialize};

use crate::Event;

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterTranscription {
    pub rooms: HashSet<RoomId>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TranscriptionEvent {
    RemoveRoom(RoomId),
}

impl From<TranscriptionEvent> for Event {
    fn from(event: TranscriptionEvent) -> Event {
        Event::Transcription(event)
    }
}

impl TryFrom<Event> for TranscriptionEvent {
    type Error = Event;

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::Transcription(transcription_event) => Ok(transcription_event),
            event => Err(event),
        }
    }
}
