// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use axum::http::StatusCode;
use bytes::Bytes;
use opentalk_orchestrator_shared::{Metrics, RoomServerEvent};
use opentalk_roomserver_types::api::{RoomServerAccess, TokenRequestBody, TokenResponse};
use opentalk_roomserver_types::{
    client_parameters::ClientParameters, room_parameters::RoomParameters,
};
use opentalk_roomserver_web_api::v1::{RoomAction, RoomBackend};
use opentalk_types_api_v1::error::{ApiError, ErrorBody};
use opentalk_types_common::rooms::RoomId;
use rand::seq::IteratorRandom;
use serde::{Deserialize, Serialize};

use crate::{Address, AppState};

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RoomServerInstance {
    pub(crate) metrics: Metrics,
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
        let Some(address) = self.select_roomserver(room_id).await else {
            return Err(ApiError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                www_authenticate: None,
                body: ErrorBody::new(
                    "no_roomserver_available",
                    "No roomserver are available on the orchestrator",
                ),
            });
        };

        log::debug!("send put request to roomserver '{address}'");

        let response = self
            .client
            .put(format!("http://{address}/rooms/{room_id}"))
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
    ) -> Result<Option<RoomServerAccess>, ApiError> {
        let Some(address) = self.select_roomserver(room_id).await else {
            return Err(ApiError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                www_authenticate: None,
                body: ErrorBody::new(
                    "no_roomserver_available",
                    "No roomserver are available on the orchestrator",
                ),
            });
        };

        log::debug!("send token request to roomserver '{address}'");

        let token_request = TokenRequestBody {
            client_parameters,
            room_parameters,
        };

        let response = self
            .client
            .post(format!("http://{address}/rooms/{room_id}/token"))
            .json(&token_request)
            .send()
            .await
            .map_err(|_| ApiError::internal().with_message("Roomserver unreachable"))?;

        let status = response.status();

        let body = response.bytes().await.map_err(|e| {
            ApiError::internal().with_message(format!("Roomserver connection interrupted: {e}"))
        })?;

        match status {
            StatusCode::OK => match deserialize_token_response(status, &body)? {
                TokenResponse::Token(roomserver_access) => Ok(Some(roomserver_access)),
                TokenResponse::UnknownRoom => Ok(None),
            },
            error_code => Err(ApiError {
                status: error_code,
                www_authenticate: None,
                body: deserialize_token_response(error_code, &body)?,
            }),
        }
    }
}

impl AppState {
    /// Select a roomserver instance for the given room id
    /// 
    /// 
    async fn select_roomserver(&self, room_id: RoomId) -> Option<Address> {
        let mut roomservers = self.roomserver_services.lock().await;

        // No roomservers are configured
        if roomservers.is_empty() {
            return None;
        }

        if let Some(existing_instance) = roomservers
            .iter()
            .find(|(_, instance)| instance.rooms.contains(&room_id))
            .map(|(address, _)| address.clone())
        {
            return Some(existing_instance);
        }

        let (address, instance) = roomservers
            .iter_mut()
            .filter(|(_, instance)| instance.metrics.accepting_jobs)
            .choose(&mut rand::rng())?;

        instance.rooms.insert(room_id);

        Some(address.clone())
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
