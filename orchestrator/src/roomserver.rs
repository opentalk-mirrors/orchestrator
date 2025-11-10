// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use axum::http::StatusCode;
use bytes::Bytes;
use opentalk_orchestrator_shared::{Metrics, RoomServerEvent};
use opentalk_roomserver_types::{
    api::{RoomServerAccess, TokenRequestBody},
    client_parameters::ClientParameters,
    room_parameters::RoomParameters,
};
use opentalk_roomserver_web_api::v1::{RoomAction, RoomBackend};
use opentalk_service_auth::{ApiKeyId, EncodingError};
use opentalk_types_api_common::error::{ApiError, ErrorBody};
use opentalk_types_common::rooms::RoomId;
use rand::seq::IteratorRandom;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

use crate::{Address, AppState};

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RoomServerInstance {
    pub(crate) metrics: Metrics,
    /// Possible key ids for requests towards the service
    pub(crate) api_key_ids: Vec<ApiKeyId>,
    pub(crate) rooms: HashSet<RoomId>,
}

impl RoomServerInstance {
    pub(crate) async fn handle_event(&mut self, event: &RoomServerEvent) {
        match event {
            RoomServerEvent::RemoveRoom(remove_room_id) => {
                self.rooms.retain(|room_id| room_id != remove_room_id);
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
            auth_header: token,
        } = match self.select_roomserver(room_id).await {
            Ok(instance) => instance,
            Err(err) => {
                log::error!("Failed to select roomserver instance {err:?}");
                return Err(err.into());
            }
        };

        log::debug!("send put request to roomserver '{address}'");

        let auth_header = format!("Bearer {}", token);

        let response = self
            .client
            .put(format!("{address}v1/rooms/{room_id}"))
            .header(AUTHORIZATION, auth_header)
            .json(&room_parameters)
            .send()
            .await
            .map_err(|_| ApiError::internal().with_message("Roomserver unreachable"))?;

        let status = response.status();

        log::debug!("received status code: {response:?}");

        let Some(room_action) = RoomAction::from_status_code(status) else {
            let body = response
                .json::<ErrorBody>()
                .await
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
            auth_header: token,
        } = match self.select_roomserver(room_id).await {
            Ok(instance) => instance,
            Err(err) => {
                log::error!("Failed to select roomserver instance {err:?}");
                return Err(err.into());
            }
        };
        log::debug!("send token request to roomserver '{address}'");

        let token_request = TokenRequestBody {
            client_parameters,
            room_parameters,
        };

        let auth_header = format!("Bearer {}", token);

        let response = self
            .client
            .post(format!("{address}v1/rooms/{room_id}/token"))
            .header(AUTHORIZATION, auth_header)
            .json(&token_request)
            .send()
            .await
            .map_err(|_| ApiError::internal().with_message("Roomserver unreachable"))?;

        let status = response.status();

        let body = response.bytes().await.map_err(|e| {
            ApiError::internal().with_message(format!("Roomserver connection interrupted: {e}"))
        })?;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedInstance {
    /// The address of the selected instance
    address: Address,
    /// The authorization header for request towards the instance
    auth_header: String,
}

#[derive(Debug, thiserror::Error)]
enum InstanceError {
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
    /// Select a roomserver instance for the given room id
    async fn select_roomserver(&self, room_id: RoomId) -> Result<SelectedInstance, InstanceError> {
        let mut roomservers = self.roomserver_services.lock().await;

        // No roomservers are registered
        if roomservers.is_empty() {
            return Err(InstanceError::NotAvailable);
        }

        if let Some((address, instance)) = roomservers
            .iter()
            .find(|(_, instance)| instance.rooms.contains(&room_id))
        {
            let auth_header = self.get_auth_header_for_instance(instance)?;

            return Ok(SelectedInstance {
                address: address.clone(),
                auth_header,
            });
        }

        let Some((address, instance)) = roomservers
            .iter_mut()
            .filter(|(_, instance)| instance.metrics.accepting_jobs)
            .choose(&mut rand::rng())
        else {
            return Err(InstanceError::NotAvailable);
        };

        let auth_header = self.get_auth_header_for_instance(instance)?;

        instance.rooms.insert(room_id);

        Ok(SelectedInstance {
            address: address.clone(),
            auth_header,
        })
    }

    fn get_auth_header_for_instance(
        &self,
        instance: &RoomServerInstance,
    ) -> Result<String, InstanceError> {
        let key_ids = &instance.api_key_ids;

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

fn deserialize_token_response<T: for<'a> Deserialize<'a>>(
    status: StatusCode,
    body: &Bytes,
) -> Result<T, ApiError> {
    match serde_json::from_slice::<T>(body) {
        Ok(t) => Ok(t),
        Err(e) => {
            log::error!(
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

    use crate::AppState;

    #[test_log::test(tokio::test)]
    async fn select_existing_room() {
        let server_1 = "0.0.0.1".to_string();
        let server_2 = "0.0.0.2".to_string();

        const ROOM_1: RoomId = RoomId::from_u128(1);
        const ROOM_2: RoomId = RoomId::from_u128(2);
        const ROOM_3: RoomId = RoomId::from_u128(3);

        let roomserver_api_key = ApiKey::new("roomserver", "secret123");

        let app_state = AppState::new(ApiKeys::new(vec![roomserver_api_key]));
        let mut roomservers = app_state.roomserver_services.lock().await;

        let mut rooms = HashSet::default();
        rooms.insert(ROOM_1);

        roomservers.insert(
            server_1.clone(),
            super::RoomServerInstance {
                metrics: Metrics {
                    load: 80,
                    accepting_jobs: true,
                },

                api_key_ids: vec!["roomserver".into()],
                rooms,
            },
        );

        let mut rooms = HashSet::default();
        rooms.insert(ROOM_2);
        rooms.insert(ROOM_3);

        roomservers.insert(
            server_2.clone(),
            super::RoomServerInstance {
                metrics: Metrics {
                    load: 20,
                    accepting_jobs: true,
                },
                api_key_ids: vec!["roomserver".into()],
                rooms,
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
