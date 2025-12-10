// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use opentalk_orchestrator_shared::TranscriptionEvent;
use opentalk_types_common::rooms::RoomId;
use serde::Serialize;

use crate::service_instance::{InstanceData, ServiceInstance};

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct TranscriptionInstance {
    pub rooms: HashSet<RoomId>,
    pub data: InstanceData,
}

#[async_trait::async_trait]
impl ServiceInstance for TranscriptionInstance {
    type Event = TranscriptionEvent;
    type ManagedResource = RoomId;

    fn new(resources: HashSet<Self::ManagedResource>) -> Self {
        Self {
            rooms: resources,
            data: InstanceData::default(),
        }
    }

    fn manages(&self, _resource: &Self::ManagedResource) -> bool {
        todo!()
    }

    fn add_managed_resource(&mut self, _resource: Self::ManagedResource) {
        todo!()
    }

    async fn handle_event(&mut self, _event: Self::Event) {
        todo!()
    }

    fn instance_data(&self) -> &InstanceData {
        &self.data
    }

    fn instance_data_mut(&mut self) -> &mut InstanceData {
        &mut self.data
    }
}
