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

use super::super::*;
use serde::{Deserialize, Serialize};

/// A history reference proves both transport position and durable object identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub session: String,
    pub turn: Option<String>,
    pub message: Option<String>,
    pub sequence: u64,
}
impl Anchor {
    pub fn valid(&self) -> bool {
        let valid = |s: &str| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control);
        valid(&self.session)
            && self.turn.as_deref().is_none_or(valid)
            && self.message.as_deref().is_none_or(valid)
            && self.sequence > 0
            && self.sequence <= 9_007_199_254_740_991
    }
}
#[derive(Clone)]
pub struct AnchorRead {
    generation: u64,
    pub anchor: Anchor,
    input: SessionTranscriptPageInput,
}
impl Chat {
    /// Read acknowledgement refers to a durable message actually exposed by this reader.
    pub fn read_marker_message(&self) -> Option<String> {
        if self.area.is_none() || self.error.is_some() || self.removed || self.view.search.is_some()
        {
            return None;
        }
        let key = self.view.selection()?;
        self.rows
            .values()
            .any(|row| row["id"] == key.message() && row["turnId"] == key.turn())
            .then(|| key.message().to_owned())
    }
    pub fn begin_anchor(&mut self, anchor: Anchor) -> Result<AnchorRead, &'static str> {
        if !anchor.valid()
            || self.session.as_deref() != Some(&anchor.session)
            || self.snapshot.is_none()
        {
            return Err("controls-anchor-unavailable");
        }
        let through = self.wanted.or(self.through);
        if through.is_none_or(|through| anchor.sequence > through) {
            return Err("controls-anchor-unavailable");
        }
        let subscription = self
            .subscription
            .clone()
            .ok_or("controls-anchor-unavailable")?;
        self.generation += 1;
        self.paging = true;
        self.older_requested = false;
        self.newer_requested = false;
        self.latest_requested = false;
        Ok(AnchorRead {
            generation: self.generation,
            input: SessionTranscriptPageInput {
                subscription_id: subscription,
                direction: SessionTranscriptPageDirection::Newer,
                through_sequence: through,
                cursor: None,
                anchor_sequence: Some(anchor.sequence - 1),
                max_bytes: 1,
            },
            anchor,
        })
    }
    pub fn complete_anchor(
        &mut self,
        request: AnchorRead,
        result: Result<TranscriptBatch, String>,
        i18n: &I18n,
        ascii: bool,
    ) -> Result<bool, String> {
        if request.generation != self.generation
            || self.session.as_deref() != Some(&request.anchor.session)
            || self.subscription.as_ref() != Some(&request.input.subscription_id)
        {
            return Ok(false);
        }
        self.paging = false;
        let batch = result?;
        if batch.rows.len() != 1
            || batch.rows[0].sequence != request.anchor.sequence
            || request
                .anchor
                .turn
                .as_ref()
                .is_some_and(|id| batch.rows[0].value["turnId"] != *id)
            || request
                .anchor
                .message
                .as_ref()
                .is_some_and(|id| batch.rows[0].value["id"] != *id)
        {
            return Err(i18n.text("controls-anchor-unavailable"));
        }
        validate_row(&batch.rows[0].value).map_err(|e| e.to_string())?;
        let trace = self.view.trace;
        self.rows.clear();
        self.bytes = 0;
        self.history = None;
        self.view = render::Transcript::default();
        self.view.trace = trace;
        self.through = Some(request.anchor.sequence);
        self.older = (request.anchor.sequence > 1)
            .then(|| (Some(request.anchor.sequence - 1), String::new()));
        self.merge(batch, SessionTranscriptPageDirection::Newer)
            .map_err(|e| e.to_string())?;
        self.reading_history = true;
        self.fill_blocked = true;
        self.presentation.sync(
            &mut self.view,
            &self.rows,
            &[],
            self.live_revision,
            i18n,
            ascii,
        );
        self.view.enter();
        self.view.pause();
        self.dirty = true;
        Ok(true)
    }
}
pub async fn execute_anchor(
    client: &Client,
    request: &AnchorRead,
) -> Result<TranscriptBatch, String> {
    let page = client
        .transcript_page(request.input.clone())
        .await
        .map_err(|e| e.to_string())?;
    client
        .complete_transcript_page(&request.input.subscription_id, page)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchor_rejects_future_zero_and_ambiguous_identity() {
        let anchor = Anchor {
            session: "s".into(),
            turn: Some("t".into()),
            message: Some("m".into()),
            sequence: 1,
        };
        assert!(anchor.valid());
        assert!(
            !Anchor {
                sequence: 0,
                ..anchor.clone()
            }
            .valid()
        );
        assert!(
            !Anchor {
                message: Some(String::new()),
                ..anchor.clone()
            }
            .valid()
        );
        assert!(Chat::default().begin_anchor(anchor).is_err());
    }
}
