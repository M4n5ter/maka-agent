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

//! Reusable executor forms are plugin contributions, sharing their owner's lifecycle.
use super::Executor;
use crate::{
    Error as PluginError,
    contributions::{Publisher, Staged},
    fiber::Context as Owner,
    remote::{self, Error, executor_session::Create},
    storage::{Data, Mutation, Store},
    terminal_ui::{
        self, Context, Descriptor, Text,
        app::{self, App, Cx, Submission},
        view::{Reply, Role, Target, Tone, View, build::*},
    },
};
use futures_util::future::BoxFuture;
use maka_runtime::{
    execution::{ThinkingLevel, WorkspaceTarget},
    executor::{ExecutorId, Settings},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{any::TypeId, sync::Arc};

fn method(id: &str, creating: bool) -> String {
    format!(
        "executor-{}-{:x}",
        if creating { "create" } else { "configure" },
        Sha256::digest(id.as_bytes())
    )
}
/// A single registration owns the execution provider and its standard forms.
/// A provider may publish custom terminal views instead of using this helper.
pub fn stage(
    staged: &mut Staged,
    owner: &Owner,
    executor: Executor,
    storage: Option<Arc<dyn Store>>,
) -> Result<(), PluginError> {
    let identity = owner.identity()?;
    let id = executor.id.clone();
    let title = &executor.display_name;
    if identity.scope == crate::composition::Scope::Profile {
        staged.insert(
            remote::key(&identity.package_id, &method(id.as_str(), true))?,
            app::endpoint(
                Form {
                    id: id.clone(),
                    title: title.clone(),
                    creating: true,
                    thinking: executor.capabilities.thinking,
                    storage: storage.clone(),
                },
                Descriptor::new(
                    Text::localized(
                        &format!("New {title} session"),
                        &format!("新建 {title} 会话"),
                        &format!("新增 {title} 對話"),
                    ),
                    Context::Application,
                )
                .icon("+", "+")
                .launch(),
            )
            .map_err(|e| PluginError::Invalid(e.to_string()))?
            .requiring_host_paths(),
        )?;
    }
    staged.insert(
        remote::key(&identity.package_id, &method(id.as_str(), false))?,
        app::endpoint(
            Form {
                id: id.clone(),
                title: title.clone(),
                creating: false,
                thinking: executor.capabilities.thinking,
                storage,
            },
            Descriptor::new(
                Text::localized(
                    &format!("Use {title}"),
                    &format!("使用 {title}"),
                    &format!("使用 {title}"),
                ),
                Context::Session,
            )
            .icon("⇆", "X"),
        )
        .map_err(|e| PluginError::Invalid(e.to_string()))?,
    )?;
    staged.insert(id.as_str(), executor)
}
/// Withdraw execution and both views atomically, leaving sibling executors live.
pub fn withdraw(publisher: &Publisher, owner: &Owner, names: &[String]) -> Result<(), PluginError> {
    let package = owner.identity()?.package_id;
    let mut keys = vec![];
    for name in names {
        keys.push((TypeId::of::<Executor>(), name.clone()));
        for creating in [true, false] {
            keys.push((
                TypeId::of::<remote::Endpoint>(),
                remote::key(&package, &method(name, creating))?,
            ));
        }
    }
    publisher.withdraw_group(&keys)
}
#[derive(Clone)]
struct Form {
    id: ExecutorId,
    title: String,
    creating: bool,
    thinking: bool,
    storage: Option<Arc<dyn Store>>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    workspace: Option<WorkspaceTarget>,
    operation: Option<uuid::Uuid>,
    session: Option<String>,
}
fn route(value: Value) -> Result<Route, Error> {
    if value.is_null() {
        Ok(Route::default())
    } else {
        serde_json::from_value(value).map_err(|e| Error::Invalid(e.to_string()))
    }
}
fn failure(error: impl std::fmt::Display) -> Error {
    Error::Provider(error.to_string())
}
impl Form {
    fn input(&self, submission: &Submission) -> Result<Create, Error> {
        let route = route(submission.route.clone())?;
        let workspace = match route.workspace {
            Some(workspace @ WorkspaceTarget::Project { .. }) => workspace,
            _ => WorkspaceTarget::HostPath {
                path: submission.text("path")?.trim().to_owned(),
            },
        };
        let settings = Settings {
            model: (!submission.text("model")?.trim().is_empty())
                .then(|| submission.text("model").unwrap().trim().to_owned()),
            thinking_level: if self.thinking && submission.text("thinking")? != "default" {
                Some(
                    serde_json::from_value(json!(submission.text("thinking")?))
                        .map_err(|e| Error::Invalid(e.to_string()))?,
                )
            } else {
                None
            },
        };
        Ok(Create {
            session_id: submission.revision.clone(),
            executor_id: self.id.clone(),
            settings,
            workspace,
            name: (!submission.text("name")?.trim().is_empty())
                .then(|| submission.text("name").unwrap().trim().to_owned()),
        })
    }
    fn key(&self, operation: uuid::Uuid) -> String {
        format!("executor-ui/{}/{operation}", self.id.as_str())
    }
    async fn remember(&self, input: &Create) -> Result<(), Error> {
        let operation =
            uuid::Uuid::parse_str(&input.session_id).map_err(|e| Error::Invalid(e.to_string()))?;
        let store = self
            .storage
            .as_ref()
            .ok_or_else(|| Error::Invalid("Plugin storage is unavailable".into()))?;
        let key = self.key(operation);
        let data = serde_json::to_value(input).map_err(failure)?;
        if let Some(existing) = store.read(key.clone()).await.map_err(failure)? {
            return if existing.data.value() == Some(&data) {
                Ok(())
            } else {
                Err(Error::Invalid(
                    "Creation identity already has different input".into(),
                ))
            };
        }
        store
            .batch(vec![Mutation {
                key,
                expected_revision: None,
                data: Data::Present(data),
            }])
            .await
            .map_err(|e| Error::OutcomeUnknown(e.to_string()))?;
        Ok(())
    }
}
impl App for Form {
    fn read(&self, value: Value, cx: Cx) -> BoxFuture<'static, Result<View, Error>> {
        let this = self.clone();
        Box::pin(async move {
            let route = route(value)?;
            if let Some(session) = route.session {
                let item = crate::terminal_ui::view::Node::Item {
                    key: "open".into(),
                    title: cx.t("Open session", "打开会话", "開啟對話"),
                    detail: String::new(),
                    meta: String::new(),
                    current: false,
                    tone: Tone::Normal,
                    target: Target::Session { session },
                };
                return Ok(View {
                    version: terminal_ui::VERSION,
                    title: this.title,
                    revision: "created".into(),
                    fields: vec![],
                    actions: vec![],
                    root: item,
                });
            }
            let current = if this.creating {
                None
            } else {
                cx.caller
                    .views
                    .executor_session(cx.session()?.into())
                    .await?
            };
            if !this.creating && current.is_none() {
                return Err(Error::Invalid("Session is unavailable".into()));
            }
            let revision = current.as_ref().map_or_else(
                || uuid::Uuid::new_v4().to_string(),
                |session| session.revision.to_string(),
            );
            let settings = current
                .as_ref()
                .filter(|session| session.executor_id.as_ref() == Some(&this.id))
                .map(|session| session.settings.clone())
                .unwrap_or_default();
            let path = match &route.workspace {
                Some(WorkspaceTarget::HostPath { path }) => path.clone(),
                _ => String::new(),
            };
            let mut fields = vec![line("model", settings.model.unwrap_or_default(), 512)];
            if this.creating {
                fields.push(line("name", "", 1024));
                if !matches!(route.workspace, Some(WorkspaceTarget::Project { .. })) {
                    fields.push(line("path", path, terminal_ui::view::MAX_TEXT));
                }
            }
            let mut body = vec![];
            if this.creating {
                body.push(input(
                    "name",
                    "name",
                    cx.t("Session name", "会话名称", "對話名稱"),
                ));
                if let Some(WorkspaceTarget::Project { project_id }) = &route.workspace {
                    body.push(text("project", project_id, Tone::Muted));
                } else {
                    body.push(input(
                        "path",
                        "path",
                        cx.t("Workspace path", "工作区路径", "工作區路徑"),
                    ));
                }
            }
            body.push(input(
                "model",
                "model",
                cx.t(
                    "Executor model (optional)",
                    "执行器模型（可选）",
                    "執行器模型（選填）",
                ),
            ));
            if this.thinking {
                let value = settings
                    .thinking_level
                    .map(|level| {
                        serde_json::to_value(level)
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned()
                    })
                    .unwrap_or_else(|| "default".into());
                let mut options = vec![(
                    "default".into(),
                    cx.t("Executor default", "执行器默认", "執行器預設"),
                )];
                options.extend(ThinkingLevel::ALL.into_iter().map(|level| {
                    let value = serde_json::to_value(level)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned();
                    (value.clone(), value)
                }));
                fields.push(choice("thinking", value, options));
                body.push(input(
                    "thinking",
                    "thinking",
                    cx.t("Thinking level", "思考强度", "思考強度"),
                ));
            }
            let mut save = action(
                "save",
                if this.creating {
                    cx.t("Create session", "新建会话", "新增對話")
                } else {
                    cx.t("Use executor", "使用执行器", "使用執行器")
                },
            );
            save.fields = fields.iter().map(|field| field.id.clone()).collect();
            if this.creating {
                save.recovery = Some(json!({"operation":revision}));
            }
            body.push(button("save", "save", Role::Primary));
            Ok(View {
                version: terminal_ui::VERSION,
                title: this.title,
                revision,
                fields,
                actions: vec![save],
                root: column("executor", body),
            })
        })
    }
    fn submit(&self, submission: Submission, cx: Cx) -> BoxFuture<'static, Result<Reply, Error>> {
        let this = self.clone();
        Box::pin(async move {
            if submission.action != "save" || submission.grant.is_some() {
                return Err(Error::Invalid("Invalid executor action".into()));
            }
            if this.creating {
                let input = this.input(&submission)?;
                this.remember(&input).await?;
                let session = cx.caller.controls.create_executor_session(input).await?;
                Ok(Reply::Applied {
                    route: json!({"session":session.session_id}),
                })
            } else {
                let model = submission.text("model")?.trim().to_owned();
                let thinking_level = if this.thinking && submission.text("thinking")? != "default" {
                    Some(
                        serde_json::from_value(json!(submission.text("thinking")?))
                            .map_err(failure)?,
                    )
                } else {
                    None
                };
                let settings = Settings {
                    model: (!model.is_empty()).then_some(model),
                    thinking_level,
                };
                let expected_revision = submission
                    .revision
                    .parse()
                    .map_err(|_| Error::Invalid("Invalid Session revision".into()))?;
                match cx
                    .caller
                    .controls
                    .configure_executor_session(remote::executor_session::Configure {
                        expected_revision,
                        executor_id: this.id,
                        settings,
                    })
                    .await?
                {
                    crate::remote::executor_session::Configured::Committed { .. } => {
                        Ok(Reply::Applied { route: Value::Null })
                    }
                    crate::remote::executor_session::Configured::RevisionConflict { .. } => {
                        Ok(Reply::Conflict)
                    }
                }
            }
        })
    }
    fn recover(&self, value: Value, cx: Cx) -> BoxFuture<'static, Result<Reply, Error>> {
        let this = self.clone();
        Box::pin(async move {
            let operation = route(value)?
                .operation
                .ok_or_else(|| Error::Invalid("Missing creation identity".into()))?;
            let store = this
                .storage
                .as_ref()
                .ok_or_else(|| Error::Invalid("Plugin storage is unavailable".into()))?;
            let Some(record) = store.read(this.key(operation)).await.map_err(failure)? else {
                return Ok(Reply::Unrecorded);
            };
            let input: Create = serde_json::from_value(
                record
                    .data
                    .value()
                    .cloned()
                    .ok_or_else(|| Error::Invalid("Missing creation intent".into()))?,
            )
            .map_err(failure)?;
            if input.executor_id != this.id || input.session_id != operation.to_string() {
                return Err(Error::Invalid("Invalid saved creation intent".into()));
            }
            match cx.caller.views.executor_creation(input).await? {
                Some(session) => Ok(Reply::Applied {
                    route: json!({"session":session.session_id}),
                }),
                None => Ok(Reply::Unrecorded),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{composition::Scope, contributions::Catalog, executor, fiber::Fiber};
    struct Unused;
    impl executor::Provider for Unused {
        fn execute(
            &self,
            _: executor::Request,
            _: executor::Context,
        ) -> BoxFuture<'static, Result<executor::Outcome, executor::Error>> {
            Box::pin(async { panic!("directory operations do not run executors") })
        }
    }
    #[tokio::test]
    async fn executor_withdrawal_retires_both_views_and_preserves_siblings() {
        let catalog = Catalog::default();
        let fiber = Fiber::new("example.workflow", "workflow", Scope::Profile).unwrap();
        fiber.begin_loading().unwrap();
        fiber.ready().unwrap();
        fiber.publish().unwrap();
        let owner = fiber.context();
        let mut staged = Staged::default();
        for id in ["first", "second"] {
            stage(
                &mut staged,
                &owner,
                Executor {
                    id: id.to_owned().try_into().unwrap(),
                    display_name: id.into(),
                    capabilities: Default::default(),
                    provider: Arc::new(Unused),
                },
                None,
            )
            .unwrap();
        }
        let registration = catalog.register(&owner, staged).unwrap();
        let before = catalog.snapshot::<remote::Endpoint>(&Scope::Profile);
        assert_eq!(before.entries.len(), 4);
        let create = before.entries
            [&remote::key("example.workflow", &method("first", true)).unwrap()]
            .clone();
        let configure = before.entries
            [&remote::key("example.workflow", &method("first", false)).unwrap()]
            .clone();
        assert!(create.admit().is_ok());
        assert!(configure.admit().is_ok());
        withdraw(&catalog.publisher(owner.clone()), &owner, &["first".into()]).unwrap();
        assert!(create.admit().is_err());
        assert!(configure.admit().is_err());
        assert_eq!(
            catalog
                .snapshot::<Executor>(&Scope::Profile)
                .entries
                .keys()
                .collect::<Vec<_>>(),
            [&"second".to_owned()]
        );
        let after = catalog.snapshot::<remote::Endpoint>(&Scope::Profile);
        assert_eq!(
            after.revision,
            before.revision + 1,
            "one atomic directory change"
        );
        assert_eq!(after.entries.len(), 2);
        assert!(after.entries.values().all(|entry| entry.admit().is_ok()));
        drop(registration);
        assert!(
            catalog
                .snapshot::<Executor>(&Scope::Profile)
                .entries
                .is_empty()
        );
        assert!(
            catalog
                .snapshot::<remote::Endpoint>(&Scope::Profile)
                .entries
                .is_empty()
        );
        assert!(after.entries.values().all(|entry| entry.admit().is_err()));
    }
}
