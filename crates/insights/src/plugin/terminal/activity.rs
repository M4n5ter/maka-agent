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
use maka_plugins::usage::{self, Activity, ActivityStatus, Outcome, ToolResult, ToolStatus};

const WINDOW: usize = 8;

fn revision(cursor: &str) -> String {
    maka_runtime::artifact::content_digest(cursor.as_bytes())
}

#[derive(Deserialize)]
struct Read {
    page: usage::Page,
}

pub(super) async fn read(
    insights: &dyn Method,
    caller: &Caller,
    place: &Place,
) -> Result<Option<usage::Page>, Error> {
    let input = match &place.cursor {
        Some(cursor) if place.refine => usage::Read::Refine {
            cursor: cursor.clone(),
            selection: place.selection.clone(),
        },
        Some(cursor) => usage::Read::Continue {
            cursor: cursor.clone(),
        },
        None => {
            let to = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| Error::Provider(error.to_string()))?
                .as_millis() as f64;
            let range = place.range.as_deref().unwrap_or("7d");
            let span = RANGES
                .iter()
                .find(|(id, _)| *id == range)
                .map_or(RANGES[1].1, |(_, span)| *span);
            let filter = usage::Filter {
                from: if span.is_finite() {
                    (to - span).max(0.0)
                } else {
                    0.0
                },
                to,
                session_id: place.session.clone(),
                activity: place.selection.clone(),
            };
            filter
                .validate()
                .map_err(|error| Error::Invalid(error.to_string()))?;
            usage::Read::Start { filter }
        }
    };
    let result = insights
        .call(
            json!({"kind":"activity","operationId":uuid::Uuid::new_v4(),"read":input}),
            caller.clone(),
        )
        .await?;
    if result["kind"] == "refresh_required" {
        return Ok(None);
    }
    let read: Read = decode(result)?;
    Ok(Some(read.page))
}

pub(super) fn restart(words: &Words, place: &Place) -> View {
    let fresh = Place {
        cursor: None,
        refine: false,
        offset: None,
        revision: None,
        attempt: None,
        model: None,
        page: 0,
        ..place.clone()
    };
    super::view(words.t("Snapshot changed", "快照已变化", "快照已變更"), "refresh".into(), vec![], vec![], column("root", vec![
        text("changed", words.t("The recorded snapshot is no longer available. Load the latest values to continue.", "原快照已不可用。请加载最新数据以继续。", "原快照已無法使用。請載入最新資料以繼續。"), Tone::Warning),
        link("reload", words.t("Load latest", "加载最新数据", "載入最新資料"), fresh.route()).into(),
    ]))
}

fn value<T: Serialize>(value: Option<T>) -> String {
    value.map_or_else(
        || "all".into(),
        |value| {
            serde_json::to_value(value)
                .expect("selection value")
                .as_str()
                .expect("selection string")
                .into()
        },
    )
}

fn selection<T: for<'de> Deserialize<'de>>(value: &str) -> Result<Option<T>, Error> {
    if value == "all" {
        Ok(None)
    } else {
        serde_json::from_value(json!(value))
            .map(Some)
            .map_err(|error| Error::Invalid(error.to_string()))
    }
}

pub(super) fn filter(submission: Submission) -> Result<Reply, Error> {
    let old: Place = serde_json::from_value(submission.route)
        .map_err(|error| Error::Invalid(error.to_string()))?;
    let range = submission
        .fields
        .get("range")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Invalid("Missing range".into()))?;
    if !RANGES.iter().any(|(id, _)| *id == range) {
        return Err(Error::Invalid("Invalid usage range".into()));
    }
    let text = |name: &str| {
        submission
            .fields
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Invalid(format!("Missing {name}")))
    };
    let session = text("session")?.trim();
    if session.len() > 256 || session.chars().any(char::is_control) {
        return Err(Error::Invalid("Invalid session".into()));
    }
    let session = (!session.is_empty()).then(|| session.to_owned());
    let selected = usage::Selection {
        kind: selection(text("kind")?)?,
        status: selection(text("status")?)?,
        search: text("search")?.trim().into(),
    };
    selected
        .validate()
        .map_err(|error| Error::Invalid(error.into()))?;
    let same_scope = old.range.as_deref().unwrap_or("7d") == range && old.session == session;
    let place = Place {
        tab: Some("activity".into()),
        range: Some(range.into()),
        session,
        selection: selected,
        refine: same_scope && old.cursor.is_some(),
        cursor: same_scope.then_some(old.cursor).flatten(),
        ..Default::default()
    };
    Ok(Reply::Applied {
        route: place.route(),
    })
}

