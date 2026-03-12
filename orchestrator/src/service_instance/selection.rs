// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use opentalk_service_auth::{ApiKeyId, EncodingError};
use opentalk_types_api_common::error::ApiError;
use opentalk_types_common::rooms::RoomId;
use url::Url;

use crate::{
    AppState,
    service_instance::{ServiceInstance, runner::InstanceCollection},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectedInstance {
    /// The address of the selected instance
    pub(crate) address: Url,
    /// The authorization header for request towards the instance
    pub(crate) auth_header: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InstanceError {
    #[error("no instance of the requested service are currently available")]
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

    async fn select_instance<T: ServiceInstance>(
        &self,
        managed_data: T::ManagedResource,
        instances: &InstanceCollection<T>,
    ) -> Result<SelectedInstance, InstanceError> {
        let mut instances = instances.write().await;

        // No instances are registered for the requested service type
        if instances.is_empty() {
            return Err(InstanceError::NotAvailable);
        }

        if let Some((address, instance)) = instances
            .iter()
            .find(|(_, instance)| instance.manages(&managed_data))
        {
            let auth_header = self.get_auth_header_for_instance(instance)?;

            return Ok(SelectedInstance {
                address: address.clone(),
                auth_header,
            });
        }

        let lowest_load_instance = instances
            .iter_mut()
            .filter(|(_, instance)| instance.instance_data().metrics.accepting_jobs)
            .min_by_key(|(_, instance)| instance.instance_data().metrics.load);

        let Some((address, instance)) = lowest_load_instance else {
            return Err(InstanceError::NotAvailable);
        };

        let auth_header = self.get_auth_header_for_instance(instance)?;

        instance.add_managed_resource(managed_data);

        Ok(SelectedInstance {
            address: address.clone(),
            auth_header,
        })
    }

    fn get_auth_header_for_instance<T: ServiceInstance>(
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use opentalk_orchestrator_shared::Metrics;
    use opentalk_service_auth::service::ApiKeys;

    use super::*;
    use crate::{roomserver::RoomserverInstance, service_instance::InstanceData};

    fn roomserver_instance_with_load(load: u8) -> RoomserverInstance {
        RoomserverInstance {
            rooms: HashSet::default(),
            data: InstanceData {
                metrics: Metrics {
                    load,
                    accepting_jobs: true,
                },
                api_key_ids: vec!["roomserver".into()],
            },
        }
    }
    #[tokio::test]
    async fn select_existing_instance() {
        let app_state = AppState::new(ApiKeys::new(vec!["roomserver:secret".parse().unwrap()]));

        let mut instances = app_state.roomserver_services.write().await;

        instances.insert(
            "http://localhost:11333".parse().unwrap(),
            RoomserverInstance {
                rooms: [RoomId::nil()].into_iter().collect(),
                data: InstanceData {
                    metrics: Metrics {
                        load: 50,
                        accepting_jobs: true,
                    },
                    api_key_ids: vec!["roomserver".into()],
                },
            },
        );

        drop(instances);

        let selected_instance = app_state
            .select_roomserver(RoomId::from_u128(0))
            .await
            .unwrap();

        assert_eq!(
            selected_instance.address,
            "http://localhost:11333".parse().unwrap()
        );
    }

    #[tokio::test]
    async fn select_lowest_load_instance() {
        let app_state = AppState::new(ApiKeys::new(vec!["roomserver:secret".parse().unwrap()]));
        let mut instances = app_state.roomserver_services.write().await;

        instances.insert(
            "http://localhost:11333".parse().unwrap(),
            roomserver_instance_with_load(70),
        );

        instances.insert(
            "http://localhost:11334".parse().unwrap(),
            roomserver_instance_with_load(30),
        );

        instances.insert(
            "http://localhost:11335".parse().unwrap(),
            roomserver_instance_with_load(50),
        );

        drop(instances);

        let selected_instance = app_state.select_roomserver(RoomId::nil()).await.unwrap();

        assert_eq!(
            selected_instance.address,
            "http://localhost:11334".parse().unwrap()
        );
    }

    #[tokio::test]
    async fn select_instance_not_accepting_jobs() {
        let app_state = AppState::new(ApiKeys::new(vec!["roomserver:secret".parse().unwrap()]));
        let mut instances = app_state.roomserver_services.write().await;

        instances.insert(
            "http://localhost:11333".parse().unwrap(),
            RoomserverInstance {
                rooms: HashSet::default(),
                data: InstanceData {
                    metrics: Metrics {
                        load: 10,
                        accepting_jobs: false,
                    },
                    api_key_ids: vec!["roomserver".into()],
                },
            },
        );

        instances.insert(
            "http://localhost:11334".parse().unwrap(),
            roomserver_instance_with_load(30),
        );

        instances.insert(
            "http://localhost:11335".parse().unwrap(),
            roomserver_instance_with_load(50),
        );

        drop(instances);

        let selected_instance = app_state.select_roomserver(RoomId::nil()).await.unwrap();

        assert_eq!(
            selected_instance.address,
            "http://localhost:11334".parse().unwrap()
        );
    }

    #[tokio::test]
    async fn no_instance_available() {
        let app_state = AppState::new(ApiKeys::new(vec!["roomserver:secret".parse().unwrap()]));

        let selected_instance = app_state.select_roomserver(RoomId::nil()).await;

        assert!(matches!(
            selected_instance,
            Err(InstanceError::NotAvailable)
        ));
    }

    #[tokio::test]
    async fn no_accepting_jobs_instances() {
        let app_state = AppState::new(ApiKeys::new(vec!["roomserver:secret".parse().unwrap()]));
        let mut instances = app_state.roomserver_services.write().await;

        instances.insert(
            "http://localhost:11333".parse().unwrap(),
            RoomserverInstance {
                rooms: HashSet::default(),
                data: InstanceData {
                    metrics: Metrics {
                        load: 80,
                        accepting_jobs: false,
                    },
                    api_key_ids: vec!["roomserver".into()],
                },
            },
        );

        drop(instances);

        let selected_instance = app_state.select_roomserver(RoomId::nil()).await;

        assert!(matches!(
            selected_instance,
            Err(InstanceError::NotAvailable)
        ));
    }
}
