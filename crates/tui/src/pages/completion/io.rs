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

mod messages;
mod remote;
use messages::messages;
use remote::resource;

use super::model::{Binding, Context, Origin, Payload};
use maka_client::Client;
use maka_protocol::{
    plugin::{self, InputResourceProjection, InputResourceReply, InputResourceRequest},
    session::{self, workspace_context as workspace},
};
use maka_runtime::input::InlineReferenceKind;
use std::sync::Arc;
use tokio::sync::watch;

pub(super) const PAGE: usize = 32;

#[derive(Clone)]
pub(super) struct Cancellation(Arc<watch::Sender<bool>>);
impl Default for Cancellation {
    fn default() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }
}
impl Cancellation {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn cancelled(&self) -> bool {
        *self.0.borrow()
    }
    async fn wait(&self) {
        let mut receiver = self.0.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
    async fn read<T, E: std::fmt::Display>(
        &self,
        future: impl std::future::Future<Output = Result<T, E>>,
    ) -> Result<T, String> {
        tokio::select! { biased; _ = self.wait() => Err("completion-cancelled".into()), result = future => result.map_err(|error| error.to_string()) }
    }
}

#[derive(Clone)]
pub struct Request {
    pub(super) id: u64,
    pub(super) generation: u64,
    pub(super) context: Context,
    pub(super) locale: String,
    pub(super) job: Job,
    pub(super) cancel: Cancellation,
}
impl Request {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

#[derive(Clone, Debug)]
pub(super) enum Job {
    Workspace(workspace::Query),
    Capture {
        input: workspace::Capture,
        accept: bool,
    },
    Sessions(session::SessionCatalogQueryInput),
    Providers {
        cursor: Option<String>,
    },
    Resources {
        provider: InputResourceProjection,
        query: String,
        cursor: Option<String>,
    },
    Resolve {
        provider: InputResourceProjection,
        id: String,
        accept: bool,
    },
    Messages {
        session: String,
        name: String,
        anchor: Option<u64>,
        query: String,
    },
}

pub enum Output {
    Workspace(workspace::Page),
    Captured(Binding),
    Sessions(session::SessionCatalogQueryResult),
    Providers(plugin::Page<InputResourceProjection>),
    Resources {
        items: Vec<maka_plugins::input::resources::Item>,
        next_cursor: Option<String>,
    },
    Messages {
        items: Vec<Message>,
        next: Option<u64>,
        conversation: Option<Binding>,
    },
}
pub struct Message {
    pub sequence: u64,
    pub id: String,
    pub turn: String,
    pub binding: Binding,
}

pub async fn execute(client: &Client, request: &Request) -> Result<Output, String> {
    if request.cancel.cancelled() {
        return Err("completion-cancelled".into());
    }
    match &request.job {
        Job::Workspace(input) => request
            .cancel
            .read(client.query_session_workspace(input.clone()))
            .await
            .map(Output::Workspace),
        Job::Capture { input, .. } => {
            let captured = request
                .cancel
                .read(client.capture_session_workspace(input.clone()))
                .await?;
            if captured.basis != input.basis
                || captured.path != input.path
                || captured.kind != input.kind
            {
                return Err("Workspace capture identity changed".into());
            }
            Ok(Output::Captured(Binding {
                label: captured.path.clone(),
                origin: Origin::Workspace,
                inline: (captured.kind == workspace::Kind::File && captured.path.len() < 4096)
                    .then_some(InlineReferenceKind::WorkspaceFile),
                payload: Payload::Context {
                    quote: captured.quote,
                    directory: captured.directory_reference,
                },
            }))
        }
        Job::Sessions(input) => request
            .cancel
            .read(client.session_catalog(input.clone()))
            .await
            .map(Output::Sessions),
        Job::Providers { cursor } => {
            let output = request
                .cancel
                .read(client.plugin_query(plugin::Query {
                    view: plugin::View::InputResources,
                    root_id: Some(maka_runtime::scope::Scope::Session(
                        request.context.session.clone(),
                    )),
                    cursor: cursor.clone(),
                    limit: Some(PAGE),
                }))
                .await?;
            let plugin::QueryResult::InputResources(page) = output else {
                return Err("Wrong resource catalog page".into());
            };
            if page
                .next_cursor
                .as_ref()
                .is_some_and(|next| cursor.as_ref() == Some(next))
            {
                return Err("Resource catalog cursor repeated".into());
            }
            Ok(Output::Providers(page))
        }
        Job::Resources {
            provider,
            query,
            cursor,
        } => {
            let value = resource(
                client,
                request,
                provider,
                InputResourceRequest::Query {
                    query: query.clone(),
                    cursor: cursor.clone(),
                    limit: PAGE,
                    locale: request.locale.clone(),
                },
            )
            .await?;
            let InputResourceReply::Page { items, next_cursor } = value else {
                return Err("Wrong resource query response".into());
            };
            if items.len() > PAGE
                || next_cursor
                    .as_ref()
                    .is_some_and(|next| cursor.as_ref() == Some(next))
            {
                return Err("Invalid resource page".into());
            }
            Ok(Output::Resources { items, next_cursor })
        }
        Job::Resolve { provider, id, .. } => {
            let value = resource(
                client,
                request,
                provider,
                InputResourceRequest::Resolve {
                    id: id.clone(),
                    locale: request.locale.clone(),
                },
            )
            .await?;
            let InputResourceReply::Resolved {
                source,
                selector,
                label,
                quote,
            } = value
            else {
                return Err("Wrong resource resolution response".into());
            };
            if selector != *id
                || source.provider != provider.provider
                || source.package_id != provider.package_id
                || source.entry_id != provider.target.entry_id
                || source.activation != provider.target.activation
                || source.registration != provider.target.registration
                || source.session_id != request.context.session
            {
                return Err("Resource identity changed".into());
            }
            Ok(Output::Captured(Binding {
                label,
                origin: Origin::Plugin {
                    title: provider.descriptor.title.resolve(&request.locale).into(),
                    package: provider.package_id.clone(),
                },
                inline: None,
                payload: Payload::Selection {
                    provider: provider.provider.clone(),
                    selector,
                    source: Some(*source),
                    quote,
                },
            }))
        }
        Job::Messages {
            session,
            name,
            anchor,
            query,
        } => messages(client, request, session, name, *anchor, query).await,
    }
}
