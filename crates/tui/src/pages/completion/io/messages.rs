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

use super::{Message, Output, Request};
use crate::pages::completion::model::{Binding, Origin, Payload};
use maka_client::Client;
use maka_protocol::session::workspace_context as workspace;

pub(super) async fn messages(
    client: &Client,
    request: &Request,
    session: &str,
    name: &str,
    anchor: Option<u64>,
    query: &str,
) -> Result<Output, String> {
    use maka_protocol::{
        subscription::{SubscriptionOpenInput, TranscriptPolicy},
        transcript::{SessionTranscriptPageDirection, SessionTranscriptPageInput},
    };
    use maka_runtime::input::{CaptureTime, QuoteRef, SessionQuoteSource};
    let opened = client
        .open_subscription(SubscriptionOpenInput {
            session_id: session.into(),
            transcript: TranscriptPolicy::Tail { max_bytes: 1 },
        })
        .await
        .map_err(|error| error.to_string())?;
    let id = opened.subscription_id.clone();
    let result = request
        .cancel
        .read(async {
            if opened.host_epoch != request.context.epoch
                || opened.snapshot.session.session_id != session
            {
                return Err("Transcript identity changed".to_owned());
            }
            let announced = opened
                .transcript
                .as_ref()
                .and_then(|transcript| transcript.durable.through_sequence);
            let through = match anchor {
                Some(anchor) if announced.is_some_and(|latest| anchor <= latest) => Some(anchor),
                Some(_) => return Err("Transcript capture fence is unavailable".into()),
                None => announced,
            };
            let page = client
                .transcript_page(SessionTranscriptPageInput {
                    subscription_id: id.clone(),
                    direction: SessionTranscriptPageDirection::Older,
                    through_sequence: through,
                    cursor: None,
                    anchor_sequence: None,
                    max_bytes: workspace::CAPTURE_BYTES as u64,
                })
                .await
                .map_err(|error| error.to_string())?;
            client
                .complete_transcript_page(&id, page)
                .await
                .map_err(|error| error.to_string())
        })
        .await;
    if let Err(error) = client.close_subscription(&id).await {
        client.disconnect();
        return Err(format!(
            "Transcript capture cleanup was not confirmed; connection retired: {error}"
        ));
    }
    let batch = result?;
    // A signed cursor belongs to its subscription. A new finite read resumes
    // with the oldest complete row minus one as an inclusive upper fence.
    let next = batch
        .next_cursor
        .as_ref()
        .and_then(|_| batch.rows.first()?.sequence.checked_sub(1))
        .filter(|sequence| *sequence > 0);
    let captured_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(8_640_000_000_000_000) as u64;
    let captured_at =
        CaptureTime::try_from(serde_json::Number::from(captured_at)).map_err(str::to_owned)?;
    let mut items = vec![];
    let mut excerpt = String::new();
    let mut excerpt_truncated = anchor.is_some() || next.is_some();
    for row in batch.rows {
        if !matches!(row.value["type"].as_str(), Some("user" | "assistant")) {
            continue;
        }
        let Some(text) = row.value["text"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
        else {
            continue;
        };
        let Some(message) = row.value["id"].as_str() else {
            return Err("Transcript message has no identity".into());
        };
        let Some(turn) = row.value["turnId"].as_str() else {
            return Err("Transcript message has no turn".into());
        };
        let captured = excerpt_prefix(text, workspace::CAPTURE_BYTES);
        let truncated = captured.len() != text.len();
        let label: String = text
            .trim_start()
            .chars()
            .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
            .take(72)
            .collect();
        let quote = QuoteRef {
            text: captured,
            label: Some(label.clone()),
            source_turn_id: Some(turn.into()),
            source: Some(SessionQuoteSource {
                session_id: session.into(),
                session_name: excerpt_prefix(name, 200),
                captured_at: captured_at.clone(),
                truncated,
            }),
        };
        let remaining = workspace::CAPTURE_BYTES.saturating_sub(excerpt.len() + 2);
        if remaining > 0 {
            let part = excerpt_prefix(&quote.text, remaining);
            if !excerpt.is_empty() {
                excerpt.push_str("\n\n");
            }
            excerpt.push_str(&part);
            excerpt_truncated |= part.len() != text.len();
        } else {
            excerpt_truncated = true;
        }
        if super::super::candidates::matches(query, [text]) {
            items.push(Message {
                sequence: row.sequence,
                id: message.into(),
                turn: turn.into(),
                binding: Binding {
                    label,
                    origin: Origin::Session { name: name.into() },
                    inline: None,
                    payload: Payload::Context {
                        quote,
                        directory: None,
                    },
                },
            });
        }
    }
    let conversation = (!excerpt.is_empty()).then(|| Binding {
        label: name.into(),
        origin: Origin::Session { name: name.into() },
        inline: None,
        payload: Payload::Context {
            quote: QuoteRef {
                text: excerpt,
                label: Some(excerpt_prefix(name, 200)),
                source_turn_id: None,
                source: Some(SessionQuoteSource {
                    session_id: session.into(),
                    session_name: excerpt_prefix(name, 200),
                    captured_at,
                    truncated: excerpt_truncated,
                }),
            },
            directory: None,
        },
    });
    Ok(Output::Messages {
        items,
        next,
        conversation,
    })
}

fn excerpt_prefix(text: &str, maximum: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let end = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| start + grapheme.len())
        .take_while(|end| *end <= maximum)
        .last()
        .unwrap_or(0);
    text[..end].to_owned()
}
