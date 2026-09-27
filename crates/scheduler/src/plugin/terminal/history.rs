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
use crate::task::Outcome;
use maka_plugins::terminal_ui::view::{self, Node, Target, Tone};

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Route {
    List {
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        revision: Option<u64>,
    },
    Run {
        id: String,
    },
    Error,
}
pub(super) fn route(task: &str, history: Route) -> Value {
    serde_json::to_value(super::Route {
        task: Some(task.into()),
        history: Some(history),
        ..Default::default()
    })
    .expect("history route")
}
pub(super) fn outcome(outcome: Outcome) -> Text {
    match outcome {
        Outcome::Ok => Text::localized("Accepted", "已接受", "已接受"),
        Outcome::Failed => Text::localized("Failed", "失败", "失敗"),
        Outcome::Blocked => Text::localized("Blocked", "受阻", "受阻"),
    }
}
pub(super) fn read(
    task: &Task,
    revision: u64,
    route_value: Route,
    locale: &str,
) -> Result<view::Reply, Error> {
    let mut page = empty(
        Text::localized("Runs and history", "运行与历史", "執行與歷史"),
        revision,
    );
    let mut session = None;
    match route_value {
        Route::List {
            offset,
            revision: expected,
        } => {
            if expected.is_some_and(|expected| expected != revision) {
                return Ok(view::Reply::Conflict);
            }
            if offset > task.runs.len() || !offset.is_multiple_of(WINDOW) {
                return Err(invalid("Invalid run history page"));
            }
            page.body = format!(
                "{}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}",
                display(&task.title, 512, false),
                Text::localized("Total runs", "运行总数", "執行總數").resolve(locale),
                task.fire_count,
                Text::localized("Last run", "上次运行", "上次執行").resolve(locale),
                instant(task.last_fire_at),
                Text::localized("Created", "创建时间", "建立時間").resolve(locale),
                instant(Some(task.created_at)),
                Text::localized("Updated", "更新时间", "更新時間").resolve(locale),
                instant(Some(task.updated_at)),
                Text::localized("Retained history", "保留的历史记录", "保留的歷史紀錄")
                    .resolve(locale),
                task.runs.len()
            );
            for run in task.runs.iter().skip(offset).take(WINDOW) {
                page.rows.push(Row {
                    id: run.id.clone(),
                    title: Text::plain(format!(
                        "{} · {}",
                        instant(Some(run.at)),
                        outcome(run.outcome).resolve(locale)
                    )),
                    description: display(&run.message, 512, false),
                    route: route(&task.id, Route::Run { id: run.id.clone() }),
                });
            }
            if offset + WINDOW < task.runs.len() {
                page.rows.push(Row {
                    id: "next".into(),
                    title: Text::localized("Next", "下一页", "下一頁"),
                    description: String::new(),
                    route: route(
                        &task.id,
                        Route::List {
                            offset: offset + WINDOW,
                            revision: Some(revision),
                        },
                    ),
                });
            }
            if let Some(error) = &task.last_error {
                page.rows.push(Row {
                    id: "error".into(),
                    title: Text::localized("Last error", "最近错误", "最近錯誤"),
                    description: display(error, 512, false),
                    route: route(&task.id, Route::Error),
                });
            }
            if !task.runs.is_empty() || task.last_error.is_some() {
                page.actions.push(Action {
                    id: "clear".into(),
                    label: Text::localized("Clear history", "清除历史", "清除歷史"),
                    enabled: true,
                    fields: vec![],
                    recovery: None,
                    confirm: Some(view::Confirm {
                        title: Text::localized(
                            "Clear this task's history?",
                            "清除此任务的历史记录？",
                            "清除此任務的歷史紀錄？",
                        )
                        .resolve(locale)
                        .into(),
                        message: display(&task.title, 512, false),
                        destructive: true,
                    }),
                });
            }
        }
        Route::Run { id } => {
            let Some(run) = task.runs.iter().find(|run| run.id == id) else {
                return Ok(view::Reply::Rejected {
                    message: Text::localized(
                        "This run is no longer retained.",
                        "此运行记录已不再保留。",
                        "此執行紀錄已不再保留。",
                    )
                    .resolve(locale)
                    .into(),
                });
            };
            page.title = outcome(run.outcome);
            page.body = format!(
                "{}\n{}\n\n{}\n\n{}: {}\n{}: {}",
                display(&task.title, 512, false),
                instant(Some(run.at)),
                display(&run.message, 8192, true),
                Text::localized("Fire ID", "触发 ID", "觸發 ID").resolve(locale),
                display(&run.id, 512, false),
                Text::localized("Run ID", "运行 ID", "執行 ID").resolve(locale),
                run.run_id.as_deref().unwrap_or("—")
            );
            session = run.session_id.clone();
        }
        Route::Error => {
            page.title = Text::localized("Last error", "最近错误", "最近錯誤");
            page.body = display(task.last_error.as_deref().unwrap_or("—"), 8192, true);
        }
    }
    let mut view = page.view(locale);
    if let Some(session) = session {
        let link = Node::Item {
            key: "session-result".into(),
            title: Text::localized("Open result session", "打开结果会话", "開啟結果對話")
                .resolve(locale)
                .into(),
            detail: session.clone(),
            meta: String::new(),
            tone: Tone::Normal,
            current: false,
            target: Target::Session { session },
        };
        if let Node::Column { children, .. } = &mut view.root {
            children.push(link);
        }
    }
    Ok(view::Reply::View { view })
}
