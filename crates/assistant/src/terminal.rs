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
//! Assistant preference forms are activation-owned plugin contributions.
use futures_util::future::BoxFuture;
use maka_plugins::{
    contributions::Staged,
    preferences::{Mutation, Update, Updated},
    remote::{Error, key},
    terminal_ui::{
        self, Context, Descriptor, Placement, Text,
        app::{self, App, Cx, Submission},
        view::{Reply, Role, Tone, View, build::*},
    },
};
use serde_json::{Value, json};

pub(super) fn publish(package: &str, staged: &mut Staged) -> Result<(), String> {
    let endpoint = app::endpoint(
        Preferences,
        Descriptor::new(
            Text::localized("Assistant preferences", "助手偏好", "助理偏好"),
            Context::Application,
        )
        .placement(Placement::Settings)
        .icon("✧", "A")
        .order(10),
    )
    .map_err(|e| e.to_string())?;
    staged
        .insert(
            key(package, "preferences").map_err(|e| e.to_string())?,
            endpoint,
        )
        .map_err(|e| e.to_string())
}
struct Preferences;
impl App for Preferences {
    fn read(&self, _: Value, cx: Cx) -> BoxFuture<'static, Result<View, Error>> {
        Box::pin(async move {
            let snapshot = cx.caller.views.preferences().await?;
            let mut save = action(
                "save",
                cx.t("Save personalization", "保存个性偏好", "儲存個人偏好"),
            );
            save.fields = vec!["name".into(), "tone".into()];
            let mut instructions = action(
                "instructions",
                cx.t(
                    "Save workspace instructions",
                    "保存工作区指令设置",
                    "儲存工作區指令設定",
                ),
            );
            instructions.fields = vec!["instructions".into()];
            Ok(View {
                version: terminal_ui::VERSION,
                title: cx.t("Assistant preferences", "助手偏好", "助理偏好"),
                revision: snapshot.revision.to_string(),
                fields: vec![
                    line("name", snapshot.personalization.display_name, 256),
                    area("tone", snapshot.personalization.assistant_tone, 4096),
                    toggle("instructions", snapshot.workspace_instructions),
                ],
                actions: vec![save, instructions],
                root: column("preferences", vec![
                    input("name", "name", cx.t("Your name", "你的称呼", "你的稱呼")),
                    input("tone", "tone", cx.t("Assistant tone", "助手语气", "助理語氣")),
                    button("save", "save", Role::Primary),
                    text("note", cx.t(
                        "Workspace instructions include AGENTS.md and other project guidance.",
                        "工作区指令包括 AGENTS.md 等项目指导。",
                        "工作區指令包括 AGENTS.md 等專案指引。",
                    ), Tone::Muted),
                    input("instructions", "instructions", cx.t("Read workspace instructions", "读取工作区指令", "讀取工作區指令")),
                    button("instructions-save", "instructions", Role::Normal),
                ]),
            })
        })
    }
    fn submit(&self, submission: Submission, cx: Cx) -> BoxFuture<'static, Result<Reply, Error>> {
        Box::pin(async move {
            if submission.grant.is_some() {
                return Err(Error::Invalid("Unexpected authorization".into()));
            }
            let expected_revision = submission
                .revision
                .parse()
                .map_err(|_| Error::Invalid("Invalid preference revision".into()))?;
            let mutation = match submission.action.as_str() {
                "save" if submission.fields.len() == 2 => Mutation::Personalization {
                    value: maka_runtime::configuration::policy::Personalization {
                        display_name: submission.text("name")?.to_owned(),
                        assistant_tone: submission.text("tone")?.to_owned(),
                    },
                },
                "instructions" if submission.fields.len() == 1 => Mutation::WorkspaceInstructions {
                    enabled: submission.toggle("instructions")?,
                },
                _ => return Err(Error::Invalid("Unknown preference action".into())),
            };
            match cx
                .caller
                .controls
                .update_preferences(Update {
                    expected_revision,
                    mutation,
                })
                .await?
            {
                Updated::Committed { .. } => Ok(Reply::Applied { route: json!(null) }),
                Updated::RevisionConflict { .. } => Ok(Reply::Conflict),
            }
        })
    }
}
