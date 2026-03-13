// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use axum::http::StatusCode;
use bytes::Bytes;
use opentalk_orchestrator_shared::RoomServerEvent;
use opentalk_roomserver_types::{
    api::{RoomServerAccess, TokenRequestBody},
    client_parameters::ClientParameters,
    room_parameters::RoomParameters,
    room_parameters_patch::RoomParametersPatch,
};
use opentalk_roomserver_web_api::v1::{RoomAction, RoomBackend};
use opentalk_types_api_common::error::{ApiError, ErrorBody};
use opentalk_types_common::rooms::RoomId;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    service_instance::{
        InstanceData, ServiceInstance,
        selection::{InstanceError, SelectedInstance},
    },
};

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RoomserverInstance {
    pub rooms: HashSet<RoomId>,
    pub data: InstanceData,
}

#[async_trait::async_trait]
impl ServiceInstance for RoomserverInstance {
    type Event = RoomServerEvent;
    type ManagedResource = RoomId;

    fn new(resources: HashSet<Self::ManagedResource>) -> Self {
        Self {
            rooms: resources,
            data: InstanceData::default(),
        }
    }

    fn manages(&self, resource: &Self::ManagedResource) -> bool {
        self.rooms.contains(resource)
    }

    fn add_managed_resource(&mut self, managed_data: Self::ManagedResource) {
        self.rooms.insert(managed_data);
    }

    fn instance_data(&self) -> &InstanceData {
        &self.data
    }

    fn instance_data_mut(&mut self) -> &mut InstanceData {
        &mut self.data
    }

    async fn handle_event(&mut self, event: Self::Event) {
        match event {
            RoomServerEvent::RemoveRoom(remove_room_id) => {
                self.rooms.retain(|room_id| room_id != &remove_room_id);
            }
        }
    }
}

#[async_trait]
impl RoomBackend for AppState {
    async fn put_room(
        &self,
        room_id: RoomId,
        room_parameters: RoomParameters,
    ) -> Result<RoomAction, ApiError> {
        let SelectedInstance {
            address,
            auth_header,
        } = match self.select_roomserver(room_id).await {
            Ok(instance) => instance,
            Err(err) => {
                tracing::error!("Failed to select roomserver instance {err:?}");
                return Err(err.into());
            }
        };

        tracing::debug!("send put request to roomserver '{address}'");

        let response = self
            .client
            .put(format!("{address}v1/rooms/{room_id}"))
            .header(AUTHORIZATION, auth_header)
            .json(&room_parameters)
            .send()
            .await;

        let Ok(response) = response else {
            tracing::debug!("Request failed with: '{response:?}'");
            return Err(ApiError::internal().with_message("Roomserver unreachable"));
        };

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            ApiError::internal().with_message(format!(
                "Failed to read response body: {:?}",
                err.to_string()
            ))
        })?;

        tracing::trace!("received status code: {status:?}, response body: {body}");

        let Some(room_action) = RoomAction::from_status_code(status) else {
            let body = serde_json::from_str::<ErrorBody>(&body)
                .map_err(|err| ApiError::internal().with_message(err.to_string()))?;

            return Err(ApiError {
                status,
                www_authenticate: None,
                body,
            });
        };

        Ok(room_action)
    }

    async fn patch_room(
        &self,
        room_id: RoomId,
        patch: RoomParametersPatch,
    ) -> Result<RoomAction, ApiError> {
        let SelectedInstance {
            address,
            auth_header,
        } = match self.select_roomserver(room_id).await {
            Ok(instance) => instance,
            Err(err) => {
                match err {
                    InstanceError::NotAvailable => {
                        // This is a special case where we would not want to return the 503 from the
                        // instance selection. Instead we return a 404 to act as if we were the
                        // roomserver and to avoid confusing the client.
                        return Err(ApiError::not_found()
                            .with_message("The requested room could not be found"));
                    }
                    err => {
                        tracing::error!("Failed to select roomserver instance {err:?}");
                        return Err(err.into());
                    }
                }
            }
        };

        tracing::debug!("send patch request to roomserver '{address}'");

        let response = self
            .client
            .patch(format!("{address}v1/rooms/{room_id}"))
            .header(AUTHORIZATION, auth_header)
            .json(&patch)
            .send()
            .await;

        let Ok(response) = response else {
            tracing::debug!("Request failed with: '{response:?}'");
            return Err(ApiError::internal().with_message("Roomserver unreachable"));
        };

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            ApiError::internal().with_message(format!(
                "Failed to read response body: {:?}",
                err.to_string()
            ))
        })?;

        tracing::trace!("received status code: {status:?}, response body: {body}");

        let Some(room_action) = RoomAction::from_status_code(status) else {
            let body = serde_json::from_str::<ErrorBody>(&body)
                .map_err(|err| ApiError::internal().with_message(err.to_string()))?;

            return Err(ApiError {
                status,
                www_authenticate: None,
                body,
            });
        };

        Ok(room_action)
    }

    async fn request_room_token(
        &mut self,
        room_id: RoomId,
        client_parameters: ClientParameters,
        room_parameters: Option<RoomParameters>,
    ) -> Result<RoomServerAccess, ApiError> {
        let SelectedInstance {
            address,
            auth_header,
        } = match self.select_roomserver(room_id).await {
            Ok(instance) => instance,
            Err(err) => {
                tracing::error!("Failed to select roomserver instance {err:?}");
                return Err(err.into());
            }
        };
        tracing::debug!("send token request to roomserver '{address}'");

        let token_request = TokenRequestBody {
            client_parameters,
            room_parameters,
        };

        let response = self
            .client
            .post(format!("{address}v1/rooms/{room_id}/token"))
            .header(AUTHORIZATION, auth_header)
            .json(&token_request)
            .send()
            .await;

        let Ok(response) = response else {
            tracing::debug!("Request failed with: '{response:?}'");
            return Err(ApiError::internal().with_message("Roomserver unreachable"));
        };

        let status = response.status();
        let body = response.bytes().await.map_err(|e| {
            ApiError::internal().with_message(format!("Roomserver connection interrupted: {e}"))
        })?;

        tracing::trace!(
            "received status code: {status:?}, response body: {:?}",
            std::str::from_utf8(&body)
        );

        match status {
            StatusCode::OK => Ok(deserialize_token_response(status, &body)?),
            StatusCode::UNAUTHORIZED => {
                Err(ApiError::internal().with_message("Failed to authorize at roomserver"))
            }
            error_code => Err(ApiError {
                status: error_code,
                www_authenticate: None,
                body: deserialize_token_response(error_code, &body)?,
            }),
        }
    }
}

