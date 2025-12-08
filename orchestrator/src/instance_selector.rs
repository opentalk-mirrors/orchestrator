// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use opentalk_orchestrator_shared::Metrics;
use opentalk_service_auth::{ApiKeyId, EncodingError};
use opentalk_types_api_common::error::ApiError;
use opentalk_types_common::rooms::RoomId;
use rand::prelude::IteratorRandom;
use serde::Serialize;

use crate::{Address, AppState, instance_runner::InstanceCollection};

/// The generic data that is held by each instance
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct InstanceData {
    /// The current metrics of the instance
    pub(crate) metrics: Metrics,
    /// Possible key ids for requests towards the service
    pub(crate) api_key_ids: Vec<ApiKeyId>,
    /// Rooms that are assigned to the instance
    pub(crate) rooms: HashSet<RoomId>,
}

/// A trait to expose the [`InstanceData`] on an instance
pub(crate) trait InstanceDataProvider {
    fn instance_data(&self) -> &InstanceData;

    fn instance_data_mut(&mut self) -> &mut InstanceData;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectedInstance {
    /// The address of the selected instance
    pub(crate) address: Address,
    /// The authorization header for request towards the instance
    pub(crate) auth_header: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InstanceError {
    #[error("no instances of the requested service are currently available")]
    NotAvailable,

    #[error("no matching service API keys found for key ids: {0:#?}")]
    UnknownApiKeyIds(Vec<ApiKeyId>),

    #[error("failed to create authorization header for key id {key_id}: {err:?}")]
    AuthHeader {
        key_id: ApiKeyId,
        err: EncodingError,
    },
}

impl From<InstanceError> for ApiError {
    fn from(instance_error: InstanceError) -> Self {
        match instance_error {
            InstanceError::NotAvailable => ApiError::service_unavailable()
                .with_message("not_available")
                .with_message(instance_error.to_string()),
            InstanceError::UnknownApiKeyIds { .. } | InstanceError::AuthHeader { .. } => {
                ApiError::internal().with_message("internal authorization error")
            }
        }
    }
}

impl AppState {
    pub(crate) async fn select_roomserver(
        &self,
        room_id: RoomId,
    ) -> Result<SelectedInstance, InstanceError> {
        self.select_instance(room_id, &self.roomserver_services)
            .await
    }

    async fn select_instance<T: InstanceDataProvider>(
        &self,
        room_id: RoomId,
        instances: &InstanceCollection<T>,
    ) -> Result<SelectedInstance, InstanceError> {
        let mut instances = instances.lock().await;

        // No instances for are registered
        if instances.is_empty() {
            return Err(InstanceError::NotAvailable);
        }

        if let Some((address, instance)) = instances
            .iter()
            .find(|(_, instance)| instance.instance_data().rooms.contains(&room_id))
        {
            let auth_header = self.get_auth_header_for_instance(instance)?;

            return Ok(SelectedInstance {
                address: address.clone(),
                auth_header,
            });
        }

        let Some((address, instance)) = instances
            .iter_mut()
            .filter(|(_, instance)| instance.instance_data().metrics.accepting_jobs)
            .choose(&mut rand::rng())
        else {
            return Err(InstanceError::NotAvailable);
        };

        let auth_header = self.get_auth_header_for_instance(instance)?;

        instance.instance_data_mut().rooms.insert(room_id);

        Ok(SelectedInstance {
            address: address.clone(),
            auth_header,
        })
    }

    fn get_auth_header_for_instance<T: InstanceDataProvider>(
        &self,
        instance: &T,
    ) -> Result<String, InstanceError> {
        let key_ids = &instance.instance_data().api_key_ids;

        let Some(api_key) = self.get_api_key_for_key_ids(key_ids) else {
            return Err(InstanceError::UnknownApiKeyIds(key_ids.clone()));
        };

        let jwt = match api_key.generate_jwt() {
            Ok(jwt) => format!("Bearer {jwt}"),
            Err(err) => {
                return Err(InstanceError::AuthHeader {
                    key_id: api_key.id,
                    err,
                });
            }
        };

        Ok(jwt)
    }
}
