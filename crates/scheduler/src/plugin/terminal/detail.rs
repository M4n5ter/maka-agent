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

pub(super) fn page(task: Task, revision: u64, locale: &str) -> Page {
    let mut page = empty(Text::plain(display(&task.title, 256, false)), revision);
    page.rows.push(Row {
        id: "schedule".into(),
        title: timing::title(&task.schedule),
        description: String::new(),
        route: serde_json::to_value(Route {
            task: Some(task.id.clone()),
            timing: true,
            ..Route::default()
        })
        .expect("task route"),
    });
    page.body = format!(
        "{}  {}",
        status(task.status).resolve(locale),
        instant(task.next_fire_at)
    );
    page.body.push_str(&format!(
        "\n{}\n{}: {} · {}: {}",
        target::summary(&task.effect, locale),
        Text::localized("Runs", "运行次数", "執行次數").resolve(locale),
        task.fire_count,
        Text::localized("Maximum", "最多", "上限").resolve(locale),
        task.max_fires
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".into())
    ));
    if let Some(expiry) = task.expires_at {
        page.body.push_str(&format!(
            "\n{}: {}",
            Text::localized("Expiry", "到期时间", "到期時間").resolve(locale),
            instant(Some(expiry))
        ));
    }
    for (id, title, route) in [
        (
            "target",
            Text::localized("Change type or target", "更改类型或目标", "變更類型或目標"),
            super::Route {
                task: Some(task.id.clone()),
                target: Some(target::Route::Pick),
                ..Default::default()
            },
        ),
        (
            "limits",
            Text::localized("Run limits", "运行限制", "執行限制"),
            super::Route {
                task: Some(task.id.clone()),
                settings: Some(settings::Kind::Limits),
                ..Default::default()
            },
        ),
    ] {
        if matches!(task.status, Status::Active | Status::Paused) {
            page.rows.push(Row {
                id: id.into(),
                title,
                description: String::new(),
                route: serde_json::to_value(route).expect("task settings route"),
            });
        }
    }
    page.rows.push(Row {
        id: "history".into(),
        title: Text::localized("Runs and history", "运行与历史", "執行與歷史"),
        description: String::new(),
        route: history::route(
            &task.id,
            history::Route::List {
                offset: 0,
                revision: None,
            },
        ),
    });
    if task.last_error.is_some() {
        page.rows.push(Row {
            id: "error".into(),
            title: Text::localized("Last error", "最近错误", "最近錯誤"),
            description: String::new(),
            route: history::route(&task.id, history::Route::Error),
        });
    }
    if task.status == Status::Active {
        page.rows.push(Row {
            id: "snooze".into(),
            title: Text::localized("Snooze…", "推迟…", "延後…"),
            description: String::new(),
            route: serde_json::to_value(super::Route {
                task: Some(task.id.clone()),
                settings: Some(settings::Kind::Snooze),
                ..Default::default()
            })
            .expect("snooze route"),
        });
    }
    let Intent::Text { body } = task.intent;
    // Never save a sanitized or truncated field over the original task content.
    let editable = matches!(task.status, Status::Active | Status::Paused)
        && body.len() <= INTENT_BYTES
        && display(&body, INTENT_BYTES, true) == body
        && display(&task.title, 512, false) == task.title;
    if editable {
        page.fields = vec![
            field(
                "title",
                Text::localized("Title", "标题", "標題"),
                task.title,
                512,
                false,
            ),
            field(
                "intent",
                Text::localized("Content", "内容", "內容"),
                body,
                INTENT_BYTES,
                true,
            ),
        ];
        page.actions.push(Action {
            id: "save".into(),
            label: Text::localized("Save", "保存", "儲存"),
            enabled: true,
            fields: vec!["title".into(), "intent".into()],
            recovery: None,
            confirm: None,
        });
    } else {
        page.body.push_str("\n\n");
        page.body
            .push_str(&display(&body, 32 * 1024 - page.body.len(), true));
    }
    let action = match task.status {
        Status::Active => Some(("pause", Text::localized("Pause", "暂停", "暫停"))),
        Status::Paused => Some(("resume", Text::localized("Resume", "恢复", "恢復"))),
        _ => None,
    };
    if let Some((id, label)) = action {
        page.actions.push(Action {
            id: id.into(),
            label,
            enabled: true,
            fields: vec![],
            recovery: None,
            confirm: None,
        });
    }
    // What a task can still do, and removing it, which asks first.
    for (id, label, offered, confirm) in [
        (
            "trigger",
            Text::localized("Run now", "立即运行", "立即執行"),
            task.status == Status::Active,
            None,
        ),
        (
            "delete",
            Text::localized("Delete", "删除", "刪除"),
            true,
            Some(maka_plugins::terminal_ui::view::Confirm {
                title: Text::localized("Delete this task?", "删除此任务？", "刪除此任務？")
                    .resolve(locale)
                    .into(),
                message: Text::localized(
                    "It stops running and its history goes with it.",
                    "任务将停止运行，其历史记录也将删除。",
                    "任務將停止執行，其歷史紀錄也將刪除。",
                )
                .resolve(locale)
                .into(),
                destructive: true,
            }),
        ),
    ] {
        if offered {
            page.actions.push(Action {
                id: id.into(),
                label,
                enabled: true,
                fields: vec![],
                recovery: None,
                confirm,
            });
        }
    }
    page
}
