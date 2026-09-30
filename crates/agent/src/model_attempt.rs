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

use crate::{
    Inner, RunError, RunInput, history,
    runner::{append, digest},
};
use maka_event_log::context::ModelContextSource;
use maka_model::prompt::Message;
use maka_model::{ModelRequest, StepBuilder, ToolDefinition};
use maka_runtime::{
    context::ModelPurpose,
    event::{Fact, ModelInterruption},
    model::ModelStep,
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(super) enum Attempt<'a> {
    Main {
        lane: maka_model::Conversation,
        continuation_base: Option<u64>,
        surface: Arc<crate::request_composition::Surface>,
        tools: &'a maka_tools::RequestTools<'a>,
        prior_unknown: bool,
    },
    Summary {
        adapter: maka_plugins::model::Binding,
        reservation: Option<&'a mut maka_model::ModelReservation>,
    },
}

#[expect(
    clippy::large_enum_variant,
    reason = "One result per request; keep the ordinary response inline without another allocation."
)]
pub(super) enum Outcome {
    Response(String, ModelStep),
    FinishedByTool,
}
impl From<(String, ModelStep)> for Outcome {
    fn from((step, output): (String, ModelStep)) -> Self {
        Self::Response(step, output)
    }
}

#[allow(clippy::too_many_arguments)] // Keep the admitted history policy explicit at each model request.
pub(super) async fn prompt(
    inner: &Inner,
    input: &RunInput,
    source: &ModelContextSource,
    purpose: ModelPurpose,
    cancellation: &CancellationToken,
    continuation_base: Option<u64>,
    current: &str,
    prior_unknown: bool,
) -> Result<Vec<Message>, RunError> {
    let route = route_identity(input)?;
    let replay = continuation_base.map(|base| history::Replay {
        base,
        current,
        route: &route,
        model: &input.provider.model,
    });
    let mut prompt = history::materialize_replay(
        &inner.log,
        &source.tail,
        source.anchor.as_ref(),
        &input.invocation.session_id,
        input.supports_vision,
        cancellation,
        replay,
        prior_unknown,
    )
    .await?;
    if let Some(baseline) = &source.baseline {
        let text = match purpose {
            ModelPurpose::Summary => format!(
                "Previous continuation summary:\n{}\n\nUpdate it using the newer conversation events that follow.",
                baseline.checkpoint.summary.text
            ),
            ModelPurpose::Main => format!(
                "Continuation summary:\n{}",
                baseline.checkpoint.summary.text
            ),
        };
        prompt.insert(0, Message::user(text));
    }
    if purpose == ModelPurpose::Main {
        let system_text = input
            .configuration
            .system_prompt
            .as_ref()
            .map(|system| system.text.clone())
            .unwrap_or_default();
        if !system_text.is_empty() {
            prompt.insert(
                0,
                Message::System {
                    content: system_text,
                    provider_options: None,
                },
            );
        }
    }
    Ok(prompt)
}

pub(super) fn prior_unknown_notice(source: &ModelContextSource) -> String {
    let events =
        source
            .anchor
            .iter()
            .map(|event| &event.event)
            .chain(source.tail.iter().filter_map(|event| match event {
                maka_event_log::context::ContextEvent::Canonical(event) => Some(&event.event),
                maka_event_log::context::ContextEvent::Archived(_)
                | maka_event_log::context::ContextEvent::ModelItems(_) => None,
            }));
    let events: Vec<_> = events.collect();
    let sealed: HashSet<_> = events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::InvocationEnded {
                outcome: maka_runtime::event::InvocationOutcome::Failed { class, .. },
            } if class == "outcome_unknown" => Some(event.invocation.invocation_id.as_str()),
            _ => None,
        })
        .collect();
    let settled: HashSet<_> = events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::ToolSettled { operation_id, .. } => Some((
                event.invocation.invocation_id.as_str(),
                operation_id.as_str(),
            )),
            _ => None,
        })
        .collect();
    let unknown: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::ToolDispatched {
                operation_id, name, ..
            } if sealed.contains(event.invocation.invocation_id.as_str())
                && !settled.contains(&(
                    event.invocation.invocation_id.as_str(),
                    operation_id.as_str(),
                )) =>
            {
                Some((name.as_str(), operation_id.as_str()))
            }
            _ => None,
        })
        .collect();
    if unknown.is_empty() {
        return String::new();
    }
    let mut notice = String::from(
        "Prior execution was interrupted after these tools were dispatched. Their results were never recorded; effects may or may not have happened. Inspect current state before repeating any action. This is historical uncertainty, not a new tool result:",
    );
    for (name, operation) in unknown.iter().take(64) {
        notice.push_str(&format!("\n- {name} (operation {operation})"));
    }
    if unknown.len() > 64 {
        notice.push_str(&format!(
            "\n- and {} more unknown operations",
            unknown.len() - 64
        ));
    }
    notice
}

