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

use super::remote::Service;
use crate::recap::{Recaps, Receipt};
use futures_util::future::BoxFuture;
use maka_plugins::{
    contributions::Staged,
    remote::{Error, Method, key},
    terminal_ui::{
        self, Context, Descriptor, Text,
        app::{self, App, Cx, Submission},
        view::{self, Reply, Role, Tone, View, build::*},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    operation: Option<Uuid>,
}

pub(super) fn publish(
    recaps: Arc<Recaps>,
    package: &str,
    staged: &mut Staged,
) -> Result<(), String> {
    let title = Text::localized("Session recap", "会话回顾", "對話回顧");
    let descriptor = Descriptor::new(title.clone(), Context::Session)
        .icon("≋", "R")
        .order(20)
        .command(terminal_ui::Command {
            name: "recap".into(),
            aliases: vec![],
            title,
            description: Text::localized(
                "Read or generate a conversation recap",
                "查看或生成会话回顾",
                "查看或產生對話回顧",
            ),
            route: Value::Null,
        });
    staged
        .insert(
            key(package, "terminal").map_err(message)?,
            app::endpoint(Recap(Arc::new(Service(recaps))), descriptor).map_err(message)?,
        )
        .map_err(message)
}
#[derive(Clone)]
struct Recap(Arc<Service>);
fn route(value: Value) -> Result<Route, Error> {
    if value.is_null() {
        Ok(Route::default())
    } else {
        serde_json::from_value(value).map_err(|e| Error::Invalid(e.to_string()))
    }
}
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
impl Recap {
    async fn receipt(&self, route: &Route, cx: &Cx) -> Result<Option<Receipt>, Error> {
        let input = route.operation.map_or_else(
            || json!({"kind":"read"}),
            |operation| json!({"kind":"query","operationId":operation}),
        );
        let output = self.0.call(input, cx.caller.clone()).await?;
        serde_json::from_value(output["recap"].clone()).map_err(|e| Error::Provider(e.to_string()))
    }
}
impl App for Recap {
    fn read(&self, value: Value, cx: Cx) -> BoxFuture<'static, Result<View, Error>> {
        let this = self.clone();
        Box::pin(async move {
            let route = route(value)?;
            let receipt = this.receipt(&route, &cx).await?;
            let operation = Uuid::new_v4();
            let title = cx.t("Session recap", "会话回顾", "對話回顧");
            let mut generate = action(
                "generate",
                cx.t("Generate new recap", "生成新回顾", "產生新回顧"),
            );
            generate.recovery = Some(json!({"operation":operation}));
            generate.confirm = Some(view::Confirm {
                title: title.clone(),
                message: cx.t(
                    "Generating a recap uses a model and may incur charges.",
                    "生成回顾会调用模型，可能产生费用。",
                    "產生回顧會呼叫模型，可能產生費用。",
                ),
                destructive: false,
            });
            // A pending original is never replaced by a fresh generation.
            generate.enabled = !matches!(receipt, Some(Receipt::Pending { .. }));
            let content = match receipt {
                Some(Receipt::Ready { text, .. }) => markdown("recap", text),
                Some(Receipt::Pending { .. }) => text(
                    "pending",
                    cx.t(
                        "The original operation is unresolved. Refresh to check its receipt.",
                        "原操作结果尚未确认。刷新以查询回执。",
                        "原操作結果尚未確認。重新整理以查詢回執。",
                    ),
                    Tone::Warning,
                ),
                Some(Receipt::Failed { .. }) => text(
                    "failed",
                    cx.t(
                        "The model could not complete the recap.",
                        "模型未能完成回顾。",
                        "模型未能完成回顧。",
                    ),
                    Tone::Warning,
                ),
                None => text(
                    "empty",
                    cx.t("No saved recap.", "尚无保存的回顾。", "尚無儲存的回顧。"),
                    Tone::Muted,
                ),
            };
            Ok(View {
                version: terminal_ui::VERSION,
                title,
                revision: operation.to_string(),
                fields: vec![],
                actions: vec![generate],
                root: boundary("recap", content)
                    .bottom(button("generate", "generate", Role::Primary))
                    .into(),
            })
        })
    }
    fn submit(&self, submission: Submission, cx: Cx) -> BoxFuture<'static, Result<Reply, Error>> {
        let this = self.clone();
        Box::pin(async move {
            if submission.action != "generate"
                || !submission.fields.is_empty()
                || submission.grant.is_some()
            {
                return Err(Error::Invalid("Invalid recap action".into()));
            }
            let _ = route(submission.route)?;
            let operation =
                Uuid::parse_str(&submission.revision).map_err(|e| Error::Invalid(e.to_string()))?;
            this.0
                .call(
                    json!({"kind":"generate","operationId":operation}),
                    cx.caller,
                )
                .await?;
            Ok(Reply::Applied {
                route: json!({"operation":operation}),
            })
        })
    }
    fn recover(&self, value: Value, cx: Cx) -> BoxFuture<'static, Result<Reply, Error>> {
        let this = self.clone();
        Box::pin(async move {
            let route = route(value)?;
            if route.operation.is_none() {
                return Err(Error::Invalid("Missing recap operation".into()));
            }
            match this.receipt(&route, &cx).await? {
                Some(Receipt::Pending { .. }) | None => Ok(Reply::Unrecorded),
                Some(_) => Ok(Reply::Applied {
                    route: serde_json::to_value(route)
                        .map_err(|e| Error::Provider(e.to_string()))?,
                }),
            }
        })
    }
}
