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
#[serde(tag = "type")]
pub enum RoomServerEvent {
    RemoveRoom(RoomId),
}

impl From<RoomServerEvent> for Event {
    fn from(event: RoomServerEvent) -> Event {
        Event::RoomServer(event)
    }
}