pub(super) async fn execute(
    inner: &Arc<Inner>,
    input: &RunInput,
    source: &mut ModelContextSource,
    mut prompt: Vec<Message>,
    definitions: Vec<ToolDefinition>,
    attempt: Attempt<'_>,
    cancellation: &CancellationToken,
) -> Result<Outcome, RunError> {
    let Attempt::Main {
        lane,
        continuation_base,
        surface,
        tools,
        prior_unknown,
    } = attempt
    else {
        return execute_once(
            inner,
            input,
            source,
            prompt,
            definitions,
            attempt,
            cancellation,
            &mut false,
        )
        .await
        .map(Outcome::from);
    };
    let mut failures = 0;
    loop {
        // Preserve captured capabilities. Rebuild history only when a completed
        // item advanced canonical progress, never from a shortened WS request.
        let mut progressed = false;
        let result = execute_once(
            inner,
            input,
            source,
            prompt.clone(),
            definitions.clone(),
            Attempt::Main {
                lane: lane.clone(),
                continuation_base,
                surface: surface.clone(),
                tools,
                prior_unknown,
            },
            cancellation,
            &mut progressed,
        )
        .await;
        let delay = match &result {
            Err(RunError::Model(maka_model::ModelError::Provider(failure)))
                if failure.replay_safe() || (progressed && failure.retained_output_safe()) =>
            {
                if cancellation.is_cancelled() {
                    return Err(RunError::Cancelled);
                }
                // A successful finishing effect is already authoritative. Safe
                // stream recovery must not reopen this turn for another model.
                if tools.finished() {
                    return Ok(Outcome::FinishedByTool);
                }
                if failures >= 9 {
                    if !lane.try_switch_fallback_transport() {
                        return result.map(Outcome::from);
                    }
                    failures = 0;
                    failure.retry_after().unwrap_or_default()
                } else {
                    let base_ms = 1_000u64 << failures.min(5);
                    failures += 1;
                    failure.retry_after().unwrap_or_else(|| {
                        std::time::Duration::from_millis(base_ms + fastrand::u64(0..=base_ms / 4))
                    })
                }
            }
            _ => return result.map(Outcome::from),
        };
        if progressed {
            let next = if prior_unknown {
                inner
                    .log
                    .read_manual_message_context(
                        &input.invocation.session_id,
                        &input.invocation.invocation_id,
                        maka_runtime::context::MAX_HISTORY_EVENTS,
                        maka_runtime::context::MAX_HISTORY_BYTES,
                    )
                    .await?
            } else {
                inner
                    .log
                    .read_model_context(
                        &input.invocation.session_id,
                        Some(&input.invocation.invocation_id),
                        maka_runtime::context::MAX_HISTORY_EVENTS,
                        maka_runtime::context::MAX_HISTORY_BYTES,
                    )
                    .await?
            };
            prompt = surface.apply(
                self::prompt(
                    inner,
                    input,
                    &next,
                    ModelPurpose::Main,
                    cancellation,
                    continuation_base,
                    &input.invocation.invocation_id,
                    prior_unknown,
                )
                .await?,
            );
            *source = next;
        }
        // execute_once has drained the worker and committed ModelInterrupted.
        // A local/storage error cannot reach this wait or authorize another send.
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(RunError::Cancelled),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_once(
    inner: &Arc<Inner>,
    input: &RunInput,
    source: &ModelContextSource,
    prompt: Vec<Message>,
    definitions: Vec<ToolDefinition>,
    attempt: Attempt<'_>,
    cancellation: &CancellationToken,
    progressed: &mut bool,
) -> Result<(String, ModelStep), RunError> {
    let (purpose, lane, surface, summary_adapter, reservation, tools) = match attempt {
        Attempt::Main {
            lane,
            surface,
            tools,
            ..
        } => (
            ModelPurpose::Main,
            Some(lane),
            Some(surface),
            None,
            None,
            Some(tools),
        ),
        Attempt::Summary {
            adapter,
            reservation,
        } => (
            ModelPurpose::Summary,
            None,
            None,
            Some(adapter),
            reservation,
            None,
        ),
    };
    if cancellation.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    let max_output_tokens = surface.as_ref().map_or(
        Some(input.main_output_limit.unwrap_or(8000).min(8000)),
        |surface| surface.max_output_tokens,
    );
    let prepared = prepare_request(input, prompt, definitions, max_output_tokens)?;
    let (binding, composition) = match surface {
        Some(surface) => (surface.adapter.clone(), surface.evidence.clone()),
        None => {
            let request = &prepared.request;
            let binding = summary_adapter.expect("summary adapter frozen at capture");
            let evidence = maka_runtime::composition::RequestComposition {
                system_prompt: request.prompt.iter().find_map(|message| match message {
                    Message::System { content, .. } => Some(content.clone()),
                    _ => None,
                }),
                dynamic_context: vec![],
                tool_catalog_digest: maka_runtime::artifact::content_digest(
                    &serde_json::to_vec(&request.tools)
                        .map_err(|error| RunError::Internal(error.to_string()))?,
                ),
                tools: request.tools.clone(),
                provider_options: Some(request.provider_options.clone()),
                max_output_tokens: request.max_output_tokens,
                sources: input
                    .model_revision
                    .iter()
                    .cloned()
                    .chain(std::iter::once(
                        binding.source(request.provider.adapter_name())?,
                    ))
                    .collect(),
            }
            .freeze()
            .map_err(|error| RunError::Internal(error.into()))?;
            (binding, Arc::new(evidence))
        }
    };
    let item_acceptance = purpose == ModelPurpose::Main && binding.supports_item_acceptance();
    let step_id = Uuid::new_v4().to_string();
    let event = maka_runtime::event::RuntimeEvent::new(
        input.invocation.clone(),
        Fact::ModelRequested {
            item_acceptance,
            effective_source_digest: Some(source.effective_source_digest.clone()),
            purpose,
            context: input.context.clone(),
            step_id: step_id.clone(),
            model_id: input.provider.model.clone(),
            source_scope: source.source_evidence.scope.clone(),
            source_high_water: source.source_evidence.high_water,
            source_digest: source.source_evidence.digest.clone(),
            input_digest: prepared.input_digest,
            route_identity: prepared.route_identity,
            checkpoint_event_id: source
                .baseline
                .as_ref()
                .map(|baseline| baseline.event_id.clone()),
        },
    );
    let mut write = maka_runtime::event::EventWrite::plain(event)?.with_composition(composition)?;
    if let Some(pricing) = &inner.pricing {
        write = write.with_quote(
            pricing
                .quote(&input.provider_id, &input.provider.model)
                .await?,
        )?;
    }
    use maka_runtime::event::EventSink;
    inner.log.clone().commit(write).await?;
    let result: Result<_, RunError> = async {
        let stream = match reservation {
            Some(reservation) => {
                reservation
                    .stream_with_adapter(prepared.request, cancellation.clone(), binding)
                    .await?
            }
            None => {
                inner
                    .model
                    .stream_with_adapter(prepared.request, cancellation.clone(), lane, binding)
                    .await?
            }
        };
        receive(
            inner,
            input,
            &step_id,
            purpose,
            stream,
            tools
                .filter(|_| item_acceptance)
                .map(|tools| tools.clone().into_step(&step_id)),
            progressed,
            cancellation,
        )
        .await
    }
    .await;
    let accepted = finish(
        inner,
        input,
        step_id,
        purpose,
        result,
        cancellation,
        item_acceptance,
    )
    .await?;
    if !item_acceptance && let Some(tools) = tools {
        use futures_util::FutureExt;
        let mut step = tools.clone().into_step(&accepted.0);
        for call in accepted
            .1
            .tool_calls()
            .filter(|call| !call.provider_executed)
        {
            let result = std::panic::AssertUnwindSafe(step.invoke(call, cancellation.clone()))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(maka_runtime::tools::ToolError::CleanupUnconfirmed(
                        "tool panicked".into(),
                    ))
                });
            tool_result(result)?;
        }
    }
    Ok(accepted)
}

