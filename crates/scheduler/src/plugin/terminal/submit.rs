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

pub(super) async fn submit(
    service: &Service,
    route: Route,
    revision: String,
    action: String,
    fields: BTreeMap<String, Value>,
    grant: Option<maka_plugins::authorization::Id>,
    cx: (&Caller, &str),
) -> Result<Reply, Error> {
    let (caller, locale) = cx;
    let task_id = route.task.clone().ok_or_else(|| invalid("Select a task"))?;
    let revision = revision.parse::<u64>().map_err(invalid)?;
    if grant.is_some() && !matches!(route.target, Some(target::Route::Selected { .. })) {
        return Err(invalid("Unexpected authorization"));
    }
    let mut accepted_grant = None;
    let mutation = if let Some(target::Route::Selected { target }) = &route.target {
        if action != "save_target" || !fields.is_empty() {
            return Err(invalid("Invalid task target action"));
        }
        let (_, current, _) = task(service, &task_id)?;
        if current != revision {
            return Ok(Reply::Conflict);
        }
        let Some(effect) = target
            .resolve(service.backend.host.sessions.as_ref(), caller, locale)
            .await?
        else {
            return Ok(Reply::Conflict);
        };
        accepted_grant = target::grant(service, &effect, grant).await?;
        if accepted_grant.is_none() {
            return Ok(Reply::Consent {
                request: target::consent(&effect, uuid::Uuid::new_v4(), locale),
            });
        }
        Mutation::Update {
            task_id,
            patch: Update {
                effect: Some(effect),
                ..Default::default()
            },
        }
    } else if let Some(settings) = route.settings {
        if grant.is_some() {
            return Err(invalid("Unexpected authorization"));
        }
        let (task, current, timezone) = task(service, &task_id)?;
        if current != revision {
            return Ok(Reply::Conflict);
        }
        match settings::mutation(settings, &task, &timezone, &action, fields) {
            Ok(mutation) => mutation,
            Err(_) => {
                return Ok(Reply::Rejected {
                    message: Text::localized(
                        "Check the run limit, expiry and snooze duration",
                        "请检查次数限制、到期时间和推迟时长",
                        "請檢查次數限制、到期時間和延後時間",
                    ),
                });
            }
        }
    } else if route.timing {
        if action != "save_schedule" {
            return Err(invalid("Unknown schedule action"));
        }
        let QueryResult::Task {
            task: Some(task),
            revision: Some(current),
            timezone: Some(timezone),
        } = service
            .query(Query::Get {
                task_id: task_id.clone(),
            })
            .map_err(error)?
        else {
            return Ok(Reply::Conflict);
        };
        if current != revision {
            return Ok(Reply::Conflict);
        }
        let patch = match timing::update(&task.schedule, &timezone, fields) {
            Ok(schedule) => schedule,
            Err(_) => {
                return Ok(Reply::Rejected {
                    message: Text::localized(
                        "Check the date, UTC offset and recurrence fields",
                        "请检查日期、时区偏移和重复规则",
                        "請檢查日期、時區偏移和重複規則",
                    ),
                });
            }
        };
        Mutation::Update {
            task_id,
            patch: Update {
                schedule: (patch != task.schedule).then_some(patch),
                ..Update::default()
            },
        }
    } else {
        if grant.is_some() {
            return Err(invalid("Unexpected authorization"));
        }
        mutation(task_id, &action, fields)?
    };
    // A deleted task has no page to return to; the list is where it was.
    let landing = if action == "delete" {
        Value::Null
    } else if action == "save_target" {
        serde_json::to_value(Route {
            task: route.task.clone(),
            ..Default::default()
        })
        .map_err(invalid)?
    } else if action == "clear" {
        history::route(
            route.task.as_deref().expect("task action"),
            history::Route::List {
                offset: 0,
                revision: None,
            },
        )
    } else {
        serde_json::to_value(route).map_err(invalid)?
    };
    // The preliminary read only decodes the form; the owner still checks the
    // same revision after any concurrent edit before accepting this mutation.
    Ok(
        match service
            .handle
            .mutate_if_current(
                mutation,
                Origin::User {
                    grant: accepted_grant,
                },
                revision,
            )
            .await
        {
            Ok(_) => Reply::Applied { route: landing },
            Err(crate::Error::RevisionConflict) => Reply::Conflict,
            Err(crate::Error::Invalid(_) | crate::Error::Time(_)) => Reply::Rejected {
                message: Text::localized(
                    "Check the task fields and status",
                    "请检查任务内容和状态",
                    "請檢查任務內容和狀態",
                ),
            },
            Err(failure) => return Err(error(failure)),
        },
    )
}

pub(super) fn mutation(
    task_id: String,
    action: &str,
    mut fields: BTreeMap<String, Value>,
) -> Result<Mutation, Error> {
    match action {
        "save" if fields.len() == 2 => {
            let title = fields
                .remove("title")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| invalid("Invalid task title"))?;
            let intent_body = fields
                .remove("intent")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| invalid("Invalid task content"))?;
            Ok(Mutation::Update {
                task_id,
                patch: Update {
                    title: Some(title),
                    intent_body: Some(intent_body),
                    ..Update::default()
                },
            })
        }
        "pause" if fields.is_empty() => Ok(Mutation::Pause { task_id }),
        "resume" if fields.is_empty() => Ok(Mutation::Resume { task_id }),
        "trigger" if fields.is_empty() => Ok(Mutation::TriggerNow { task_id }),
        "clear" if fields.is_empty() => Ok(Mutation::ClearHistory { task_id }),
        "delete" if fields.is_empty() => Ok(Mutation::Delete { task_id }),
        _ => Err(invalid("Unknown scheduled-task action or fields")),
    }
}
