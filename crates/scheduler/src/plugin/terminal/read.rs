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
    route: Route,
    locale: &str,
) -> Result<maka_plugins::terminal_ui::view::Reply, Error> {
    if let Some(target) = route.target {
        return target::read(service, caller, route.task, target, locale)
            .await
            .map(|reply| reply.view(locale));
    }
    if let Some(creation) = route.creation {
        return create::read(service, caller, creation, locale)
            .await
            .map(|reply| reply.view(locale));
    }
    if let Some(task_id) = route.task {
        return match service.query(Query::Get { task_id }).map_err(error)? {
            QueryResult::Task {
                task: Some(task),
                revision: Some(revision),
                timezone: Some(timezone),
            } => {
                if let Some(history) = route.history {
                    return history::read(&task, revision, history, locale);
                }
                Ok(Reply::Page {
                    page: if route.timing {
                        timing::page(&task, revision, &timezone, locale)?
                    } else if let Some(settings) = route.settings {
                        settings::page(settings, &task, revision, &timezone, locale)?
                    } else {
                        detail(*task, revision, locale)
                    },
                }
                .view(locale))
            }
            QueryResult::Task { task: None, .. } => Ok(Reply::Rejected {
                message: Text::localized("Task no longer exists", "任务已不存在", "任務已不存在"),
            }
            .view(locale)),
            _ => Err(invalid("Missing task revision")),
        };
    }
    match service
        .query(Query::List {
            cursor: route.cursor.clone(),
            expected_revision: route.revision,
        })
        .map_err(error)?
    {
        QueryResult::RevisionChanged { .. } => Ok(Reply::Conflict.view(locale)),
        QueryResult::Page {
            revision,
            tasks,
            next_cursor,
        } => {
            let mut page = empty(
                Text::localized("Scheduled tasks", "计划任务", "排程任務"),
                revision,
            );
            page.rows.push(create::entry());
            if route.offset > tasks.len() {
                return Err(invalid("Unknown scheduled-task offset"));
            }
            for task in tasks.iter().skip(route.offset).take(WINDOW) {
                page.rows.push(Row {
                    id: task.id.clone(),
                    title: Text::plain(display(&task.title, 256, false)),
                    description: format!(
                        "{}  {}",
                        status(task.status).resolve(locale),
                        instant(task.next_fire_at)
                    ),
                    route: serde_json::to_value(Route {
                        task: Some(task.id.clone()),
                        ..Route::default()
                    })
                    .map_err(invalid)?,
                });
            }
            let next = if route.offset + WINDOW < tasks.len() {
                Some(Route {
                    cursor: Some(route.cursor.unwrap_or_else(|| "0".into())),
                    revision: Some(revision),
                    offset: route.offset + WINDOW,
                    task: None,
                    timing: false,
                    creation: None,
                    ..Route::default()
                })
            } else {
                next_cursor.map(|cursor| Route {
                    cursor: Some(cursor),
                    revision: Some(revision),
                    ..Route::default()
                })
            };
            if let Some(next) = next {
                page.rows.push(Row {
                    id: "next".into(),
                    title: Text::localized("Next", "下一页", "下一頁"),
                    description: String::new(),
                    route: serde_json::to_value(next).map_err(invalid)?,
                });
            }
            Ok(Reply::Page { page }.view(locale))
        }
        _ => Err(invalid("Expected scheduled-task page")),
    }
}
