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

use super::{
    Action, Error, Reply, Row, Service, Text, Value, empty, error, field, invalid, settings,
    target, timing,
};
use crate::{
    authorization::Origin,
    command::{Mutation, MutationResult},
    schedule::{Recurrence, Schedule},
    task::Create,
};
use maka_plugins::authorization::Id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Route {
    Pick,
    Form {
        schedule: Kind,
        #[serde(default)]
        target: target::Selector,
    },
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Once,
    Interval,
    Daily,
    Weekly,
    Monthly,
    Cron,
}
impl Kind {
    pub(super) fn schedule(self, at: i64) -> Schedule {
        match self {
            Self::Once => Schedule::Once { run_at: at },
            Self::Interval => Schedule::Interval {
                every_seconds: 3600,
                start_at: at,
            },
            Self::Daily | Self::Weekly | Self::Monthly => Schedule::Calendar {
                recurrence: match self {
                    Self::Daily => Recurrence::Daily,
                    Self::Weekly => Recurrence::Weekly,
                    _ => Recurrence::Monthly,
                },
                anchor_at: at,
            },
            Self::Cron => Schedule::Cron {
                expression: "0 9 * * *".into(),
                start_at: at,
            },
        }
    }
}
fn title() -> Text {
    Text::localized("New task", "新建任务", "新增任務")
}
pub(super) fn route(creation: Route) -> Value {
    serde_json::to_value(super::Route {
        creation: Some(creation),
        ..super::Route::default()
    })
    .expect("creation route")
}
pub(super) fn entry() -> Row {
    Row {
        id: "create".into(),
        title: title(),
        description: String::new(),
        route: serde_json::to_value(super::Route {
            target: Some(target::Route::Pick),
            ..Default::default()
        })
        .expect("task type route"),
    }
}
pub(super) async fn read(
    service: &Service,
    caller: &super::Caller,
    creation: Route,
    locale: &str,
) -> Result<Reply, Error> {
    let mut page = empty(title(), 0);
    match creation {
        Route::Pick => {
            let selector = target::Selector::Local;
            page.body = target::summary(
                &crate::task::Effect::Notify(crate::task::Notification::Local),
                locale,
            );
            for (i, kind) in [
                Kind::Once,
                Kind::Interval,
                Kind::Daily,
                Kind::Weekly,
                Kind::Monthly,
                Kind::Cron,
            ]
            .into_iter()
            .enumerate()
            {
                page.rows.push(Row {
                    id: format!("schedule-{i}"),
                    title: timing::title(&kind.schedule(0)),
                    description: String::new(),
                    route: route(Route::Form {
                        schedule: kind,
                        target: selector.clone(),
                    }),
                });
            }
        }
        Route::Form { schedule, target } => {
            let Some(effect) = target
                .resolve(service.backend.host.sessions.as_ref(), caller, locale)
                .await?
            else {
                return Ok(Reply::Conflict);
            };
            page = form(&service.timezone, schedule, &effect, locale)?;
        }
    }
    Ok(Reply::Page { page })
}
pub(super) fn form(
    timezone: &str,
    schedule: Kind,
    effect: &crate::task::Effect,
    locale: &str,
) -> Result<super::Page, Error> {
    let mut page = empty(title(), 0);
    page.revision = uuid::Uuid::new_v4().to_string();
    page.body = format!("{}\n{}", target::summary(effect, locale), timezone);
    page.fields = vec![
        field(
            "title",
            Text::localized("Title", "标题", "標題"),
            String::new(),
            512,
            false,
        ),
        field(
            "intent",
            Text::localized("Content", "内容", "內容"),
            String::new(),
            super::INTENT_BYTES,
            true,
        ),
    ];
    page.fields.extend(timing::fields(
        &schedule.schedule(jiff::Timestamp::now().as_millisecond() + 3_600_000),
        timezone,
        locale,
    )?);
    page.fields
        .extend(settings::limit_fields(None, None, timezone)?);
    page.actions.push(Action {
        id: "create".into(),
        label: Text::localized("Create task", "创建任务", "建立任務"),
        enabled: true,
        fields: page.fields.iter().map(|field| field.id.clone()).collect(),
        recovery: Some(serde_json::json!({"operation":page.revision})),
        confirm: None,
    });
    Ok(page)
}
pub(super) async fn submit(
    service: &Service,
    route: Route,
    revision: String,
    action: String,
    mut fields: BTreeMap<String, Value>,
    grant: Option<Id>,
    cx: (&super::Caller, &str),
) -> Result<Reply, Error> {
    let (caller, locale) = cx;
    let Route::Form { schedule, target } = route else {
        return Err(invalid("Select a reminder schedule"));
    };
    if action != "create" {
        return Err(invalid("Unknown reminder action"));
    }
    let operation_id = uuid::Uuid::parse_str(&revision).map_err(invalid)?;
    let recorded = service
        .creation(operation_id)
        .await
        .map_err(error)?
        .is_some();
    let Some(effect) = target
        .resolve(service.backend.host.sessions.as_ref(), caller, locale)
        .await?
    else {
        return Ok(Reply::Conflict);
    };
    let input = (|| {
        let title = take(&mut fields, "title")?;
        let intent_body = take(&mut fields, "intent")?;
        // Optional constraints were absent from earlier creation forms. Their
        // original None values must survive exact creation-receipt replay.
        for key in ["max_fires", "expires"] {
            fields
                .entry(key.into())
                .or_insert(Value::String(String::new()));
        }
        let (max_fires, expires_at) = settings::limits(&mut fields, None, &service.timezone)?;
        let schedule = timing::update(&schedule.schedule(0), &service.timezone, fields)?;
        let input = Create {
            title,
            intent_body,
            schedule,
            effect,
            max_fires,
            expires_at,
        };
        if !recorded {
            input
                .validate(jiff::Timestamp::now().as_millisecond())
                .map_err(invalid)?;
        }
        Ok::<_, Error>(input)
    })();
    let input = match input {
        Ok(input) => input,
        Err(_) => {
            return Ok(Reply::Rejected {
                message: Text::localized(
                    "Check the title, content, schedule and limits",
                    "请检查标题、内容、计划和限制",
                    "請檢查標題、內容、排程和限制",
                ),
            });
        }
    };
    let grant = if recorded {
        // Reconciliation is a read of the original creation, not a new grant.
        // The owner still checks the complete immutable input fingerprint.
        None
    } else {
        match target::grant(service, &input.effect, grant).await? {
            Some(id) => Some(id),
            None => {
                return Ok(Reply::Consent {
                    request: target::consent(&input.effect, operation_id, locale),
                });
            }
        }
    };
    let MutationResult::Created { task_id, .. } = service
        .mutate(
            Mutation::CreateOnce {
                operation_id,
                input,
            },
            Origin::User { grant },
        )
        .await
        .map_err(error)?
    else {
        return Err(invalid("Expected a created task"));
    };
    Ok(Reply::Applied {
        route: serde_json::to_value(super::Route {
            task: Some(task_id),
            ..super::Route::default()
        })
        .map_err(invalid)?,
    })
}
pub(super) async fn recover(service: &Service, value: Value) -> Result<Reply, Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Recovery {
        operation: uuid::Uuid,
    }
    let recovery: Recovery = serde_json::from_value(value).map_err(invalid)?;
    match service.creation(recovery.operation).await.map_err(error)? {
        Some(task) => Ok(Reply::Applied {
            route: serde_json::to_value(super::Route {
                task: Some(task),
                ..Default::default()
            })
            .map_err(invalid)?,
        }),
        None => Ok(Reply::Unrecorded),
    }
}
pub(super) fn take(fields: &mut BTreeMap<String, Value>, key: &str) -> Result<String, Error> {
    fields
        .remove(key)
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| invalid("Missing reminder field"))
}
