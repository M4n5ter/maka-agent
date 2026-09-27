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
use maka_plugins::{
    authorization::{Capability, Request, Target as AccessTarget},
    session::catalog,
};

pub(in crate::recall::terminal) async fn sessions(
    recall: &Recall,
    route: &Value,
    filters: &Filters,
    cx: Cx,
) -> Result<View, Error> {
    recall
        .check_privacy()
        .await
        .map_err(|error| Error::Provider(error.to_string()))?;
    let owned = cx
        .caller
        .views
        .authorize(Request {
            operation_id: cx.caller.document_id,
            title: "Choose a conversation to search".into(),
            target: cx
                .caller
                .session_id
                .as_ref()
                .map(|id| AccessTarget::Session {
                    session_id: id.clone(),
                })
                .unwrap_or(AccessTarget::Profile),
            capabilities: [Capability::ReadHistory].into(),
        })
        .await?;
    let result = recall
        .history
        .list(
            owned.scope(),
            catalog::List {
                revision: route["catalogRevision"].as_str().map(str::to_owned),
                cursor: route["catalogCursor"].as_str().map(str::to_owned),
                include_archived: true,
            },
        )
        .await;
    owned
        .finish()
        .await
        .map_err(|_| Error::CleanupUnconfirmed)?;
    recall
        .check_privacy()
        .await
        .map_err(|error| Error::Provider(error.to_string()))?;
    let mut base = serde_json::to_value(filters).expect("recall filters");
    let mut all = base.clone();
    all["session"] = Value::Null;
    let mut children = vec![
        link(
            "back",
            cx.t("Back to search", "返回搜索", "返回搜尋"),
            base.clone(),
        )
        .into(),
    ];
    if cx.caller.session_id.is_none() {
        children.push(
            link(
                "all",
                cx.t("All conversations", "全部对话", "全部對話"),
                all,
            )
            .into(),
        );
    }
    match result {
        Err(error) => {
            children.push(text(
                "error",
                view::build::clean(&error.to_string(), false),
                Tone::Warning,
            ));
            base["pickSession"] = json!(true);
            children.push(
                link(
                    "restart",
                    cx.t(
                        "Refresh conversation list",
                        "刷新对话列表",
                        "重新整理對話清單",
                    ),
                    base,
                )
                .into(),
            );
        }
        Ok(page) => {
            let offset = route["catalogOffset"]
                .as_u64()
                .and_then(|offset| usize::try_from(offset).ok())
                .unwrap_or(0)
                .min(page.entries.len());
            for entry in page.entries.iter().skip(offset).take(20) {
                use sha2::{Digest, Sha256};
                let mut selected = base.clone();
                selected["session"] = json!({"id":entry.session.session_id,"title":entry.session.name.chars().take(128).collect::<String>()});
                children.push(
                    link(
                        format!(
                            "session-{:x}",
                            Sha256::digest(entry.session.session_id.as_bytes())
                        ),
                        view::build::clean(&entry.session.name, false)
                            .chars()
                            .take(128)
                            .collect::<String>(),
                        selected,
                    )
                    .detail(if entry.archived {
                        cx.t("Archived", "已归档", "已封存")
                    } else {
                        String::new()
                    })
                    .into(),
                );
            }
            if offset + 20 < page.entries.len() || page.next_cursor.is_some() {
                base["pickSession"] = json!(true);
                base["catalogRevision"] = json!(page.revision);
                if offset + 20 < page.entries.len() {
                    base["catalogCursor"] = route["catalogCursor"].clone();
                    base["catalogOffset"] = json!(offset + 20);
                } else {
                    base["catalogCursor"] = json!(page.next_cursor);
                    base["catalogOffset"] = json!(0);
                }
                children.push(
                    link(
                        "next",
                        cx.t("More conversations", "更多对话", "更多對話"),
                        base,
                    )
                    .into(),
                );
            }
        }
    }
    Ok(View {
        version: VERSION,
        title: cx.t("Choose conversation", "选择对话", "選擇對話"),
        revision: "recall-sessions".into(),
        fields: vec![],
        actions: vec![],
        root: column("root", children),
    })
}