pub(super) struct PreparedRequest {
    pub request: ModelRequest,
    pub input_digest: String,
    pub route_identity: String,
}

pub(super) fn route_identity(input: &RunInput) -> Result<String, RunError> {
    Ok(digest(
        &serde_json::to_vec(&input.provider)
            .map_err(|error| RunError::Internal(error.to_string()))?,
    ))
}

pub(super) fn prepare_request(
    input: &RunInput,
    prompt: Vec<Message>,
    definitions: Vec<ToolDefinition>,
    max_output_tokens: Option<u64>,
) -> Result<PreparedRequest, RunError> {
    let max_output_tokens = Some(max_output_tokens.unwrap_or(8000));
    let prompt = if input.supports_vision
        && matches!(
            &input.provider.kind,
            maka_model::ProviderKind::OpenaiChat
                | maka_model::ProviderKind::OpenaiCompatible { .. }
        ) {
        history::project_chat(prompt)
    } else {
        prompt
    };
    let prompt = history::project_compatible(prompt, &input.provider.kind);
    let prompt = maka_model::reasoning::project(prompt, &input.provider.kind);
    let mut evidence = json!({"projection":"maka.model-history.v1","prompt":prompt,
        "tools":definitions,"providerOptions":input.provider_options});
    if let Some(limit) = max_output_tokens {
        evidence["maxOutputTokens"] = json!(limit);
    }
    let input_digest = digest(
        &serde_json::to_vec(&evidence).map_err(|error| RunError::Internal(error.to_string()))?,
    );
    Ok(PreparedRequest {
        request: ModelRequest {
            provider: input.provider.clone(),
            prompt,
            tools: definitions,
            provider_options: input.provider_options.clone(),
            max_output_tokens,
        },
        input_digest,
        route_identity: route_identity(input)?,
    })
}

