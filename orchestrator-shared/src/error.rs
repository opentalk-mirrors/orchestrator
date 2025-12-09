// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use serde::{Deserialize, Serialize};

/// Errors that can occur during the service registration process
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationError {
    #[error("the requested address is already in use")]
    AddressAlreadyInUse,
    #[error("unknown api key ids")]
    UnknownApiKeyIds,
    #[error("failed to receive registration message within timeout")]
    Timeout,
    #[error("expected text/binary format")]
    InvalidMessageType,
    #[error("failed to parse registration message")]
    InvalidJson,
}

/// Event related errors that can occur when sending events to the orchestrator
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum EventError {
    #[error("the provided event does not match the service type")]
    EventTypeMismatch,
    #[error("failed to parse the event message")]
    InvalidJson,
}
