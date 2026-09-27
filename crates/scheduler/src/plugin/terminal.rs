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

mod create;
mod detail;
mod history;
mod read;
mod submit;
use read::read;
#[cfg(test)]
use submit::mutation;
use submit::submit;
mod settings;
mod target;
use detail::page as detail;
#[cfg(test)]
mod product;
mod timing;

use super::{ID, Service, remote::error};
use crate::{
    authorization::Origin,
    command::{Mutation, Query, QueryResult, Update},
    task::{Intent, Status, Task},
};
use futures_util::future::BoxFuture;
use maka_plugins::{
    contributions::Staged,
    remote::{Caller, Endpoint, Error, Handler, Method, key},
    terminal_ui::{
        Context, Descriptor, Text,
        page::{Action, Control, Field, Page, Reply, Request, Row},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

const WINDOW: usize = 8;
const INTENT_BYTES: usize = 16 * 1024;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Route {
    cursor: Option<String>,
    revision: Option<u64>,
    offset: usize,
    task: Option<String>,
    timing: bool,
    creation: Option<create::Route>,
    target: Option<target::Route>,
    settings: Option<settings::Kind>,
    history: Option<history::Route>,
}

pub(super) fn publish(service: Service, staged: &mut Staged) -> Result<(), String> {
    let endpoint = Endpoint::standalone(Handler::Method(Arc::new(View(service))))
        .with_terminal_view(
            Descriptor::new(
                Text::localized("Scheduled tasks", "计划任务", "排程任務"),
                Context::Application,
            )
            .icon("⏲", "S")
            .changes("changes")
            .order(20),
        )
        .map_err(super::display)?;
    staged
        .insert(key(ID, "terminal").map_err(super::display)?, endpoint)
        .map_err(super::display)
}

struct View(Service);
impl Method for View {
    fn call(&self, input: Value, caller: Caller) -> BoxFuture<'static, Result<Value, Error>> {
        let service = self.0.clone();
        Box::pin(async move {
            let request: Request = serde_json::from_value(input).map_err(invalid)?;
            request.validate().map_err(invalid)?;
            let locale = request.locale().to_owned();
            let _lease = service.context.admit().map_err(|_| Error::Retired)?;
            let reply = match request {
                Request::Recover { route, .. } => create::recover(&service, route).await?,
                Request::Read { route, .. } => {
                    let reply = read(&service, &caller, decode(route)?, &locale).await?;
                    reply.validate().map_err(invalid)?;
                    return serde_json::to_value(reply).map_err(invalid);
                }
                Request::Submit {
                    route,
                    revision,
                    action,
                    fields,
                    grant,
                    ..
                } => {
                    let route = decode(route)?;
                    if let Some(creation) = route.creation {
                        create::submit(
                            &service,
                            creation,
                            revision,
                            action,
                            fields,
                            grant,
                            (&caller, &locale),
                        )
                        .await?
                    } else {
                        submit(
                            &service,
                            route,
                            revision,
                            action,
                            fields,
                            grant,
                            (&caller, &locale),
                        )
                        .await?
                    }
                }
            };
            let reply = reply.view(&locale);
            reply.validate().map_err(invalid)?;
            serde_json::to_value(reply).map_err(invalid)
        })
    }
}

fn decode(value: Value) -> Result<Route, Error> {
    let route: Route = if value.is_null() {
        Route::default()
    } else {
        serde_json::from_value(value).map_err(invalid)?
    };
    let modes = usize::from(route.timing)
        + usize::from(route.creation.is_some())
        + usize::from(route.target.is_some())
        + usize::from(route.settings.is_some())
        + usize::from(route.history.is_some());
    if modes > 1
        || route.offset >= 64
        || !route.offset.is_multiple_of(WINDOW)
        || (route.task.is_some() || modes > 0)
            && (route.cursor.is_some() || route.revision.is_some() || route.offset != 0)
        || route.creation.is_some() && route.task.is_some()
        || (route.timing || route.settings.is_some() || route.history.is_some())
            && route.task.is_none()
    {
        return Err(invalid("Invalid scheduled-task route"));
    }
    Ok(route)
}
fn task(service: &Service, id: &str) -> Result<(Task, u64, String), Error> {
    match service
        .query(Query::Get { task_id: id.into() })
        .map_err(error)?
    {
        QueryResult::Task {
            task: Some(task),
            revision: Some(revision),
            timezone: Some(timezone),
        } => Ok((*task, revision, timezone)),
        _ => Err(invalid("Task no longer exists")),
    }
}
fn empty(title: Text, revision: u64) -> Page {
    Page {
        title,
        revision: revision.to_string(),
        body: String::new(),
        rows: vec![],
        fields: vec![],
        actions: vec![],
    }
}
fn field(id: &str, label: Text, value: String, max_bytes: usize, multiline: bool) -> Field {
    Field {
        id: id.into(),
        label,
        enabled: true,
        control: Control::Text {
            value,
            max_bytes,
            multiline,
            placeholder: String::new(),
            secret: false,
        },
    }
}
fn status(value: Status) -> Text {
    match value {
        Status::Active => Text::localized("Active", "进行中", "進行中"),
        Status::Paused => Text::localized("Paused", "已暂停", "已暫停"),
        Status::Completed => Text::localized("Completed", "已完成", "已完成"),
        Status::Expired => Text::localized("Expired", "已过期", "已過期"),
    }
}
fn instant(value: Option<i64>) -> String {
    value
        .and_then(|value| jiff::Timestamp::from_millisecond(value).ok())
        .map_or_else(String::new, |value| {
            value
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M %:z")
                .to_string()
        })
}
fn display(value: &str, max: usize, multiline: bool) -> String {
    let mut text = String::new();
    for c in value.chars().filter(|c| {
        (!c.is_control() || multiline && matches!(c, '\n' | '\t'))
            && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        if text.len() + c.len_utf8() > max {
            break;
        }
        text.push(c);
    }
    if !multiline && text.trim().is_empty() {
        text = "—".into();
    }
    text
}
fn invalid(error: impl ToString) -> Error {
    Error::Invalid(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        plan::Plan,
        schedule::Schedule,
        task::{Create, Creator, Effect, Notification},
    };

    #[test]
    fn task_states_are_localized_words_and_user_symbols_are_preserved() {
        let mut task = Plan::create(
            "one".into(),
            Create {
                title: "任务 ▶ Ⅱ ✓ — 😀".into(),
                intent_body: "提醒 ▶ Ⅱ ✓ — 😀".into(),
                schedule: Schedule::Once { run_at: 10_000 },
                effect: Effect::Notify(Notification::Local),
                max_fires: None,
                expires_at: None,
            },
            Creator::User,
            "UTC".into(),
            1000,
        )
        .unwrap()
        .task;
        for (state, labels) in [
            (Status::Active, ["Active", "进行中", "進行中"]),
            (Status::Paused, ["Paused", "已暂停", "已暫停"]),
            (Status::Completed, ["Completed", "已完成", "已完成"]),
            (Status::Expired, ["Expired", "已过期", "已過期"]),
        ] {
            task.status = state;
            for (locale, label) in ["en", "zh-CN", "zh-TW"].into_iter().zip(labels) {
                let page = detail(task.clone(), 42, locale);
                assert!(page.body.starts_with(&format!("{label}  ")));
                let view = page.view(locale);
                view.validate().unwrap();
                assert_eq!(view.title, "任务 ▶ Ⅱ ✓ — 😀");
                assert!(
                    serde_json::to_string(&view)
                        .unwrap()
                        .contains("提醒 ▶ Ⅱ ✓ — 😀")
                );
            }
        }
    }

    #[test]
    fn unrepresentable_content_is_read_only_never_a_truncated_edit() {
        for body in [
            "界".repeat(8000),
            "unsafe\u{202e}text".into(),
            "ordinary\ntext".into(),
        ] {
            let task = Plan::create(
                "one".into(),
                Create {
                    title: "Reminder".into(),
                    intent_body: body.clone(),
                    schedule: Schedule::Once { run_at: 10_000 },
                    effect: Effect::Notify(Notification::Local),
                    max_fires: None,
                    expires_at: None,
                },
                Creator::User,
                "UTC".into(),
                1000,
            )
            .unwrap()
            .task;
            let page = detail(task, 42, "en");
            page.clone().view("en").validate().unwrap();
            let editable = body == "ordinary\ntext";
            assert_eq!(
                page.actions.iter().any(|action| action.id == "save"),
                editable
            );
            assert_eq!(!page.fields.is_empty(), editable);
            if editable {
                assert!(
                    matches!(&page.fields[1].control, Control::Text { value, .. } if value == &body)
                );
            }
        }
        assert!(
            mutation(
                "one".into(),
                "pause",
                BTreeMap::from([("intent".into(), Value::String("hidden edit".into()))])
            )
            .is_err()
        );
    }
}