#[allow(clippy::too_many_arguments)]
async fn receive(
    inner: &Arc<Inner>,
    input: &RunInput,
    step_id: &str,
    purpose: ModelPurpose,
    mut stream: maka_model::ModelStream,
    mut tools: Option<maka_tools::StepTools<'_>>,
    progressed: &mut bool,
    cancellation: &CancellationToken,
) -> Result<ModelStep, RunError> {
    use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
    use maka_runtime::{model::ModelEvent, tools::ToolError};
    let mut builder = StepBuilder::for_step(step_id)?;
    let tool_cancellation = cancellation.child_token();
    let mut running = FuturesUnordered::new();
    // Bound effects without stopping stream ingestion while a tool awaits user
    // input or slow I/O. Native item counts bound the pending call collection.
    let permits = Arc::new(tokio::sync::Semaphore::new(8));
    let result: Result<_, RunError> = async {
        loop {
            tokio::select! {
                biased;
                result = running.next(), if !running.is_empty() => { tool_result(result.unwrap())?; }
                event = stream.next() => {
                    let Some(event) = event else { break; };
                    let event = event?;
                    builder.push(event.clone())?;
                    append(inner, &input.invocation, Fact::ModelObserved {
                        step_id: step_id.to_owned(), event: event.clone(),
                    }).await?;
                    if tools.is_some() && builder.accepted_item().is_some() {
                        *progressed = true;
                    }
                    if let ModelEvent::ToolCall(call) = event
                        && !call.provider_executed
                        && let Some(tools) = tools.as_mut()
                    {
                        let future = tools.dispatch(call, tool_cancellation.clone());
                        let permits = permits.clone();
                        running.push(async move {
                            let _permit = permits.acquire_owned().await.expect("tool execution gate remains open");
                            std::panic::AssertUnwindSafe(future).catch_unwind().await
                                .unwrap_or_else(|_| Err(ToolError::CleanupUnconfirmed("tool panicked".into())))
                        });
                    }
                }
            }
        }
        builder.finish().map_err(Into::into)
    }.await;
    stream.cancel_and_wait().await;
    if matches!(result, Err(ref error) if !matches!(error, RunError::Model(_))) {
        tool_cancellation.cancel();
    }
    let mut cleanup = Ok(());
    while let Some(result) = running.next().await {
        if let Err(error) = tool_result(result) {
            tool_cancellation.cancel();
            if cleanup.is_ok() {
                cleanup = Err(error);
            }
        }
    }
    cleanup?;
    let output = result?;
    if purpose == ModelPurpose::Summary
        && output.parts.iter().any(|part| {
            matches!(
                part,
                maka_runtime::model::ModelPart::ToolCall { .. }
                    | maka_runtime::model::ModelPart::ToolResult { .. }
            )
        })
    {
        return Err(maka_model::ModelError::Adapter("summary contains tool content".into()).into());
    }
    Ok(output)
}

