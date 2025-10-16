// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use opentalk_orchestrator_shared::{Metrics, TranscriptionEvent};
use opentalk_types_common::rooms::RoomId;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct TranscriptionInstance {
    pub(crate) metrics: Metrics,
    pub(crate) rooms: HashSet<RoomId>,
}

impl TranscriptionInstance {
    pub(crate) async fn handle_event(&mut self, _event: &TranscriptionEvent) {
        // TODO
    }
}
