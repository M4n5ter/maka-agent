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
use maka_plugins::terminal_ui::view::Choice;
use maka_runtime::configuration::Patch;

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Limits,
    Snooze,
}

pub(super) fn limit_fields(
    max: Option<u32>,
    expires: Option<i64>,
    timezone: &str,
) -> Result<Vec<Field>, Error> {
    Ok(vec![
        field(
            "max_fires",
            Text::localized(
                "Maximum runs · blank for unlimited",
                "最多运行次数 · 留空不限",
                "最多執行次數 · 留空不限",
            ),
            max.map(|value| value.to_string()).unwrap_or_default(),
            16,
            false,
        ),
        field(
            "expires",
            Text::localized(
                "Expiry · date and UTC offset, or blank",
                "到期时间 · 日期及时区偏移，可留空",
                "到期時間 · 日期及時區偏移，可留空",
            ),
            expires
                .map(|at| timing::date(at, timezone))
                .transpose()?
                .unwrap_or_default(),
            64,
            false,
        ),
    ])
}
pub(super) fn limits(
    fields: &mut BTreeMap<String, Value>,
    previous: Option<i64>,
    timezone: &str,
) -> Result<(Option<u32>, Option<i64>), Error> {
    let maximum = create::take(fields, "max_fires")?;
    let expiry = create::take(fields, "expires")?;
    let maximum = if maximum.trim().is_empty() {
        None
    } else {
        let value = maximum.trim().parse::<u32>().map_err(invalid)?;
        if !(1..=10_000).contains(&value) {
            return Err(invalid("Maximum runs must be between 1 and 10000"));
        }
        Some(value)
    };
    let expiry = if expiry.trim().is_empty() {
        None
    } else if previous
        .is_some_and(|at| timing::date(at, timezone).is_ok_and(|value| value == expiry))
    {
        previous
    } else {
        Some(timing::parse_date(&expiry)?)
    };
    Ok((maximum, expiry))
}
pub(super) fn page(
    kind: Kind,
    task: &Task,
    revision: u64,
    timezone: &str,
    locale: &str,
) -> Result<Page, Error> {
    let (title, id) = match kind {
        Kind::Limits => (
            Text::localized("Run limits", "运行限制", "執行限制"),
            "save_limits",
        ),
        Kind::Snooze => (Text::localized("Snooze", "推迟", "延後"), "snooze"),
    };
    let mut page = empty(title, revision);
    page.body = match kind {
        Kind::Limits => format!(
            "{} · {} {}\n{}",
            display(&task.title, 512, false),
            task.fire_count,
            Text::localized("runs so far", "次已运行", "次已執行").resolve(locale),
            timezone
        ),
        Kind::Snooze => Text::localized(
            "Add a delay to the next trigger, from 1 millisecond to 7 days.",
            "将下次触发推迟指定时长，范围为 1 毫秒至 7 天。",
            "將下次觸發延後指定時間，範圍為 1 毫秒至 7 天。",
        )
        .resolve(locale)
        .into(),
    };
    page.fields = match kind {
        Kind::Limits => limit_fields(task.max_fires, task.expires_at, timezone)?,
        Kind::Snooze => vec![
            field(
                "delay",
                Text::localized("Delay", "时长", "時間"),
                "60".into(),
                16,
                false,
            ),
            Field {
                id: "unit".into(),
                label: Text::localized("Unit", "单位", "單位"),
                enabled: true,
                control: Control::Choice {
                    value: "minutes".into(),
                    options: [
                        (
                            "milliseconds",
                            Text::localized("Milliseconds", "毫秒", "毫秒"),
                        ),
                        ("seconds", Text::localized("Seconds", "秒", "秒")),
                        ("minutes", Text::localized("Minutes", "分钟", "分鐘")),
                        ("hours", Text::localized("Hours", "小时", "小時")),
                        ("days", Text::localized("Days", "天", "天")),
                    ]
                    .into_iter()
                    .map(|(value, label)| Choice {
                        value: value.into(),
                        label: label.resolve(locale).into(),
                    })
                    .collect(),
                },
            },
        ],
    };
    let enabled = match kind {
        Kind::Limits => matches!(task.status, Status::Active | Status::Paused),
        Kind::Snooze => task.status == Status::Active,
    };
    for field in &mut page.fields {
        field.enabled = enabled;
    }
    page.actions.push(Action {
        id: id.into(),
        label: Text::localized("Save", "保存", "儲存"),
        enabled,
        fields: page.fields.iter().map(|field| field.id.clone()).collect(),
        recovery: None,
        confirm: None,
    });
    Ok(page)
}
pub(super) fn mutation(
    kind: Kind,
    task: &Task,
    timezone: &str,
    action: &str,
    mut fields: BTreeMap<String, Value>,
) -> Result<Mutation, Error> {
    let mutation = match (kind, action) {
        (Kind::Limits, "save_limits") => {
            let (maximum, expiry) = limits(&mut fields, task.expires_at, timezone)?;
            Mutation::Update {
                task_id: task.id.clone(),
                patch: Update {
                    max_fires: patch(task.max_fires, maximum),
                    expires_at: patch(task.expires_at, expiry),
                    ..Default::default()
                },
            }
        }
        (Kind::Snooze, "snooze") => {
            let delay = create::take(&mut fields, "delay")?
                .trim()
                .parse::<i64>()
                .map_err(invalid)?;
            let multiplier = match create::take(&mut fields, "unit")?.as_str() {
                "milliseconds" => 1,
                "seconds" => 1000,
                "minutes" => 60_000,
                "hours" => 3_600_000,
                "days" => 86_400_000,
                _ => return Err(invalid("Invalid snooze unit")),
            };
            let delay_ms = delay
                .checked_mul(multiplier)
                .filter(|value| (1..=604_800_000).contains(value))
                .ok_or_else(|| invalid("Snooze must be between 1 millisecond and 7 days"))?;
            Mutation::Snooze {
                task_id: task.id.clone(),
                delay_ms,
            }
        }
        _ => return Err(invalid("Unknown task settings action")),
    };
    if !fields.is_empty() {
        return Err(invalid("Unexpected task settings fields"));
    }
    Ok(mutation)
}
fn patch<T: PartialEq>(old: Option<T>, value: Option<T>) -> Patch<T> {
    if old == value {
        Patch::Keep
    } else {
        value.map_or(Patch::Clear, Patch::Set)
    }
}