fn tool_result(
    result: Result<serde_json::Value, maka_runtime::tools::ToolError>,
) -> Result<(), RunError> {
    use maka_runtime::tools::ToolError;
    match result {
        Ok(_) | Err(ToolError::Failed(_) | ToolError::Io { .. } | ToolError::OutcomeUnknown(_)) => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn finish(
    inner: &Arc<Inner>,
    input: &RunInput,
    step_id: String,
    purpose: ModelPurpose,
    result: Result<ModelStep, RunError>,
    cancellation: &CancellationToken,
    item_acceptance: bool,
) -> Result<(String, ModelStep), RunError> {
    let output = match result {
        Ok(output) => output,
        Err(RunError::Model(error)) => {
            let status = match error {
                maka_model::ModelError::Cancelled => ModelInterruption::Cancelled,
                maka_model::ModelError::TimedOut => ModelInterruption::TimedOut,
                maka_model::ModelError::Adapter(_) => ModelInterruption::Failed,
                maka_model::ModelError::ContextOverflow { .. } => ModelInterruption::Failed,
                maka_model::ModelError::Provider(ref failure)
                    if failure.replay_safe()
                        || (item_acceptance && failure.retained_output_safe()) =>
                {
                    ModelInterruption::RetryableFailure
                }
                maka_model::ModelError::Provider(_) => ModelInterruption::Failed,
            };
            append(
                inner,
                &input.invocation,
                Fact::ModelInterrupted {
                    step_id: step_id.clone(),
                    status,
                },
            )
            .await?;
            return Err(error.into());
        }
        Err(error) => return Err(error),
    };
    use maka_runtime::event::{EventWrite, RuntimeEvent};
    let incomplete = purpose == ModelPurpose::Main
        && output.finish_reason == maka_runtime::model::ModelFinishReason::Length;
    let mut writes = vec![EventWrite::plain(RuntimeEvent::new(
        input.invocation.clone(),
        Fact::ModelCompleted {
            step_id: step_id.clone(),
            output: output.clone(),
        },
    ))?];
    if incomplete && !item_acceptance {
        for call in output.tool_calls().filter(|call| !call.provider_executed) {
            writes.push(EventWrite::plain(RuntimeEvent::new(
                input.invocation.clone(),
                Fact::ToolRejected {
                    operation_id: format!("{step_id}:{}", call.id),
                    call: maka_runtime::tool_call::ToolCallIdentity::provider(
                        step_id.clone(),
                        call.id.clone(),
                    ),
                    name: call.name.clone(),
                    input: call.input.clone(),
                    reason: maka_runtime::tool_call::ToolRejection::PreparationFailed {
                        message: RunError::ModelIncomplete.to_string(),
                    },
                },
            ))?);
        }
    }
    inner.log.append_batch(&writes).await?;
    if incomplete {
        return Err(RunError::ModelIncomplete);
    }
    if cancellation.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    Ok((step_id, output))
}
