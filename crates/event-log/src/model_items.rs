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

//! Streaming acceptance proofs reuse ingress validation, without a second
//! durable projection. Rebuild at effect/terminal boundaries, never per delta.
use crate::StoreError;
use futures_util::TryStreamExt;
use maka_runtime::{
    event::{Fact, RuntimeEvent},
    model::assembly::StepBuilder,
};
use sqlx::SqliteConnection;

pub(crate) async fn read(
    connection: &mut SqliteConnection,
    invocation: &str,
    step: &str,
) -> Result<Option<StepBuilder>, StoreError> {
    let enabled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_events WHERE invocation_id=? AND operation_id=? AND kind='model_requested' AND json_extract(event_json,'$.fact.item_acceptance')=1)")
        .bind(invocation).bind(step).fetch_one(&mut *connection).await?;
    if !enabled {
        return Ok(None);
    }
    let mut builder = StepBuilder::for_step(step).map_err(invalid)?;
    let mut rows = sqlx::query_scalar::<_, String>("SELECT event_json FROM runtime_events WHERE invocation_id=? AND kind='model_observed' AND json_extract(event_json,'$.fact.step_id')=? ORDER BY sequence")
        .bind(invocation).bind(step).fetch(connection);
    while let Some(json) = rows.try_next().await? {
        let event: RuntimeEvent = serde_json::from_str(&json)?;
        let Fact::ModelObserved { event, .. } = event.fact else {
            unreachable!()
        };
        builder.push(event).map_err(invalid)?;
    }
    Ok(Some(builder))
}

pub(crate) async fn validate(
    connection: &mut SqliteConnection,
    event: &RuntimeEvent,
) -> Result<(), StoreError> {
    if let Fact::ModelRequested {
        item_acceptance: true,
        purpose,
        ..
    } = &event.fact
        && *purpose != maka_runtime::context::ModelPurpose::Main
    {
        return Err(StoreError::InvalidTransition(
            "item acceptance is only valid for main requests".into(),
        ));
    }
    let step = match &event.fact {
        Fact::ModelCompleted { step_id, .. } | Fact::ModelInterrupted { step_id, .. } => step_id,
        _ => return Ok(()),
    };
    if let Some(builder) = read(connection, &event.invocation.invocation_id, step).await?
        && let Fact::ModelCompleted { output, .. } = &event.fact
        && builder.finish().map_err(invalid)? != *output
    {
        return Err(StoreError::InvalidTransition(
            "model completion differs from accepted observations".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn project(
    connection: &mut SqliteConnection,
    invocation: maka_runtime::event::Invocation,
    step_id: String,
    sequence: u64,
    event_id: String,
    max_bytes: usize,
) -> Result<crate::context::AcceptedModelStep, StoreError> {
    use maka_runtime::model::ModelEvent;
    let mut builder = StepBuilder::for_step(&step_id).map_err(invalid)?;
    let mut items = Vec::new();
    let mut scratch = 0usize;
    let mut rows = sqlx::query_as::<_, (i64, String)>("SELECT sequence,event_json FROM runtime_events WHERE invocation_id=?1 AND kind='model_observed' AND json_extract(event_json,'$.fact.step_id')=?2 AND sequence<?3 ORDER BY sequence")
        .bind(&invocation.invocation_id).bind(&step_id).bind(sequence as i64).fetch(connection);
    while let Some((position, json)) = rows.try_next().await? {
        let event: RuntimeEvent = serde_json::from_str(&json)?;
        let Fact::ModelObserved {
            event: observation, ..
        } = event.fact
        else {
            unreachable!()
        };
        // Count accumulated content, not the repeated per-token JSON envelope.
        scratch = scratch.saturating_add(match &observation {
            ModelEvent::PartDelta {
                text,
                provider_options,
                ..
            } => {
                text.len()
                    + provider_options
                        .as_ref()
                        .map_or(0, |value| value.to_string().len())
            }
            ModelEvent::ResponseMetadata { .. } | ModelEvent::Finished { .. } => 0,
            _ => serde_json::to_vec(&observation)?.len(),
        });
        if scratch > max_bytes {
            return Err(StoreError::PrefixTooLarge);
        }
        builder.push(observation).map_err(invalid)?;
        if let Some((index, part)) = builder.accepted_item() {
            items.push(crate::context::AcceptedModelItem {
                sequence: crate::sequence_number(position)?,
                event_id: event.id,
                index,
                part: part.clone(),
            });
        }
    }
    drop(rows);
    items.sort_by_key(|item| item.index);
    Ok(crate::context::AcceptedModelStep {
        sequence,
        event_id,
        invocation,
        step_id,
        items,
    })
}

fn invalid(error: impl std::fmt::Display) -> StoreError {
    StoreError::InvalidTransition(format!("invalid model item acceptance: {error}"))
}

// One selection rule for recovery, context fences and archive causality.
macro_rules! accepted_calls {
    () => { "accepted_calls AS NOT MATERIALIZED (
        SELECT m.invocation_id, m.operation_id AS step_id, m.sequence, m.event_id,
               json_extract(p.value,'$.call') AS call
        FROM runtime_events m, json_each(m.event_json,'$.fact.output.parts') p
        WHERE m.kind='model_completed' AND json_extract(p.value,'$.kind')='tool_call'
          AND NOT EXISTS(SELECT 1 FROM runtime_events r WHERE r.invocation_id=m.invocation_id
            AND r.operation_id=m.operation_id AND r.kind='model_requested'
            AND json_extract(r.event_json,'$.fact.item_acceptance')=1)
        UNION ALL
        SELECT o.invocation_id, r.operation_id, o.sequence, o.event_id,
               json_extract(o.event_json,'$.fact.event.data')
        FROM runtime_events r JOIN runtime_events o ON o.invocation_id=r.invocation_id
          AND o.kind='model_observed' AND json_extract(o.event_json,'$.fact.step_id')=r.operation_id
        WHERE r.kind='model_requested' AND json_extract(r.event_json,'$.fact.item_acceptance')=1
          AND json_extract(o.event_json,'$.fact.event.kind')='tool_call'
    )" };
}
pub(crate) use accepted_calls;
