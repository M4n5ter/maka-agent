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
use crate::task::{Effect, ExecutionTemplate, Notification};
use maka_plugins::{
    authorization::{Capability, Id, Request as Consent, Target},
    terminal_ui::view,
};
use maka_runtime::execution::WorkspaceTarget;

mod catalog;
mod selector;
pub(super) use selector::Selector;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Route {
    Pick,
    Sessions {
        selection: Selection,
        #[serde(default)]
        catalog: maka_plugins::session::catalog::List,
        #[serde(default)]
        offset: usize,
    },
    Selected {
        target: Selector,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Selection {
    Resume,
    Run,
}

pub(super) fn selected(task: Option<String>, target: Selector) -> Value {
    serde_json::to_value(match task {
        Some(task) => super::Route {
            task: Some(task),
            target: Some(Route::Selected { target }),
            ..Default::default()
        },
        None => super::Route {
            creation: Some(create::Route::Form {
                schedule: create::Kind::Once,
                target,
            }),
            ..Default::default()
        },
    })
    .expect("task target route")
}
pub(super) async fn read(
    service: &Service,
    caller: &Caller,
    task_id: Option<String>,
    route: Route,
    locale: &str,
) -> Result<Reply, Error> {
    match route {
        Route::Sessions {
            selection,
            catalog,
            offset,
        } => catalog::read(service, caller, task_id, selection, catalog, offset, locale).await,
        Route::Pick => {
            let mut page = empty(Text::localized("Task type", "任务类型", "任務類型"), 0);
            page.body = Text::localized("Notifications need a connected notification client. New agent runs copy a model session's workspace and execution settings.", "通知需要已连接的通知客户端。新 Agent 任务使用所选模型会话的工作区与执行设置。", "通知需要已連線的通知客戶端。新 Agent 任務使用所選模型會話的工作區與執行設定。").resolve(locale).into();
            page.rows.push(Row {
                id: "local".into(),
                title: label(&Effect::Notify(Notification::Local)),
                description: String::new(),
                route: selected(task_id.clone(), Selector::Local),
            });
            for (selection, title) in [
                (
                    Selection::Resume,
                    Text::localized("Resume a session", "继续会话", "繼續對話"),
                ),
                (
                    Selection::Run,
                    Text::localized("New agent run", "新 Agent 任务", "新 Agent 任務"),
                ),
            ] {
                page.rows.push(Row {
                    id: match selection {
                        Selection::Resume => "resume",
                        Selection::Run => "run",
                    }
                    .into(),
                    title,
                    description: String::new(),
                    route: serde_json::to_value(super::Route {
                        task: task_id.clone(),
                        target: Some(Route::Sessions {
                            selection,
                            catalog: Default::default(),
                            offset: 0,
                        }),
                        ..Default::default()
                    })
                    .map_err(invalid)?,
                });
            }
            Ok(Reply::Page { page })
        }
        Route::Selected { target } => {
            let Some(effect) = target
                .resolve(service.backend.host.sessions.as_ref(), caller, locale)
                .await?
            else {
                return Ok(Reply::Conflict);
            };
            let (task, revision, _) = super::task(
                service,
                task_id.as_deref().ok_or_else(|| invalid("Select a task"))?,
            )?;
            let page = review(&task, revision, &effect, locale);
            Ok(Reply::Page { page })
        }
    }
}
pub(super) fn review(task: &Task, revision: u64, effect: &Effect, locale: &str) -> Page {
    let mut page = empty(
        Text::localized("Type and target", "类型与目标", "類型與目標"),
        revision,
    );
    page.body = format!(
        "{}\n\n{}",
        display(&task.title, 512, false),
        summary(effect, locale)
    );
    let needs_content =
        !matches!(effect, Effect::Notify(_)) && task.intent.body().trim().is_empty();
    if needs_content {
        page.body.push_str(&format!(
            "\n\n{}",
            Text::localized(
                "Add task content on the task page before choosing an execution target.",
                "请先在任务页面填写内容，再选择执行目标。",
                "請先在任務頁面填寫內容，再選擇執行目標。"
            )
            .resolve(locale)
        ));
    }
    page.actions.push(Action {
        id: "save_target".into(),
        label: Text::localized("Use this target", "使用此目标", "使用此目標"),
        enabled: matches!(task.status, Status::Active | Status::Paused) && !needs_content,
        fields: vec![],
        recovery: None,
        confirm: Some(view::Confirm {
            title: Text::localized(
                "Change this task's target?",
                "更改此任务的目标？",
                "變更此任務的目標？",
            )
            .resolve(locale)
            .into(),
            message: format!(
                "{}\n{}",
                display(&task.title, 512, false),
                Text::localized(
                    "Future runs will use the target shown on this page.",
                    "后续运行将使用此页面显示的目标。",
                    "後續執行將使用此頁面顯示的目標。"
                )
                .resolve(locale)
            ),
            destructive: false,
        }),
    });
    page
}
pub(super) fn label(effect: &Effect) -> Text {
    match effect {
        Effect::Notify(Notification::Local) => {
            Text::localized("Local notification", "本地通知", "本機通知")
        }
        Effect::Notify(Notification::Bot { .. }) => {
            Text::localized("Channel notification", "频道通知", "頻道通知")
        }
        Effect::SessionResume { .. } => Text::localized("Resume a session", "继续会话", "繼續對話"),
        Effect::AgentRun { .. } => {
            Text::localized("New agent run", "新 Agent 任务", "新 Agent 任務")
        }
    }
}
pub(super) fn summary(effect: &Effect, locale: &str) -> String {
    let heading = label(effect).resolve(locale).to_owned();
    match effect {
        Effect::Notify(Notification::Local) => format!("{heading}\n{}", Text::localized("Delivery waits for an authorized notification client.", "等待已授权的通知客户端送达。", "等待已授權的通知客戶端送達。").resolve(locale)),
        Effect::Notify(Notification::Bot {platform, chat_id}) => format!("{heading} · {}\n{}\n{}", serde_json::to_value(platform).expect("platform").as_str().unwrap(), display(chat_id, 1024, false), Text::localized("No native channel delivery adapter is available. The existing target is preserved.", "原生客户端没有频道送达适配器，已有目标保持不变。", "原生客戶端沒有頻道送達介接器，既有目標保持不變。").resolve(locale)),
        Effect::SessionResume { session_id } => format!("{heading}\n{}", display(session_id, 512, false)),
        Effect::AgentRun { execution } => format!("{heading}\n{}: {}\n{}: {} / {}\n{}: {}\n{}: {}\n{}: {} · {} · {}\n{}: {}",
            Text::localized("Workspace", "工作区", "工作區").resolve(locale), display(&execution.cwd, 4096, false),
            Text::localized("Model", "模型", "模型").resolve(locale), display(&execution.llm_connection_slug, 1024, false), display(&execution.model, 1024, false),
            Text::localized("Behavior", "行为", "行為").resolve(locale), execution.orchestration_mode.as_str(),
            Text::localized("Thinking", "思考", "思考").resolve(locale), serde_json::to_value(execution.thinking_level).expect("thinking"),
            Text::localized("Permissions", "权限", "權限").resolve(locale), serde_json::to_value(execution.sandbox_mode).expect("sandbox"), serde_json::to_value(execution.approval_policy).expect("approval"), serde_json::to_value(execution.collaboration_mode).expect("mode"),
            Text::localized("Tool limit", "工具范围", "工具範圍").resolve(locale), execution.bound_tools.as_ref().map_or_else(|| Text::localized("No additional restriction", "无额外限制", "無額外限制").resolve(locale).into(), |tools| format!("{} {}", tools.len(), Text::localized("captured tools", "项已固定工具", "項已固定工具").resolve(locale)))),
    }
}
/// Capture configuration only. Creation still needs its own background grant
/// and Host admission; permission to inspect a source Session is not execution authority.
pub(super) fn from_session(
    selection: Selection,
    session: &maka_plugins::session::View,
) -> Result<Effect, Error> {
    if session.tool_profile.is_some() {
        return Err(invalid(
            "This native tool profile is unavailable for scheduled work",
        ));
    }
    let maka_plugins::execution::Target::Model {
        model,
        thinking_level,
    } = &session.target
    else {
        return Err(invalid("Scheduled tasks require a model session"));
    };
    Ok(match selection {
        Selection::Resume => Effect::SessionResume {
            session_id: session.session_id.clone(),
        },
        Selection::Run => Effect::AgentRun {
            execution: ExecutionTemplate {
                cwd: session.workspace.host_cwd.clone(),
                project_id: match &session.workspace.target {
                    WorkspaceTarget::Project { project_id } => Some(project_id.clone()),
                    _ => None,
                },
                llm_connection_id: model.connection_id.clone(),
                llm_connection_slug: model.connection_slug.clone(),
                model: model.model.clone(),
                thinking_level: *thinking_level,
                sandbox_mode: session.sandbox_mode,
                approval_policy: session.approval_policy,
                collaboration_mode: session.collaboration_mode,
                orchestration_mode: session.behavior.clone(),
                bound_tools: session.bound_tools.clone(),
            },
        },
    })
}
pub(super) fn consent(effect: &Effect, operation_id: uuid::Uuid, locale: &str) -> Consent {
    let (target, capability) = match effect {
        Effect::Notify(_) => (Target::Profile, Capability::Notifications),
        Effect::SessionResume { session_id } => (
            Target::Session {
                session_id: session_id.clone(),
            },
            Capability::Executions,
        ),
        Effect::AgentRun { execution } => (
            Target::Workspace {
                workspace: execution.project_id.as_ref().map_or_else(
                    || WorkspaceTarget::HostPath {
                        path: execution.cwd.clone(),
                    },
                    |project_id| WorkspaceTarget::Project {
                        project_id: project_id.clone(),
                    },
                ),
                sandbox_mode: execution.sandbox_mode,
            },
            Capability::Executions,
        ),
    };
    Consent {
        operation_id,
        title: Text::localized("Scheduled tasks", "计划任务", "排程任務")
            .resolve(locale)
            .into(),
        target,
        capabilities: [capability].into(),
    }
}
pub(super) async fn grant(
    service: &Service,
    effect: &Effect,
    provided: Option<Id>,
) -> Result<Option<Id>, Error> {
    if let Some(grant) = provided {
        return Ok(Some(grant));
    }
    match service
        .backend
        .authorization(Origin::User { grant: None }, effect.clone())
        .await
    {
        Ok(authorization) => Ok(Some(authorization.grant)),
        Err(crate::Error::AuthorizationRequired) => Ok(None),
        Err(failure) => Err(error(failure)),
    }
}
