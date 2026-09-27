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
use crate::plugin::terminal::{create, target};
use maka_plugins::{
    remote::{self, SessionView},
    session::catalog::{Page as SessionPage, Summary},
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

fn session() -> Session {
    serde_json::from_value(json!({
        "sessionId":"large-session", "revision":7, "name":"Large model session", "boundaryRevision":3,
        "workspace":{"target":{"kind":"project","projectId":"project-original"},"hostCwd":"/work"},
        "target":{"kind":"model","model":{"connection_id":"connection","connection_slug":"provider","model":"model"},"thinkingLevel":"high"},
        "sandboxMode":"read-only", "approvalPolicy":{"kind":"on-request"}, "collaborationMode":"agent", "behavior":"default", "boundTools":null
    })).unwrap()
}
fn summary(session: Session) -> Summary {
    Summary {
        session,
        archived: false,
        labels: vec![],
        updated_at: 1,
        last_message_at: None,
    }
}
#[test]
fn full_public_views_and_requests_accept_large_sources_through_compact_selectors() {
    for suffix in [1, 80] {
        let mut source = session();
        source.workspace.host_cwd = format!("/{}", "directory".repeat(350));
        source.bound_tools = Some(
            (0..128)
                .map(|index| format!("tool{index:03}_{}", "x".repeat(suffix)))
                .collect::<BTreeSet<_>>(),
        );
        let effect = from_session(Selection::Run, &source).unwrap();
        effect.validate().unwrap();
        let old = json!({"creation":{"kind":"form","schedule":"once","effect":effect}});
        assert!(
            maka_plugins::terminal_ui::view::Request::Read {
                route: old,
                locale: "en".into()
            }
            .validate()
            .is_err(),
            "the original embedded template violates the real public route contract"
        );
        let selector = Selector::capture(Selection::Run, &source).unwrap();
        let mut ordinary = source.clone();
        ordinary.session_id = "ordinary-session".into();
        ordinary.bound_tools = None;
        let Reply::Page { page } = super::super::catalog::page(
            SessionPage {
                revision: "catalog-1".into(),
                entries: vec![summary(source.clone()), summary(ordinary)],
                next_cursor: None,
            },
            None,
            Selection::Run,
            List::default(),
            0,
            "en",
        )
        .unwrap() else {
            panic!("page")
        };
        assert_eq!(page.rows.len(), 2);
        for row in &page.rows {
            maka_plugins::terminal_ui::view::Request::Read {
                route: row.route.clone(),
                locale: "en".into(),
            }
            .validate()
            .unwrap();
        }
        page.view("en").validate().unwrap();
        let task = crate::plan::Plan::create(
            "task".into(),
            crate::task::Create {
                title: "Existing task".into(),
                intent_body: "Work".into(),
                schedule: crate::schedule::Schedule::Once { run_at: 10000 },
                effect: Effect::Notify(Notification::Local),
                max_fires: None,
                expires_at: None,
            },
            crate::task::Creator::User,
            "UTC".into(),
            1000,
        )
        .unwrap()
        .task;
        for locale in ["en", "zh-CN", "zh-TW"] {
            let form = create::form("UTC", create::Kind::Once, &effect, locale).unwrap();
            let fields = form
                .fields
                .iter()
                .map(|field| {
                    (
                        field.id.clone(),
                        match &field.control {
                            Control::Text { value, .. } | Control::Choice { value, .. } => {
                                json!(value)
                            }
                            Control::Toggle { value } => json!(value),
                        },
                    )
                })
                .collect();
            maka_plugins::terminal_ui::view::Request::Submit {
                route: target::selected(None, selector.clone()),
                revision: form.revision.clone(),
                action: "create".into(),
                fields,
                grant: None,
                locale: locale.into(),
            }
            .validate()
            .unwrap();
            form.view(locale).validate().unwrap();
            let review = target::review(&task, 3, &effect, locale);
            review.clone().view(locale).validate().unwrap();
            maka_plugins::terminal_ui::view::Request::Submit {
                route: target::selected(Some(task.id.clone()), selector.clone()),
                revision: review.revision,
                action: "save_target".into(),
                fields: Default::default(),
                grant: None,
                locale: locale.into(),
            }
            .validate()
            .unwrap();
            maka_plugins::terminal_ui::view::Reply::Consent {
                request: target::consent(&effect, uuid::Uuid::new_v4(), locale),
            }
            .validate()
            .unwrap();
        }
    }
}

struct Sessions(Mutex<(String, Vec<Session>)>);
impl Queries for Sessions {
    fn list(
        &self,
        _: maka_plugins::call::Scope,
        input: List,
    ) -> BoxFuture<'_, Result<SessionPage, maka_plugins::execution::CommandError>> {
        Box::pin(async move {
            let state = self.0.lock().unwrap();
            if input
                .revision
                .as_ref()
                .is_some_and(|revision| revision != &state.0)
            {
                return Err(maka_plugins::execution::CommandError::Conflict);
            }
            let index = input
                .cursor
                .map(|value| value.parse::<usize>().unwrap())
                .unwrap_or(0);
            Ok(SessionPage {
                revision: state.0.clone(),
                entries: state
                    .1
                    .get(index)
                    .cloned()
                    .map(summary)
                    .into_iter()
                    .collect(),
                next_cursor: (index + 1 < state.1.len()).then(|| (index + 1).to_string()),
            })
        })
    }
}
#[derive(Default)]
struct Views(Mutex<Vec<maka_plugins::call::Scope>>);
impl remote::Views for Views {
    fn authorize(
        &self,
        request: Consent,
    ) -> BoxFuture<'_, Result<maka_plugins::call::Owned, Error>> {
        assert_eq!(request.target, Target::Profile);
        assert_eq!(request.capabilities, [Capability::ReadSessions].into());
        let scope = maka_plugins::call::Issuer::default()
            .issue(
                maka_plugins::call::Identity::Remote {
                    request_id: uuid::Uuid::new_v4(),
                },
                CancellationToken::new(),
            )
            .unwrap();
        self.0.lock().unwrap().push(scope.clone());
        Box::pin(async move { Ok(maka_plugins::call::Owned::new(scope)) })
    }
    fn session(&self) -> BoxFuture<'_, Result<SessionView, Error>> {
        unreachable!()
    }
    fn workspace(
        &self,
        _: remote::WorkspaceViewInput,
    ) -> BoxFuture<'_, Result<SessionView, Error>> {
        unreachable!()
    }
    fn query_database(
        &self,
        _: maka_plugins::filesystem::database::Read,
    ) -> BoxFuture<
        '_,
        Result<
            Vec<maka_plugins::filesystem::database::Table>,
            maka_plugins::filesystem::database::Error,
        >,
    > {
        unreachable!()
    }
}
#[tokio::test]
async fn selectors_reread_exact_session_revision_and_release_scope_on_stale_targets() {
    let source = session();
    let selector = Selector::capture(Selection::Run, &source).unwrap();
    let mut sibling = source.clone();
    sibling.session_id = "another-session".into();
    let sessions = Sessions(Mutex::new((
        "catalog-1".into(),
        vec![sibling, source.clone()],
    )));
    let views = Arc::new(Views::default());
    let caller = Caller {
        connection_id: uuid::Uuid::new_v4(),
        client_instance_id: "selector".into(),
        document_id: uuid::Uuid::new_v4(),
        session_id: None,
        access: remote::Access::Granted,
        views: views.clone(),
        resources: Arc::default(),
        cancellation: CancellationToken::new(),
    };
    assert_eq!(
        selector.resolve(&sessions, &caller, "en").await.unwrap(),
        Some(from_session(Selection::Run, &source).unwrap())
    );
    sessions.0.lock().unwrap().0 = "catalog-unrelated-change".into();
    assert!(
        selector
            .resolve(&sessions, &caller, "en")
            .await
            .unwrap()
            .is_some(),
        "another Session does not change this selected revision"
    );
    for field in ["workspace", "policy", "tools", "model"] {
        let mut changed = source.clone();
        changed.revision += 1;
        match field {
            "workspace" => changed.workspace.host_cwd = "/replacement".into(),
            "policy" => changed.sandbox_mode = maka_runtime::execution::SandboxMode::WorkspaceWrite,
            "tools" => changed.bound_tools = Some(["Read".into()].into()),
            _ => {
                if let maka_plugins::execution::Target::Model { model, .. } = &mut changed.target {
                    model.model = "replacement".into();
                }
            }
        }
        sessions.0.lock().unwrap().1[1] = changed;
        assert!(
            selector
                .resolve(&sessions, &caller, "en")
                .await
                .unwrap()
                .is_none(),
            "{field} must not rebind the original intent"
        );
    }
    sessions.0.lock().unwrap().1.pop();
    assert!(
        selector
            .resolve(&sessions, &caller, "en")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        views
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|scope| scope.cancellation.is_cancelled()),
        "all borrowed read scopes settled"
    );
    assert_eq!(
        Selector::Local
            .resolve(&sessions, &caller, "en")
            .await
            .unwrap(),
        Some(Effect::Notify(Notification::Local))
    );
}
