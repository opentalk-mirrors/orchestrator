// SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
//
// SPDX-License-Identifier: EUPL-1.2

#[cfg(feature = "recording-service")]
pub mod recorder;
#[cfg(feature = "roomserver-service")]
pub mod roomserver;
#[cfg(feature = "transcription-service")]
pub mod transcription;
