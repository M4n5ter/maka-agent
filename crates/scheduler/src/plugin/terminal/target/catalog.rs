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
pub(super) async fn read(
    service: &Service,
    caller: &Caller,
    task: Option<String>,
    selection: Selection,
    catalog: maka_plugins::session::catalog::List,
    offset: usize,
    locale: &str,
) -> Result<Reply, Error> {
    catalog.validate().map_err(invalid)?;
    let authorized = caller
        .views
        .authorize(Consent {
            operation_id: uuid::Uuid::new_v4(),
            title: Text::localized(
                "Select a scheduled task session",
                "选择计划任务的会话",
                "選擇排程任務的對話",
            )
            .resolve(locale)
            .into(),
            target: Target::Profile,
            capabilities: [Capability::ReadSessions].into(),
        })
        .await?;
    let result = service
        .backend
        .host
        .sessions
        .list(authorized.scope(), catalog.clone())
        .await;
    authorized
        .finish()
        .await
        .map_err(|_| Error::CleanupUnconfirmed)?;
    let source = result.map_err(|error| Error::Provider(error.to_string()))?;
    page(source, task, selection, catalog, offset, locale)
}

pub(in crate::plugin::terminal) fn page(
    source: maka_plugins::session::catalog::Page,
    task: Option<String>,
    selection: Selection,
    catalog: maka_plugins::session::catalog::List,
    offset: usize,
    locale: &str,
) -> Result<Reply, Error> {
    if offset > source.entries.len() {
        return Err(invalid("Session page changed"));
    }
    let mut page = empty(
        Text::localized("Choose a model session", "选择模型会话", "選擇模型對話"),
        0,
    );
    page.body = Text::localized("Only model sessions can be scheduled. Select one to capture its exact target and workspace.", "仅模型会话支持计划执行。选择会话后将固定其目标与工作区。", "僅模型對話支援排程執行。選擇對話後將固定其目標與工作區。").resolve(locale).into();
    let mut consumed = 0;
    for item in source.entries.iter().skip(offset).take(WINDOW) {
        consumed += 1;
        if item.session.tool_profile.is_some() {
            page.body.push_str(&format!(
                "\n{} — {}",
                display(&item.session.name, 512, false),
                Text::localized(
                    "native tool profile unavailable",
                    "原生工具配置暂不可用",
                    "原生工具設定暫不可用"
                )
                .resolve(locale)
            ));
            continue;
        }
        let Ok(target) = Selector::capture(selection, &item.session) else {
            continue;
        };
        page.rows.push(Row {
            id: item.session.session_id.clone(),
            title: Text::plain(display(&item.session.name, 512, false)),
            description: display(
                &format!(
                    "{} · {}",
                    item.session.session_id, item.session.workspace.host_cwd
                ),
                1024,
                false,
            ),
            route: selected(task.clone(), target),
        });
    }
    let next = if offset + consumed < source.entries.len() {
        Some((
            maka_plugins::session::catalog::List {
                revision: Some(source.revision.clone()),
                ..catalog
            },
            offset + consumed,
        ))
    } else {
        source.next_cursor.map(|cursor| {
            (
                maka_plugins::session::catalog::List {
                    revision: Some(source.revision),
                    cursor: Some(cursor),
                    include_archived: false,
                },
                0,
            )
        })
    };
    if let Some((catalog, offset)) = next {
        page.rows.push(Row {
            id: "next".into(),
            title: Text::localized("Next", "下一页", "下一頁"),
            description: String::new(),
            route: serde_json::to_value(super::super::Route {
                task,
                target: Some(Route::Sessions {
                    selection,
                    catalog,
                    offset,
                }),
                ..Default::default()
            })
            .map_err(invalid)?,
        });
    }
    if page.rows.is_empty() {
        page.body.push_str(&format!(
            "\n{}",
            Text::localized(
                "No available model sessions on this page. Create a standard model session to continue.",
                "此页没有可用的模型会话。请先创建普通模型会话。",
                "此頁沒有可用的模型對話。請先建立一般模型對話。"
            )
            .resolve(locale)
        ));
    }
    Ok(Reply::Page { page })
}
