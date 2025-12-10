// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use opentalk_orchestrator_shared::{Event, Metrics};
use opentalk_service_auth::ApiKeyId;
use serde::Serialize;

pub(crate) mod registration;
pub(crate) mod runner;
pub(crate) mod selection;

/// The common behavior for orchestrated service instances
#[async_trait::async_trait]
pub(crate) trait ServiceInstance: 'static + Default {
    /// The signaling event that is sent by the service client
    type Event: TryFrom<Event>;
    /// The resource that needs to be tracked for instance selection
    type ManagedResource;

    /// Create a new service instance, initialized with the given resources
    fn new(resources: HashSet<Self::ManagedResource>) -> Self;

    /// Check if this service instance manages the given resources
    fn manages(&self, resource: &Self::ManagedResource) -> bool;

    /// Add a new resource to this service instance
    fn add_managed_resource(&mut self, resource: Self::ManagedResource);

    /// Handle a service specific event from the connected client
    async fn handle_event(&mut self, event: Self::Event);

    /// Get the [`InstanceData`] for this service instance
    fn instance_data(&self) -> &InstanceData;

    /// Get mutable [`InstanceData`] for this service instance
    fn instance_data_mut(&mut self) -> &mut InstanceData;
}

/// The general data that is held by each instance
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct InstanceData {
    /// The current metrics of the instance
    pub(crate) metrics: Metrics,
    /// Possible key ids for requests towards the service
    pub(crate) api_key_ids: Vec<ApiKeyId>,
}
