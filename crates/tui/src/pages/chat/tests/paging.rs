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
use ratatui::{Terminal, backend::TestBackend};

fn setup() -> (Chat, Terminal<TestBackend>) {
    let mut chat = Chat::default();
    chat.select(&Route::Session("a".into()));
    let request = chat.open_query().unwrap();
    let mut initial = opened();
    initial.batch = batch(100, 180, 180);
    initial.batch.next_cursor = Some("older".into());
    initial
        .snapshot
        .transcript
        .as_mut()
        .unwrap()
        .durable
        .through_sequence = Some(180);
    chat.opened(request, Ok(initial));
    let mut screen = Terminal::new(TestBackend::new(80, 20)).unwrap();
    draw(&mut chat, &mut screen);
    (chat, screen)
}
fn draw(chat: &mut Chat, screen: &mut Terminal<TestBackend>) {
    let i18n = I18n::new(
        crate::LocalePreference::Explicit(crate::Locale::En),
        crate::Locale::En,
    );
    screen
        .draw(|f| {
            chat.draw(f, f.area(), &i18n, false);
        })
        .unwrap();
}
fn approach_start(chat: &mut Chat, screen: &mut Terminal<TestBackend>) -> PageRequest {
    for _ in 0..200 {
        chat.scroll(true, 3);
        draw(chat, screen);
        if let Some(request) = chat.page_query() {
            assert!(request.prefetch);
            assert!(
                !chat.view.at_top(),
                "read ahead before hitting the boundary"
            );
            return request;
        }
    }
    panic!("never prefetched");
}

#[test]
fn prefetch_is_directional_bounded_and_preserves_the_visible_anchor() {
    let (mut chat, mut screen) = setup();
    assert!(
        chat.page_query().is_none(),
        "idle readers don't scan history"
    );
    let request = approach_start(&mut chat, &mut screen);
    assert!(chat.page_query().is_none(), "only one read in flight");
    let anchor = chat.view.first_visible();
    let mut page = batch(80, 99, 180);
    page.next_cursor = Some("even-older".into());
    chat.page(request, Ok(page));
    draw(&mut chat, &mut screen);
    assert_eq!(chat.view.first_visible(), anchor);
    assert!(
        chat.page_query().is_none(),
        "completion doesn't trigger speculative scans"
    );
    assert!(chat.rows.contains_key(&80));

    chat.reading_history = true;
    chat.wanted = Some(300);
    chat.view.latest();
    draw(&mut chat, &mut screen);
    chat.scroll(true, 6);
    draw(&mut chat, &mut screen);
    chat.scroll(false, 1);
    let request = chat.page_query().unwrap();
    assert!(request.prefetch);
    assert_eq!(
        request.input.direction,
        SessionTranscriptPageDirection::Newer
    );
    let anchor = chat.view.first_visible();
    let mut page = batch(181, 195, 300);
    page.next_cursor = Some("more-newer".into());
    chat.page(request, Ok(page));
    draw(&mut chat, &mut screen);
    assert_eq!(chat.view.first_visible(), anchor);
    assert_eq!(chat.through, Some(195));
    assert!(chat.page_query().is_none());
}

#[test]
fn oversized_prefetch_waits_at_most_one_page_without_evicting_or_refetching() {
    let (mut chat, mut screen) = setup();
    let request = approach_start(&mut chat, &mut screen);
    let anchor = chat.view.first_visible();
    let mut page = batch(99, 99, 180);
    page.rows[0].value["text"] = json!("x".repeat(WINDOW_BYTES));
    page.next_cursor = Some("more".into());
    chat.page(request, Ok(page));
    draw(&mut chat, &mut screen);
    assert_eq!(chat.view.first_visible(), anchor);
    assert!(chat.prefetched.is_some());
    assert!(!chat.rows.contains_key(&99));
    for _ in 0..3 {
        chat.scroll(true, 1);
        assert!(chat.page_query().is_none());
    }
    chat.scroll(true, usize::MAX);
    assert!(chat.prefetched.is_none());
    assert!(chat.rows.contains_key(&99));
    assert!(
        chat.page_query().is_none(),
        "edge input consumes the already read page"
    );
}

#[test]
fn arriving_prefetch_at_the_edge_is_applied_immediately_and_latest_cancels_it() {
    let (mut chat, mut screen) = setup();
    let request = approach_start(&mut chat, &mut screen);
    chat.scroll(true, usize::MAX);
    let mut page = batch(99, 99, 180);
    page.rows[0].value["text"] = json!("x".repeat(WINDOW_BYTES));
    chat.page(request, Ok(page));
    assert!(chat.rows.contains_key(&99));
    assert!(chat.prefetched.is_none());

    let (mut chat, mut screen) = setup();
    let request = approach_start(&mut chat, &mut screen);
    chat.latest();
    chat.page(request, Ok(batch(99, 99, 180)));
    assert!(
        !chat.rows.contains_key(&99),
        "latest supersedes the in-flight lookahead"
    );
    assert!(chat.page_query().unwrap().tail);

    let (mut chat, mut screen) = setup();
    let request = approach_start(&mut chat, &mut screen);
    let mut page = batch(99, 99, 180);
    page.rows[0].value["text"] = json!("x".repeat(WINDOW_BYTES));
    chat.page(request, Ok(page));
    assert!(chat.prefetched.is_some());
    chat.latest();
    assert!(chat.prefetched.is_none());
    assert!(chat.view.following());
}

#[test]
fn inline_media_does_not_evict_text_or_schedule_context_diagnostics() {
    let (mut chat, _) = setup();
    let request = chat.context_query().unwrap();
    chat.context_completed(request, Err("not needed".into()));
    let mut page = batch(99, 99, 180);
    page.rows[0].value = json!({"type":"tool_result","id":"image","turnId":"turn","content":{"kind":"json","value":{"content":[{"type":"image","mimeType":"image/png","data":"A".repeat(WINDOW_BYTES * 2)}]}}});
    chat.merge(media::prepare(page), SessionTranscriptPageDirection::Older)
        .unwrap();
    assert_eq!(chat.rows.len(), 82);
    assert!(chat.bytes < 100_000);
    assert!(
        chat.context_query().is_none(),
        "reading history doesn't change model context"
    );
}
