// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use opentalk_types::common::rooms::RoomId;
use serde::{Deserialize, Serialize};

use crate::Event;

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRecorder {
    pub rooms: HashSet<RecorderResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Hash, Eq)]
pub struct RecorderResource {
    pub room_id: RoomId,
    pub breakout_id: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RecorderEvent {
    RemoveRoom(RoomId),
}

impl From<RecorderEvent> for Event {
    fn from(event: RecorderEvent) -> Event {
        Event::Recorder(event)
    }
}

impl TryFrom<Event> for RecorderEvent {
    type Error = Event;

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::Recorder(recorder_event) => Ok(recorder_event),
            event => Err(event),
        }
    }
}