fn status(words: &Words, status: ActivityStatus) -> (String, Tone) {
    let (en, cn, tw, tone) = match status {
        ActivityStatus::Success => ("Success", "成功", "成功", Tone::Success),
        ActivityStatus::Error => ("Failed", "失败", "失敗", Tone::Error),
        ActivityStatus::Aborted => ("Cancelled", "已取消", "已取消", Tone::Muted),
        ActivityStatus::Unknown => ("Unknown", "未知", "未知", Tone::Warning),
        ActivityStatus::Rejected => ("Rejected", "已拒绝", "已拒絕", Tone::Warning),
    };
    (words.t(en, cn, tw), tone)
}

fn identity(attempt: &Activity) -> (&str, &str, Option<&str>, ActivityStatus) {
    match attempt {
        Activity::Model(model) => (
            &model.request_id,
            &model.model_id,
            model.session_id.as_deref(),
            match model.outcome {
                Outcome::Success => ActivityStatus::Success,
                Outcome::Error => ActivityStatus::Error,
                Outcome::Aborted => ActivityStatus::Aborted,
                Outcome::Unknown => ActivityStatus::Unknown,
            },
        ),
        Activity::Tool(tool) => (
            &tool.request_id,
            &tool.name,
            Some(&tool.invocation.session_id),
            match tool.result {
                ToolResult::Rejected { .. } => ActivityStatus::Rejected,
                ToolResult::Settled { outcome, .. } => match outcome {
                    ToolStatus::Success => ActivityStatus::Success,
                    ToolStatus::Error => ActivityStatus::Error,
                    ToolStatus::Unknown => ActivityStatus::Unknown,
                },
            },
        ),
    }
}

fn attempt_key(attempt: &Activity) -> String {
    let kind = match attempt {
        Activity::Model(_) => "model",
        Activity::Tool(_) => "tool",
    };
    format!("{kind}:{}", identity(attempt).0)
}

fn detail(words: &Words, attempt: &Activity) -> String {
    match attempt {
        Activity::Model(model) => {
            let input = model.usage.input_tokens.map_or_else(|| "?".into(), count);
            let output = model.usage.output_tokens.map_or_else(|| "?".into(), count);
            let cost = model
                .cost_usd
                .map_or_else(|| words.t("Cost unknown", "费用未知", "費用未知"), money);
            words.t(
                &format!("{input} in · {output} out · {cost}"),
                &format!("输入 {input} · 输出 {output} · {cost}"),
                &format!("輸入 {input} · 輸出 {output} · {cost}"),
            )
        }
        Activity::Tool(tool) => tool.latency_ms().map_or_else(
            || {
                words.t(
                    "No observed execution duration",
                    "无已记录的执行时长",
                    "無已記錄的執行時間",
                )
            },
            |duration| format!("{duration:.1} ms"),
        ),
    }
}

fn instant(at: f64) -> String {
    jiff::Timestamp::from_millisecond(at as i64)
        .map_or_else(|_| at.to_string(), |at| at.to_string())
}