fn deserialize_token_response<T: for<'a> Deserialize<'a>>(
    status: StatusCode,
    body: &Bytes,
) -> Result<T, ApiError> {
    match serde_json::from_slice::<T>(body) {
        Ok(t) => Ok(t),
        Err(e) => {
            tracing::error!(
                "Unexpected roomserver token response body for status {status}: {e}\nBody:\n{}",
                String::from_utf8_lossy(body)
            );
            Err(ApiError::internal().with_message("Unexpected roomserver response body"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use opentalk_orchestrator_shared::Metrics;
    use opentalk_service_auth::{ApiKey, service::ApiKeys};
    use opentalk_types_common::rooms::RoomId;
    use url::Url;

    use crate::{AppState, service_instance::InstanceData};

    #[test_log::test(tokio::test)]
    async fn select_existing_room() {
        let server_1 = Url::parse("http://0.0.0.1").unwrap();
        let server_2 = Url::parse("http://0.0.0.2").unwrap();

        const ROOM_1: RoomId = RoomId::from_u128(1);
        const ROOM_2: RoomId = RoomId::from_u128(2);
        const ROOM_3: RoomId = RoomId::from_u128(3);

        let roomserver_api_key = ApiKey::new("roomserver", "secret123");

        let app_state = AppState::new(ApiKeys::new(vec![roomserver_api_key]));
        let mut roomservers = app_state.roomserver_services.write().await;

        let mut rooms = HashSet::default();
        rooms.insert(ROOM_1);

        roomservers.insert(
            server_1.clone(),
            super::RoomserverInstance {
                rooms,
                data: InstanceData {
                    metrics: Metrics {
                        load: 80,
                        accepting_jobs: true,
                    },

                    api_key_ids: vec!["roomserver".into()],
                },
            },
        );

        let mut rooms = HashSet::default();
        rooms.insert(ROOM_2);
        rooms.insert(ROOM_3);

        roomservers.insert(
            server_2.clone(),
            super::RoomserverInstance {
                rooms,
                data: InstanceData {
                    metrics: Metrics {
                        load: 20,
                        accepting_jobs: true,
                    },

                    api_key_ids: vec!["roomserver".into()],
                },
            },
        );

        drop(roomservers);

        let server = app_state.select_roomserver(ROOM_1).await.unwrap().address;
        assert_eq!(server, server_1.clone());

        let server = app_state.select_roomserver(ROOM_2).await.unwrap().address;
        assert_eq!(server, server_2.clone());

        let server = app_state.select_roomserver(ROOM_3).await.unwrap().address;
        assert_eq!(server, server_2.clone());
    }
}
