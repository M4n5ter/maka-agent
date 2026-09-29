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

use super::{Executions, plugins::llm, provider};
use crate::session::SessionConfiguration;
use maka_event_log::{sessions::SessionMutation, usage::AuxiliarySource};
use maka_plugins::{
    composition::Scope,
    session::title::{NAME, SessionTitlePolicy},
};
use maka_runtime::{
    event::{Fact, Invocation, InvocationInput},
    tools::ToolError,
};
use std::{sync::Arc, time::Duration};

impl Executions {
    /// A derived title is owned by the Host, not by the turn's Stop token.
    /// No task is scheduled by exact retries or Host startup recovery.
    pub(super) fn title_after_message(self: &Arc<Self>, invocation: Invocation) {
        let host = self.clone();
        self.workers.spawn(async move {
            if let Err(ToolError::Persistence(error) | ToolError::CleanupUnconfirmed(error)) =
                host.generate_title(invocation).await
            {
                host.begin_drain();
                eprintln!("Automatic title persistence failed: {error}");
            }
            host.request_removal_recovery();
        });
    }

    async fn generate_title(self: &Arc<Self>, invocation: Invocation) -> Result<(), ToolError> {
        let session_id = &invocation.session_id;
        let Some(session) = self
            .log
            .get_session::<SessionConfiguration>(session_id)
            .await
            .map_err(persistence)?
        else {
            return Ok(());
        };
        // Older create paths persisted false even for explicitly supplied names.
        if session.configuration.title_is_manual
            || session.configuration.name != crate::session::DEFAULT_NAME
            || session.archived
            || self
                .log
                .session_manager(session_id)
                .await
                .map_err(persistence)?
                .is_some()
        {
            return Ok(());
        }
        let Some(opening) = self
            .log
            .first_message_opening(session_id)
            .await
            .map_err(|error| match error {
                maka_event_log::StoreError::PrefixTooLarge => ToolError::Failed(error.to_string()),
                other => persistence(other),
            })?
        else {
            return Ok(());
        };
        if opening.event.invocation != invocation {
            return Ok(());
        }
        let Fact::InvocationOpened {
            input:
                InvocationInput::Message {
                    content,
                    source_messages,
                    ..
                },
            configuration: Some(configuration),
            ..
        } = opening.event.fact
        else {
            return Ok(());
        };
        let Some(model) = configuration.model else {
            return Ok(());
        };
        let snapshot = self
            .plugin_catalog
            .capture(&Scope::Session(session_id.clone()))
            .typed::<SessionTitlePolicy>();
        let Some(policy) = snapshot.entries.get(NAME) else {
            return Ok(());
        };
        let _lease = policy
            .admit()
            .map_err(|error| ToolError::Failed(error.to_string()))?;
        let text = source_messages
            .first()
            .map_or(content.text.as_str(), |source| {
                source.unprepared_content.text.as_str()
            });
        let Some(mut input) = policy.value.0.prepare(text) else {
            return Ok(());
        };
        input
            .validate()
            .map_err(|error| ToolError::Failed(error.to_string()))?;
        input.max_output_tokens = Some(input.max_output_tokens.unwrap_or(2048).min(2048));
        let cancellation = self.shutdown.child_token();
        let _cancel = cancellation.clone().drop_guard();
        let operation = async {
            // A short auxiliary title uses the least reasoning the provider
            // declares, without altering the Session's main-turn preference.
            let observed = provider::observe_binding(self, session_id, &model, None)
                .await
                .map_err(|error| ToolError::Failed(error.message))?;
            let least = maka_runtime::execution::ThinkingLevel::ALL
                .into_iter()
                .find(|level| observed.thinking_levels.contains(level));
            let prepared = match least {
                Some(level) => provider::observe_binding(self, session_id, &model, Some(level))
                    .await
                    .map_err(|error| ToolError::Failed(error.message))?,
                None => observed,
            }
            .admit(&self.oauth)
            .map_err(|error| ToolError::Failed(error.message))?;
            let provider_id = prepared.provider_id.clone();
            let request = llm::request(prepared, input);
            let adapter = maka_model::adapters::resolve(
                &self
                    .plugin_catalog
                    .capture(&Scope::Session(session_id.clone())),
                request.provider.adapter_name(),
            )
            .map_err(|error| ToolError::Failed(error.to_string()))?;
            let quote = self
                .configuration
                .quote_model(provider_id, model.model.clone())
                .await
                .map_err(persistence)?;
            if !self.accepting() || !policy.is_effective() {
                return Err(ToolError::Failed(
                    "title generation retired before dispatch".into(),
                ));
            }
            llm::generate(
                self.models.clone(),
                request,
                adapter,
                cancellation.clone(),
                self.log.clone(),
                AuxiliarySource::SessionTitle {
                    invocation: invocation.clone(),
                },
                quote,
            )
            .await
        };
        tokio::pin!(operation);
        let generated = tokio::select! {
            biased;
            _ = policy.retired() => { cancellation.cancel(); operation.await },
            _ = tokio::time::sleep(Duration::from_secs(30)) => { cancellation.cancel(); operation.await },
            result = &mut operation => result,
        }?;
        if cancellation.is_cancelled()
            || !policy.is_effective()
            || generated.finish_reason != maka_runtime::model::ModelFinishReason::Stop
        {
            return Ok(());
        }
        let title = crate::session::normalize_title(&generated.text)
            .map_err(|error| ToolError::Failed(error.to_string()))?;
        // Execution/read-state changes advance the public revision too. Re-read
        // it after generation; only a manual/changed name supersedes this policy.
        let Some(current) = self
            .log
            .get_session::<SessionConfiguration>(session_id)
            .await
            .map_err(persistence)?
        else {
            return Ok(());
        };
        if current.archived
            || current.configuration.title_is_manual
            || current.configuration.name != session.configuration.name
        {
            return Ok(());
        }
        let _apply = policy
            .admit()
            .map_err(|error| ToolError::Failed(error.to_string()))?;
        let result = self
            .log
            .update_session_metadata(
                session_id,
                current.revision,
                move |configuration: &mut SessionConfiguration| {
                    if !configuration.title_is_manual {
                        configuration.name = title;
                    }
                    Ok(())
                },
            )
            .await;
        let result = match result {
            Ok(result) => result,
            Err(
                maka_event_log::StoreError::SessionNotFound
                | maka_event_log::StoreError::SessionRetired
                | maka_event_log::StoreError::SessionBusy,
            ) => return Ok(()),
            Err(error) => return Err(persistence(error)),
        };
        if matches!(result, SessionMutation::Committed(ref record) if record.revision != current.revision)
        {
            self.publish_session_change(session_id).await;
        }
        Ok(())
    }
}

fn persistence(error: impl ToString) -> ToolError {
    ToolError::Persistence(error.to_string())
}
