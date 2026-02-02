// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use opentalk_types::common::rooms::RoomId;
use serde::{Deserialize, Serialize};

use crate::Event;

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRoomServer {
    pub rooms: HashSet<RoomId>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum RoomServerEvent {
    RemoveRoom(RoomId),
}

impl From<RoomServerEvent> for Event {
    fn from(event: RoomServerEvent) -> Event {
        Event::RoomServer(event)
    }
}

impl TryFrom<Event> for RoomServerEvent {
    type Error = Event;

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::RoomServer(roomserver_event) => Ok(roomserver_event),
            event => Err(event),
        }
    }
}
