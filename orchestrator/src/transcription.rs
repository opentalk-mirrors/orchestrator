// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::HashSet;

use async_trait::async_trait;
use opentalk_orchestrator_shared::{TranscriptionEvent, TranscriptionResource};
use opentalk_transcription_web_api::v1::{TranscriptionBackend, TranscriptionTarget};
use opentalk_types_api_common::error::{ApiError, ErrorBody};
use reqwest::header::AUTHORIZATION;
use serde::Serialize;

use crate::{
    AppState,
    service_instance::{InstanceData, ServiceInstance, selection::SelectedInstance},
};

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct TranscriptionInstance {
    pub transcriptions: HashSet<TranscriptionResource>,
    pub data: InstanceData,
}

#[async_trait::async_trait]
impl ServiceInstance for TranscriptionInstance {
    type Event = TranscriptionEvent;
    type ManagedResource = TranscriptionResource;

    fn new(resources: HashSet<Self::ManagedResource>) -> Self {
        Self {
            transcriptions: resources,
            data: InstanceData::default(),
        }
    }

    fn manages(&self, resource: &Self::ManagedResource) -> bool {
        self.transcriptions.contains(resource)
    }

    fn add_managed_resource(&mut self, resource: Self::ManagedResource) {
        self.transcriptions.insert(resource);
    }

    async fn handle_event(&mut self, event: Self::Event) {
        match event {
            TranscriptionEvent::RemoveTranscription(resource) => {
                self.transcriptions.remove(&resource);
            }
        }
    }

    fn instance_data(&self) -> &InstanceData {
        &self.data
    }

    fn instance_data_mut(&mut self) -> &mut InstanceData {
        &mut self.data
    }
}

#[async_trait]
impl TranscriptionBackend for AppState {
    async fn init(&self, room_info: TranscriptionTarget) -> Result<(), ApiError> {
        let transcription_resource = TranscriptionResource {
            room_id: room_info.room_id,
            breakout_id: room_info.breakout_room,
        };

        let SelectedInstance {
            address,
            auth_header,
        } = self
            .select_transcription(transcription_resource.clone())
            .await?;

        let url = address.join("v1/init").map_err(|_| {
            ApiError::internal().with_message("Failed to construct transcription URL")
        })?;

        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, auth_header)
            .json(&room_info)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("Failed to send request to transcription service: {e}");
                ApiError::internal().with_message("Transcription service unreachable")
            })?;

        let status = response.status();
        let body = response.bytes().await.map_err(|e| {
            tracing::error!("Failed to read transcription service response body: {e}");
            ApiError::internal().with_message("Failed to read transcription service response body")
        })?;

        if status.is_success() {
            return Ok(());
        } else {
            let error_body = match serde_json::from_slice::<ErrorBody>(&body) {
                Ok(t) => Ok(t),
                Err(e) => {
                    tracing::error!(
                        "Unexpected transcription service response body for status {status}: {e}\nBody:\n{}",
                        String::from_utf8_lossy(&body)
                    );
                    Err(ApiError::internal()
                        .with_message("Unexpected transcription service response body"))
                }
            }?;

            Err(ApiError {
                status,
                www_authenticate: None,
                body: error_body,
            })
        }
    }
}