fn facts(words: &Words, attempt: &Activity) -> Vec<Node> {
    let fact = |id: &str, label: String, value: String| {
        stack(
            id,
            vec![
                text("label", label, Tone::Subtle),
                text("value", clean(&value, true), Tone::Normal),
            ],
        )
    };
    let unknown = || words.t("Unknown", "未知", "未知");
    let (completed, duration, binding, request) = match attempt {
        Activity::Model(model) => (
            model.completed_at,
            Some(model.latency_ms()),
            model.binding.as_ref(),
            &model.request_id,
        ),
        Activity::Tool(tool) => (
            tool.completed_at,
            tool.latency_ms(),
            tool.binding.as_ref(),
            &tool.request_id,
        ),
    };
    let mut nodes = vec![
        fact(
            "settled",
            words.t("Settled · UTC", "结算时间 · UTC", "結算時間 · UTC"),
            instant(completed),
        ),
        fact(
            "duration",
            words.t("Observed duration", "已记录的执行时长", "已記錄的執行時間"),
            duration.map_or_else(unknown, |duration| format!("{duration:.1} ms")),
        ),
    ];
    if let Some(binding) = binding {
        nodes.push(fact(
            "connection",
            words.t("Connection", "连接", "連線"),
            binding.connection_slug.clone(),
        ));
    }
    match attempt {
        Activity::Model(model) => {
            nodes.push(fact(
                "provider",
                words.t("Provider", "提供商", "提供者"),
                model
                    .quote
                    .as_ref()
                    .map(|quote| quote.provider_id.clone())
                    .unwrap_or_else(unknown),
            ));
            for (id, label, value) in [
                (
                    "input",
                    words.t("Input tokens", "输入 Token", "輸入 Token"),
                    model.usage.input_tokens,
                ),
                (
                    "output",
                    words.t("Output tokens", "输出 Token", "輸出 Token"),
                    model.usage.output_tokens,
                ),
                (
                    "cache-read",
                    words.t("Cache read tokens", "缓存读取 Token", "快取讀取 Token"),
                    model.usage.cache_read_tokens,
                ),
                (
                    "cache-write",
                    words.t("Cache write tokens", "缓存写入 Token", "快取寫入 Token"),
                    model.usage.cache_write_tokens,
                ),
                (
                    "reasoning",
                    words.t("Reasoning tokens", "推理 Token", "推理 Token"),
                    model.usage.reasoning_tokens,
                ),
            ] {
                nodes.push(fact(
                    id,
                    label,
                    value.map_or_else(unknown, |value| value.to_string()),
                ));
            }
        }
        Activity::Tool(tool) => {
            if let ToolResult::Rejected { reason } = tool.result {
                use maka_runtime::tool_call::RejectionKind;
                let (en, cn, tw) = match reason {
                    RejectionKind::Unavailable => {
                        ("Tool unavailable", "工具不可用", "工具無法使用")
                    }
                    RejectionKind::InvalidInput => ("Invalid input", "输入无效", "輸入無效"),
                    RejectionKind::PolicyDenied => ("Permission denied", "权限被拒绝", "權限遭拒"),
                    RejectionKind::PreparationFailed => {
                        ("Preparation failed", "准备失败", "準備失敗")
                    }
                    RejectionKind::ExclusiveConflict => {
                        ("Conflicting execution", "执行冲突", "執行衝突")
                    }
                    RejectionKind::Cancelled => {
                        ("Cancelled before execution", "执行前已取消", "執行前已取消")
                    }
                };
                nodes.push(fact(
                    "reason",
                    words.t("Reason", "原因", "原因"),
                    words.t(en, cn, tw),
                ));
            }
        }
    }
    nodes.push(fact(
        "request",
        words.t("Record ID", "记录 ID", "紀錄 ID"),
        request.clone(),
    ));
    nodes
}

fn page_links(words: &Words, place: &Place, total: usize) -> Vec<Node> {
    let page = place.page.min(total.saturating_sub(1) / WINDOW);
    let mut nodes = vec![];
    for (key, label, target) in [
        (
            "previous",
            words.t("Previous", "上一页", "上一頁"),
            page.checked_sub(1),
        ),
        (
            "next",
            words.t("More", "更多", "更多"),
            ((page + 1) * WINDOW < total).then_some(page + 1),
        ),
    ] {
        if let Some(page) = target {
            nodes.push(
                link(
                    key,
                    label,
                    Place {
                        page,
                        attempt: None,
                        ..place.clone()
                    }
                    .route(),
                )
                .into(),
            );
        }
    }
    nodes
}

