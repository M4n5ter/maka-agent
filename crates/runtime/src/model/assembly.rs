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

use super::error::ModelError;
use super::merge_provider_options as merge;
fn invalid(message: &str) -> ModelError {
    ModelError::Adapter(message.into())
}
use super::{ModelEvent, ModelFinishReason, ModelPart, ModelStep, ModelUsage};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
mod identity;
#[derive(Default)]
enum StepState {
    #[default]
    Assembling,
    Finished {
        reason: ModelFinishReason,
        usage: ModelUsage,
        provider_options: Option<Value>,
    },
    Poisoned,
}

/// Shared validation and assembly for Host ingress, durable acceptance proofs
/// and history projection. Only closed items can authorize streaming execution.
#[derive(Default)]
pub struct StepBuilder {
    step_id_bytes: Option<usize>,
    parts: Vec<ModelPart>,
    accepted: Option<usize>,
    open: HashMap<String, usize>,
    seen: HashSet<String>,
    calls: HashMap<String, (String, bool)>,
    results: HashSet<String>,
    state: StepState,
    response_id: Option<String>,
    model: Option<String>,
    timestamp: Option<String>,
}
impl StepBuilder {
    pub fn push(&mut self, event: ModelEvent) -> Result<(), ModelError> {
        self.accepted = None;
        let result = self.apply(event);
        if result.is_err() {
            self.state = StepState::Poisoned;
        }
        result
    }
    /// The complete item admitted by the most recent valid observation. Its
    /// index is allocated when it starts, independent of completion order.
    pub fn accepted_item(&self) -> Option<(usize, &ModelPart)> {
        self.accepted.map(|index| (index, &self.parts[index]))
    }

    pub fn accepted_parts(&self) -> impl Iterator<Item = (usize, &ModelPart)> {
        self.parts
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.open.values().any(|open| open == index))
    }

    fn apply(&mut self, event: ModelEvent) -> Result<(), ModelError> {
        if !matches!(self.state, StepState::Assembling) {
            return Err(invalid("event after failed or finished step"));
        }
        match event {
            ModelEvent::Source(source) => {
                self.accepted = Some(self.parts.len());
                self.parts.push(ModelPart::Source { source });
            }
            ModelEvent::PartStarted {
                id,
                text_kind,
                provider_options,
            } => {
                if !self.seen.insert(id.clone()) {
                    return Err(invalid("duplicate model part"));
                }
                self.open.insert(id, self.parts.len());
                self.parts.push(ModelPart::Text {
                    text_kind,
                    text: String::new(),
                    provider_options,
                });
            }
            ModelEvent::PartDelta {
                id,
                text,
                provider_options,
            } => {
                let index = *self
                    .open
                    .get(&id)
                    .ok_or_else(|| invalid("delta without open part"))?;
                if let ModelPart::Text {
                    text: accumulated,
                    provider_options: metadata,
                    ..
                } = &mut self.parts[index]
                {
                    accumulated.push_str(&text);
                    merge(metadata, provider_options);
                }
            }
            ModelEvent::PartFinished {
                id,
                provider_options,
            } => {
                let index = self
                    .open
                    .remove(&id)
                    .ok_or_else(|| invalid("end without open part"))?;
                if let ModelPart::Text {
                    provider_options: metadata,
                    ..
                } = &mut self.parts[index]
                {
                    merge(metadata, provider_options);
                }
                self.accepted = Some(index);
            }
            ModelEvent::ToolCall(call) => {
                self.validate_call(&call)?;
                if self
                    .calls
                    .insert(call.id.clone(), (call.name.clone(), call.provider_executed))
                    .is_some()
                {
                    return Err(invalid("duplicate tool call"));
                }
                self.accepted = Some(self.parts.len());
                self.parts.push(ModelPart::ToolCall { call });
            }
            ModelEvent::ProviderToolResult {
                id,
                name,
                output,
                is_error,
                provider_options,
            } => {
                if self.calls.get(&id) != Some(&(name.clone(), true))
                    || !self.results.insert(id.clone())
                {
                    return Err(invalid(
                        "provider result without matching outstanding provider call",
                    ));
                }
                self.accepted = Some(self.parts.len());
                self.parts.push(ModelPart::ToolResult {
                    id,
                    name,
                    output,
                    is_error,
                    provider_options,
                });
            }
            ModelEvent::ResponseMetadata {
                id,
                model,
                timestamp,
            } => {
                if id.is_some() {
                    self.response_id = id;
                }
                if model.is_some() {
                    self.model = model;
                }
                if timestamp.is_some() {
                    self.timestamp = timestamp;
                }
            }
            ModelEvent::Finished {
                reason,
                usage,
                provider_options,
            } => {
                if !self.open.is_empty() {
                    return Err(invalid("incomplete model step"));
                }
                if self
                    .calls
                    .iter()
                    .any(|(id, (_, provider))| *provider && !self.results.contains(id))
                {
                    return Err(invalid("provider tool result missing"));
                }
                self.state = StepState::Finished {
                    reason,
                    usage,
                    provider_options,
                };
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Result<ModelStep, ModelError> {
        let (finish_reason, usage, provider_options) = match self.state {
            StepState::Assembling => return Err(invalid("model step missing finish")),
            StepState::Poisoned => return Err(invalid("model step previously failed")),
            StepState::Finished {
                reason,
                usage,
                provider_options,
            } => (reason, usage, provider_options),
        };
        Ok(ModelStep {
            parts: self.parts,
            finish_reason,
            usage,
            provider_options,
            response_id: self.response_id,
            model: self.model,
            timestamp: self.timestamp,
        })
    }
}
