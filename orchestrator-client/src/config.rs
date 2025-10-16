// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

use http::{
    Uri,
    uri::{InvalidUri, Scheme},
};
use serde::{Deserialize, Deserializer, de};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub endpoint: Endpoint,
}

impl OrchestratorConfig {
    #[must_use]
    pub(crate) fn websocket_url(&self) -> String {
        let scheme = if self.endpoint.0.scheme() == Some(&Scheme::HTTP) {
            "ws"
        } else {
            "wss"
        };

        format!("{scheme}://{}/register", self.endpoint.0)
    }
}

#[derive(Debug, Clone)]
pub struct Endpoint(pub Uri);

#[derive(Debug, Error)]
pub enum EndpointError {
    #[error(transparent)]
    Uri(InvalidUri),
    #[error("the endpoint scheme schould be http or https")]
    InvalidScheme,
}

impl<'de> Deserialize<'de> for Endpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let uri = String::deserialize(deserializer)?;

        Endpoint::try_from(uri).map_err(de::Error::custom)
    }
}

impl Endpoint {
    pub fn try_from<U>(uri: U) -> Result<Self, EndpointError>
    where
        U: TryInto<Uri, Error = InvalidUri>,
    {
        let uri: Uri = uri.try_into().map_err(EndpointError::Uri)?;
        if let Some(scheme) = uri.scheme()
            && scheme != &Scheme::HTTP
            && scheme != &Scheme::HTTPS
        {
            return Err(EndpointError::InvalidScheme);
        }

        Ok(Self(uri))
    }
}