fn fields(words: &Words, place: &Place) -> (Vec<view::Field>, Vec<Node>) {
    let all = || words.t("All", "全部", "全部");
    let ranges = RANGES
        .iter()
        .map(|(id, _)| {
            (
                (*id).into(),
                match *id {
                    "24h" => words.t("Day", "一天", "一天"),
                    "7d" => words.t("Week", "一周", "一週"),
                    "30d" => words.t("Month", "一个月", "一個月"),
                    _ => all(),
                },
            )
        })
        .collect();
    let mut outcomes = vec![("all".into(), all())];
    outcomes.extend(
        [
            ActivityStatus::Success,
            ActivityStatus::Error,
            ActivityStatus::Aborted,
            ActivityStatus::Unknown,
            ActivityStatus::Rejected,
        ]
        .into_iter()
        .map(|outcome| (value(Some(outcome)), status(words, outcome).0)),
    );
    (
        vec![
            choice("range", place.range.as_deref().unwrap_or("7d"), ranges),
            line("session", place.session.as_deref().unwrap_or_default(), 256),
            choice(
                "kind",
                value(place.selection.kind),
                vec![
                    ("all".into(), all()),
                    ("model".into(), words.t("Model", "模型", "模型")),
                    ("tool".into(), words.t("Tool", "工具", "工具")),
                ],
            ),
            choice("status", value(place.selection.status), outcomes),
            line("search", &place.selection.search, 1024),
        ],
        vec![
            input("range", "range", words.t("Range", "时间范围", "時間範圍")),
            input(
                "session",
                "session",
                words.t(
                    "Session ID · optional",
                    "会话 ID · 可选",
                    "工作階段 ID · 選填",
                ),
            ),
            input("kind", "kind", words.t("Kind", "类型", "類型")),
            input("status", "status", words.t("Outcome", "结果", "結果")),
            input("search", "search", words.t("Find", "查找", "尋找")),
        ],
    )
}

pub(super) fn view(words: &Words, place: &Place, page: &usage::Page) -> Result<View, Error> {
    if let Some(id) = &place.attempt {
        let attempt = page
            .attempts
            .iter()
            .find(|attempt| attempt_key(attempt) == *id)
            .ok_or_else(|| {
                Error::Invalid("This activity is no longer in the snapshot; refresh".into())
            })?;
        let (_, name, session, outcome) = identity(attempt);
        let mut nodes = vec![
            link(
                "activities",
                words.t("Activity", "活动", "活動"),
                Place {
                    attempt: None,
                    ..place.clone()
                }
                .route(),
            )
            .into(),
            heading("name", clean(name, false)),
            text("status", status(words, outcome).0, status(words, outcome).1),
            text("totals", detail(words, attempt), Tone::Normal),
        ];
        if let Some(session) = session {
            nodes.push(Node::Item {
                key: "session".into(),
                title: words.t("Open session", "打开会话", "開啟工作階段"),
                detail: clean(session, false),
                meta: String::new(),
                tone: Tone::Accent,
                current: false,
                target: view::Target::Session {
                    session: session.into(),
                },
            });
        }
        nodes.extend(facts(words, attempt));
        return Ok(super::view(
            price_title(name),
            revision(&page.cursor),
            vec![],
            vec![],
            column("root", nodes),
        ));
    }
    let (fields, inputs) = fields(words, place);
    let mut nodes = vec![
        tabs_at(words, "activity", place),
        stack("filters", inputs),
        row("apply", vec![button("filter", "filter", Role::Primary)]),
    ];
    nodes.push(text(
        "count",
        words.t(
            &format!("{} matching activities", page.total),
            &format!("{} 条匹配活动", page.total),
            &format!("{} 筆符合的活動", page.total),
        ),
        Tone::Muted,
    ));
    // A viewport window is not an inventory limit. Large opaque cursors also
    // count towards each row's route, so stop before the wire budget is used.
    let start = place.page.min(page.attempts.len());
    let size = |value: &Value| serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len());
    let reserve = size(&json!(&nodes))
        .saturating_add(size(&json!(&fields)))
        .saturating_add(size(&place.route()).saturating_mul(3))
        .saturating_add(4096);
    let budget = view::MAX_BYTES.saturating_sub(reserve).min(24 * 1024);
    let mut bytes: usize = 0;
    let mut rows: Vec<Node> = vec![];
    for (index, attempt) in page.attempts.iter().skip(start).take(WINDOW).enumerate() {
        let (_, name, _, outcome) = identity(attempt);
        let (label, tone) = status(words, outcome);
        let node: Node = link(
            format!("attempt-{index}"),
            price_title(name),
            Place {
                attempt: Some(attempt_key(attempt)),
                ..place.clone()
            }
            .route(),
        )
        .detail(detail(words, attempt))
        .meta(label)
        .tone(tone)
        .into();
        let length = size(&json!(&node));
        if !rows.is_empty() && bytes.saturating_add(length) > budget {
            break;
        }
        bytes = bytes.saturating_add(length);
        rows.push(node);
    }
    let next = start + rows.len();
    nodes.push(stack("activities", rows));
    if start > 0 {
        nodes.push(
            link(
                "first",
                words.t("First on this page", "本页开头", "本頁開頭"),
                Place {
                    page: 0,
                    ..place.clone()
                }
                .route(),
            )
            .into(),
        );
    }
    if next < page.attempts.len() {
        nodes.push(
            link(
                "next",
                words.t("More activity", "更多活动", "更多活動"),
                Place {
                    page: next,
                    ..place.clone()
                }
                .route(),
            )
            .into(),
        );
    }
    if next >= page.attempts.len()
        && let Some(cursor) = &page.next_cursor
    {
        nodes.push(
            link(
                "older",
                words.t("Older activity", "更早的活动", "更早的活動"),
                Place {
                    cursor: Some(cursor.clone()),
                    page: 0,
                    ..place.clone()
                }
                .route(),
            )
            .into(),
        );
    }
    let fresh = Place {
        cursor: None,
        page: 0,
        ..place.clone()
    };
    nodes.push(
        link(
            "fresh",
            words.t("Refresh snapshot", "刷新快照", "重新整理快照"),
            fresh.route(),
        )
        .into(),
    );
    Ok(super::view(
        words.t("Activity", "活动", "活動"),
        revision(&page.cursor),
        vec![Action {
            fields: fields.iter().map(|field| field.id.clone()).collect(),
            ..action("filter", words.t("Apply filters", "应用筛选", "套用篩選"))
        }],
        fields,
        column("root", nodes),
    ))
}

