/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

use base64::{Engine as _, engine::general_purpose::STANDARD};
use maka_event_log::EventLog;
use maka_model::prompt::{ContentPart, FileData, Message, ToolOutput};
use maka_runtime::attachment::{AttachmentRef, StorageRef, sniff_binary_mime};
use maka_runtime::event::StoredEvent;
use maka_runtime::tool_output::{AudioOutput, ImageOutput};
use tokio_util::sync::CancellationToken;

use crate::RunError;

pub(super) const NO_VISION: &str =
    "Image was read, but the selected model does not support image input.";
// Bound each artifact read, not the accumulated media in a model request.
// Historical images must not consume the allowance for a newly captured image.
const MEDIA_READ_LIMIT: usize = 12 * 1024 * 1024;

/// Targets borrow canonical evidence and are emitted alongside their messages.
/// Arbitrary JSON tool results cannot authorize a storage read.
pub(super) enum Target<'a> {
    Audio {
        message: usize,
        part: usize,
        audio: &'a AudioOutput,
    },
    User {
        message: usize,
        image: &'a AttachmentRef,
    },
    Tool {
        message: usize,
        part: usize,
        image: &'a ImageOutput,
    },
}

enum ImageRead {
    Bytes(Vec<u8>),
    Unavailable(&'static str),
}

pub(crate) async fn materialize(
    log: &EventLog,
    events: &[maka_event_log::context::ContextEvent],
    anchor: Option<&StoredEvent>,
    session: &str,
    vision: bool,
    cancellation: &CancellationToken,
) -> Result<Vec<Message>, RunError> {
    materialize_replay(
        log,
        events,
        anchor,
        session,
        vision,
        cancellation,
        None,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)] // Model projection needs both the replay cut and admission policy.
pub(crate) async fn materialize_replay(
    log: &EventLog,
    events: &[maka_event_log::context::ContextEvent],
    anchor: Option<&StoredEvent>,
    session: &str,
    vision: bool,
    cancellation: &CancellationToken,
    replay: Option<super::Replay<'_>>,
    prior_unknown: bool,
) -> Result<Vec<Message>, RunError> {
    let mut targets = Vec::new();
    let selected = anchor
        .into_iter()
        .map(super::EventRef::Canonical)
        .chain(events.iter().map(super::EventRef::from));
    let resources =
        super::resources::Resources::load(log, session, selected.clone(), cancellation).await?;
    let mut messages = super::build(
        selected,
        &resources,
        &mut targets,
        vision,
        replay,
        prior_unknown,
    )?;
    for target in targets {
        if cancellation.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let (reference, mime) = match &target {
            Target::Audio { audio, .. } => (&audio.reference, &audio.mime_type),
            Target::User { image, .. } => (&image.storage_ref, &image.mime_type),
            Target::Tool { image, .. } => (&image.reference, &image.mime_type),
        };
        let audio = matches!(target, Target::Audio { .. });
        let read = match resources.resolve(reference) {
            Some(reference) => read(log, session, reference, audio).await?,
            None => ImageRead::Unavailable("session_mismatch"),
        };
        if cancellation.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let part = match read {
            ImageRead::Bytes(bytes) => ContentPart::File {
                data: FileData::Data(STANDARD.encode(bytes)),
                media_type: mime.clone(),
                provider_options: match &target {
                    Target::Tool { image, .. } => image
                        .detail
                        .map(|detail| serde_json::json!({"openai":{"imageDetail":detail}})),
                    Target::User { .. } | Target::Audio { .. } => None,
                },
            },
            ImageRead::Unavailable(reason) => match &target {
                Target::User { image, .. } => ContentPart::text(format!(
                    "Image attachment \"{}\" could not be loaded: {reason}.",
                    image.name
                )),
                Target::Audio { .. } => ContentPart::text(format!(
                    "Audio could not be loaded from artifact storage: {reason}."
                )),
                Target::Tool { .. } => ContentPart::text(format!(
                    "Image could not be loaded from artifact storage: {reason}."
                )),
            },
        };
        match target {
            Target::User { message, .. } => {
                let Message::User { content, .. } = &mut messages[message] else {
                    unreachable!("user image target comes from the user message builder");
                };
                content.push(part);
            }
            Target::Audio {
                message,
                part: index,
                ..
            }
            | Target::Tool {
                message,
                part: index,
                ..
            } => {
                let Message::Tool { content, .. } = &mut messages[message] else {
                    unreachable!("tool image target comes from the tool message builder");
                };
                let ToolOutput::Content(parts) = &mut content[0].output else {
                    unreachable!("image target comes from a structured content output");
                };
                parts[index] = part;
            }
        }
    }
    if cancellation.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    Ok(messages)
}

async fn read(
    log: &EventLog,
    session: &str,
    reference: &StorageRef,
    audio: bool,
) -> Result<ImageRead, RunError> {
    let StorageRef::SessionFile {
        session_id,
        relative_path,
    } = reference
    else {
        return Ok(ImageRead::Unavailable("unsupported_ref_kind"));
    };
    if session_id != session {
        return Ok(ImageRead::Unavailable("session_mismatch"));
    }
    if maka_runtime::interaction::entity_id(relative_path).is_err() {
        return Ok(ImageRead::Unavailable("not_found"));
    }
    // Signatures fit within 1024 bytes. Read one bounded snapshot, enough to
    // distinguish an invalid image from a valid image exceeding the per-artifact read limit.
    let Some(chunk) = log
        .read_artifact_chunk(session, relative_path, 0, MEDIA_READ_LIMIT + 1)
        .await?
    else {
        return Ok(ImageRead::Unavailable("not_found"));
    };
    if !audio && sniff_binary_mime(&chunk.bytes).is_none() {
        return Ok(ImageRead::Unavailable("unsupported_mime"));
    }
    if chunk.total_bytes > MEDIA_READ_LIMIT as u64 {
        return Ok(ImageRead::Unavailable(
            "media exceeds the 12 MiB per-artifact read limit",
        ));
    }
    Ok(ImageRead::Bytes(chunk.bytes))
}
