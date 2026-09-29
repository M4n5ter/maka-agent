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
use maka_runtime::display::Text;
use ratatui::{Terminal, backend::TestBackend};

fn running() -> Chat {
    let mut chat = Chat::default();
    chat.select(&Route::Session("a".into()));
    let request = chat.open_query().unwrap();
    let mut initial = opened();
    initial.snapshot.snapshot.root_turn = Some(
        maka_protocol::turn::decode_turn_snapshot(
            &json!({"sessionId":"a","turnId":"turn","runId":"run","status":"running"}),
        )
        .unwrap(),
    );
    chat.opened(request, Ok(initial));
    chat
}
fn event(chat: &mut Chat, run: &str, event: Value) {
    chat.accept(ObservationFrame::Tool(
        decode_tool_observation_frame(&json!({
            "kind":"subscription.session_event","hostEpoch":"epoch","subscriptionId":"sub",
            "sequence":1,"sessionId":"a","runId":run,"event":event,
        }))
        .unwrap(),
    ))
    .unwrap();
}
fn start(id: &str) -> Value {
    json!({"type":"tool_start","id":id,"turnId":"turn","ts":1,"toolUseId":id,
        "toolName":"plugin_action", "title":Text::localized("Inspect Calculator","查看计算器","查看計算機")})
}
fn screen(chat: &mut Chat, locale: crate::Locale) -> String {
    let i18n = I18n::new(crate::LocalePreference::Explicit(locale), locale);
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal
        .draw(|frame| {
            chat.draw(frame, frame.area(), &i18n, false);
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
        .replace(' ', "")
}

#[test]
fn tool_activity_survives_live_to_durable_replay_without_duplicate_cards_or_losing_errors() {
    let mut chat = running();
    event(&mut chat, "run", start("call"));
    let text = screen(&mut chat, crate::Locale::En);
    assert_eq!(text.matches("InspectCalculator").count(), 1);
    assert!(text.contains("Running"));
    event(
        &mut chat,
        "run",
        json!({"type":"tool_progress","id":"p","turnId":"turn","ts":2,"toolUseId":"call","chunk":"Reading controls"}),
    );
    chat.cadence.flush();
    assert!(screen(&mut chat, crate::Locale::En).contains("Readingcontrols"));
    let title = Text::localized("Inspect Calculator", "查看计算器", "查看計算機");
    chat.fixture_rows(BTreeMap::from([
        (1,json!({"type":"tool_call","id":"call","turnId":"turn","toolName":"plugin_action","args":{"title":"untrusted argument title","code":"do_not_show_as_headline()"},"origin":"provider"})),
        (2,json!({"type":"tool_activity","id":"activity","turnId":"turn","toolUseId":"call","toolName":"plugin_action","title":title})),
    ]));
    let text = screen(&mut chat, crate::Locale::En);
    assert_eq!(text.matches("InspectCalculator").count(), 1);
    assert!(!text.contains("do_not_show_as_headline"));
    event(
        &mut chat,
        "run",
        json!({"type":"tool_result","id":"done","turnId":"turn","ts":3,"toolUseId":"call","status":"errored"}),
    );
    chat.rows.insert(3,json!({"type":"tool_result","id":"result","turnId":"turn","toolUseId":"call","isError":true,"content":{"kind":"text","text":"Accessibility permission denied"},"origin":"provider"}));
    chat.dirty = true;
    let text = screen(&mut chat, crate::Locale::En);
    assert!(text.contains("Accessibilitypermissiondenied"));
    assert!(!text.contains("Running"));
    assert!(
        chat.presentation.activity.entries.is_empty(),
        "durable rows replace transient activity"
    );
    chat.presentation = Default::default(); // Fresh reader reconstructs titles from durable rows.
    chat.invalidate_layout();
    let text = screen(&mut chat, crate::Locale::ZhCn);
    assert_eq!(text.matches("查看计算器").count(), 1);
    assert!(text.contains("Accessibilitypermissiondenied"));
}

#[test]
fn activity_is_bounded_and_does_not_leak_across_history_runs_or_disconnects() {
    let mut chat = running();
    event(&mut chat, "old-run", start("wrong"));
    assert!(chat.presentation.activity.entries.is_empty());
    event(&mut chat, "run", start("call"));
    chat.reading_history = true;
    assert!(!screen(&mut chat, crate::Locale::En).contains("InspectCalculator"));
    chat.reading_history = false;
    for n in 0..300 {
        let id = format!("extra-{n}");
        event(&mut chat, "run", start(&id));
        event(
            &mut chat,
            "run",
            json!({"type":"tool_result","id":format!("done-{n}"),"turnId":"turn","ts":3,"toolUseId":id,"status":"completed"}),
        );
    }
    assert!(chat.presentation.activity.entries.len() <= 256);
    let mut snapshot = chat.snapshot.clone().unwrap();
    snapshot.root_turn = Some(maka_protocol::turn::decode_turn_snapshot(&json!({"sessionId":"a","turnId":"turn","runId":"run","status":"cancelled","terminalEventId":"end","abortSource":"user"})).unwrap());
    chat.accept(ObservationFrame::Projection(Box::new(
        SessionProjectionFrame::SessionProjection {
            host_epoch: "epoch".into(),
            subscription_id: "sub".into(),
            sequence: 2,
            snapshot,
        },
    )))
    .unwrap();
    assert!(!screen(&mut chat, crate::Locale::En).contains("Running"));
    chat.disconnect("lost connection".into());
    assert!(chat.presentation.activity.entries.is_empty());
    chat.select(&Route::Session("other".into()));
    assert!(chat.presentation.activity.entries.is_empty());
}