pub(super) fn groups(words: &Words, place: &Place, summary: &Summary) -> View {
    let tab = place.tab.as_deref().unwrap_or("providers");
    let (title, rows): (String, Vec<(String, String)>) = match tab {
        "models" => (
            words.t("By model", "按模型", "按模型"),
            summary
                .by_model
                .iter()
                .map(|row| {
                    (
                        row.model_id.clone(),
                        format!(
                            "{} · {} · {}",
                            cost(words, &row.totals),
                            row.totals.calls,
                            tokens(words, &row.totals)
                        ),
                    )
                })
                .collect(),
        ),
        "tools" => (
            words.t("By tool", "按工具", "按工具"),
            summary
                .by_tool
                .iter()
                .map(|row| {
                    (
                        row.name.clone(),
                        words.t(
                            &format!("{} calls · {} failed", row.totals.calls, row.totals.error),
                            &format!("{} 次调用 · {} 次失败", row.totals.calls, row.totals.error),
                            &format!("{} 次呼叫 · {} 次失敗", row.totals.calls, row.totals.error),
                        ),
                    )
                })
                .collect(),
        ),
        _ => (
            words.t("By provider", "按提供商", "按提供者"),
            summary
                .by_provider
                .iter()
                .map(|row| {
                    (
                        row.provider_id
                            .clone()
                            .unwrap_or_else(|| words.t("Unknown", "未知", "未知")),
                        format!(
                            "{} · {} · {}",
                            cost(words, &row.totals),
                            row.totals.calls,
                            tokens(words, &row.totals)
                        ),
                    )
                })
                .collect(),
        ),
    };
    let start = place.page.min(rows.len().saturating_sub(1) / WINDOW) * WINDOW;
    let mut nodes = vec![
        link(
            "overview",
            words.t("Usage", "用量", "用量"),
            Place {
                tab: Some("overview".into()),
                page: 0,
                ..place.clone()
            }
            .route(),
        )
        .into(),
        heading("title", &title),
    ];
    for (index, (name, detail)) in rows.iter().skip(start).take(WINDOW).enumerate() {
        nodes.push(stack(
            format!("group-{index}"),
            vec![
                text("name", clean(name, false), Tone::Normal),
                text("totals", detail, Tone::Subtle),
            ],
        ));
    }
    nodes.extend(page_links(words, place, rows.len()));
    super::view(
        title,
        revision(place.cursor.as_deref().unwrap_or("summary")),
        vec![],
        vec![],
        column("root", nodes),
    )
}
