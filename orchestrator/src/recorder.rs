// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use opentalk_orchestrator_shared::{RecorderEvent, RecorderResource};
use serde::Serialize;

use crate::instance::{InstanceData, ServiceInstance};

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RecorderInstance {
    rooms: HashSet<RecorderResource>,
    data: InstanceData,
}

#[async_trait::async_trait]
impl ServiceInstance for RecorderInstance {
    type Event = RecorderEvent;
    type ManagedResource = RecorderResource;

    fn new(resources: HashSet<Self::ManagedResource>) -> Self {
        Self {
            rooms: resources,
            data: InstanceData::default(),
        }
    }

    fn manages(&self, resource: &Self::ManagedResource) -> bool {
        self.rooms.contains(resource)
    }

    fn add_managed_resource(&mut self, resource: Self::ManagedResource) {
        self.rooms.insert(resource);
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
