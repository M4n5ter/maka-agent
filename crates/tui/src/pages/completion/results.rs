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

use super::*;
use super::{
    io::Job,
    model::{Candidate, Cursor, Payload, Pick},
};
use maka_protocol::session::{SessionCatalogQueryResult, workspace_context as workspace};

impl App {
    pub fn completion_completed(&mut self, request: Request, result: Result<Output, String>) {
        if self
            .completion
            .pending
            .as_ref()
            .is_none_or(|pending| pending.id != request.id)
        {
            return;
        }
        self.completion.pending = None;
        let Some(popup) = &self.completion.popup else {
            return;
        };
        if popup.generation != request.generation
            || !self.completion_current(&request.context, popup.explicit)
            || request.cancel.cancelled()
        {
            return;
        }
        let output = match result {
            Ok(output) => output,
            Err(error) => {
                let text = format!(
                    "{}: {}",
                    self.i18n.text("completion-failed"),
                    crate::view::safe(&error)
                );
                self.completion.popup.as_mut().unwrap().error = Some(text);
                return;
            }
        };
        let query = popup.query.text().to_owned();
        let mut rows = vec![];

        let next = match output {
            Output::Captured(binding) => {
                self.completion_captured(request, binding);
                return;
            }
            Output::Workspace(page) => {
                let Job::Workspace(input) = &request.job else {
                    return;
                };
                if page.basis.root_id != request.context.root
                    || page.basis.session_id != request.context.session
                    || page.directory != input.directory
                    || page.filter != input.filter
                {
                    self.completion_error("completion-stale");
                    return;
                }
                for entry in page.entries {
                    let kind = if entry.kind == workspace::Kind::Directory {
                        "completion-directory"
                    } else {
                        "completion-file"
                    };
                    rows.push(Candidate {
                        id: format!("workspace:{:?}:{}", entry.kind, entry.path),
                        title: entry.path.clone(),
                        detail: self.i18n.text(kind),
                        source: self.i18n.text("completion-workspace"),
                        enabled: true,
                        pick: Pick::Workspace(workspace::Capture {
                            basis: page.basis.clone(),
                            path: entry.path,
                            kind: entry.kind,
                        }),
                    });
                }
                page.next_cursor.map(Cursor::Workspace)
            }
            Output::Skills(result) => {
                let maka_skills::api::InvocableResult::Page {
                    revision,
                    items,
                    next_cursor,
                } = result
                else {
                    self.completion_error("completion-stale");
                    return;
                };
                for item in items.into_iter().filter(|item| {
                    super::candidates::matches(
                        &query,
                        [&item.name, &item.description, &item.reference],
                    )
                }) {
                    rows.push(Candidate {
                        id: format!("skill:{}", item.id),
                        title: format!("/{}", item.name),
                        detail: item.description,
                        source: self.i18n.text("skills-title"),
                        enabled: true,
                        pick: Pick::Skill {
                            id: item.id,
                            name: item.name,
                        },
                    });
                }
                next_cursor.map(|cursor| Cursor::Skills { revision, cursor })
            }
            Output::Sessions(result) => {
                let SessionCatalogQueryResult::Page {
                    revision,
                    sessions,
                    next_cursor,
                } = result
                else {
                    self.completion_error("completion-stale");
                    return;
                };
                for session in sessions.into_iter().filter(|session| {
                    super::candidates::matches(&query, [&session.name, &session.workspace.host_cwd])
                }) {
                    rows.push(Candidate {
                        id: format!("session:{}", session.id),
                        title: session.name.clone(),
                        detail: session.workspace.host_cwd,
                        source: self.i18n.text("completion-sessions"),
                        enabled: true,
                        pick: Pick::Session {
                            id: session.id,
                            name: session.name,
                        },
                    });
                }
                next_cursor.map(|cursor| Cursor::Catalog { revision, cursor })
            }
            Output::Providers(page) => {
                if let Some(reselect) = &self.completion.reselect {
                    if let Some(provider) = page
                        .items
                        .iter()
                        .find(|provider| provider.provider == reselect.provider)
                    {
                        self.completion.resolve = Some(Job::Resolve {
                            provider: provider.clone(),
                            id: reselect.selector.clone(),
                            accept: false,
                        });
                    } else if let Some(cursor) = page.next_cursor {
                        let popup = self.completion.popup.as_mut().unwrap();
                        popup.scan = Some(Cursor::Providers(cursor));
                        popup.requested = true;
                    } else {
                        self.completion_error("completion-source-unavailable");
                    }
                    return;
                }
                for provider in page.items.into_iter().filter(|provider| {
                    super::candidates::matches(
                        &query,
                        [
                            provider.descriptor.title.resolve(&request.locale),
                            provider.package_id.as_str(),
                        ],
                    )
                }) {
                    rows.push(Candidate {
                        id: format!(
                            "provider:{}:{}",
                            provider.provider, provider.target.registration
                        ),
                        title: provider.descriptor.title.resolve(&request.locale).into(),
                        detail: provider.package_id.clone(),
                        source: self.i18n.text("completion-plugins"),
                        enabled: true,
                        pick: Pick::Provider(provider),
                    });
                }
                page.next_cursor.map(Cursor::Providers)
            }
            Output::Resources { items, next_cursor } => {
                let Job::Resources { provider, .. } = &request.job else {
                    return;
                };
                for item in items {
                    rows.push(Candidate {
                        id: format!(
                            "resource:{}:{}:{}",
                            provider.provider, provider.target.registration, item.id
                        ),
                        title: item.title,
                        detail: item.description.unwrap_or_default(),
                        source: provider.descriptor.title.resolve(&request.locale).into(),
                        enabled: true,
                        pick: Pick::Resource {
                            provider: provider.clone(),
                            id: item.id,
                        },
                    });
                }
                next_cursor.map(Cursor::Resource)
            }
            Output::Messages {
                items,
                next: more,
                conversation,
            } => {
                if query.is_empty()
                    && let Some(conversation) = conversation
                {
                    rows.push(Candidate {
                        id: "conversation".into(),
                        title: self.i18n.text("completion-capture-conversation"),
                        detail: self.i18n.text("completion-captured-excerpt"),
                        source: view::origin(self, &conversation.origin),
                        enabled: true,
                        pick: Pick::Captured(conversation),
                    });
                }
                for item in items {
                    rows.push(Candidate {
                        id: format!("message:{}:{}:{}", item.sequence, item.turn, item.id),
                        title: item.binding.label.clone(),
                        detail: match &item.binding.payload {
                            Payload::Context { quote, .. } => {
                                quote.text.chars().take(160).collect()
                            }
                            _ => String::new(),
                        },
                        source: view::origin(self, &item.binding.origin),
                        enabled: true,
                        pick: Pick::Captured(item.binding),
                    });
                }
                more.map(Cursor::Messages)
            }
        };
        // Search scans empty transport pages, not an arbitrary inventory quota.
        if rows.is_empty() && next.is_some() {
            let popup = self.completion.popup.as_mut().unwrap();
            popup.scan = next;
            popup.requested = true;
            return;
        }
        if matches!(request.job, Job::Skills { .. })
            && matches!(
                self.completion.popup.as_ref().map(|popup| &popup.source),
                Some(Source::Commands)
            )
        {
            let mut local = std::mem::take(&mut self.completion.popup.as_mut().unwrap().candidates);
            local.extend(rows);
            rows = local;
        }
        self.completion_rows(rows, next);
    }
}
