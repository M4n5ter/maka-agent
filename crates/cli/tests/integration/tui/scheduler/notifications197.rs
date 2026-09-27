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

const TITLE: &str = "Native reminder 197";
const BODY: &str = "Native notification body 197.\nPresented by the actual TUI.";

fn badge(screen: &str, text: &str) -> Option<usize> {
    let header = screen.lines().next()?;
    Some(header[..header.find(text)?].width())
}

// The Task behind this sheet repeats its title, body and Back action. Only
// text inside the rendered Notifications border may identify an inbox control.
fn notifications(screen: &str) -> Option<Vec<(usize, usize, &str)>> {
    let lines: Vec<_> = screen.lines().collect();
    let (row, left, right) = lines.iter().enumerate().find_map(|(row, line)| {
        let title = line.find("Notifications")?;
        let left = line[..title].rfind('│')?;
        let right = title + line[title..].find('│')?;
        (line[left + '│'.len_utf8()..right].trim() == "Notifications").then_some((
            row,
            line[..left].width(),
            line[..right].width(),
        ))
    })?;
    let byte_at = |line: &str, col| {
        line.char_indices()
            .find_map(|(byte, _)| (line[..byte].width() == col).then_some(byte))
    };
    let top = lines.get(row.checked_sub(2)?)?;
    let start = byte_at(top, left)?;
    let end = byte_at(top, right)?;
    // Sheet has one padding row between its top border and title.
    if !top[start..].starts_with('╭')
        || !top[end..].starts_with('╮')
        || !top[start + '╭'.len_utf8()..end].chars().all(|ch| ch == '─')
    {
        return None;
    }
    let mut rows = Vec::new();
    for (row, line) in lines.iter().enumerate().skip(row) {
        let start = byte_at(line, left)?;
        let end = byte_at(line, right)?;
        if line[start..].starts_with('╰') && line[end..].starts_with('╯') {
            return Some(rows);
        }
        if !line[start..].starts_with('│') || !line[end..].starts_with('│') {
            return None;
        }
        rows.push((row, left + 1, &line[start + '│'.len_utf8()..end]));
    }
    None
}

fn notifications_show(screen: &str, texts: &[&str]) -> bool {
    notifications(screen).is_some_and(|rows| {
        texts
            .iter()
            .all(|text| rows.iter().any(|(_, _, line)| line.contains(*text)))
    })
}

fn click_notification(tui: &mut Pty, text: &str) {
    tui.wait_until(|screen| notifications_show(screen, &[text]));
    let snapshot = tui.screen.snapshot().unwrap();
    let (row, col) = notifications(&snapshot.screen)
        .unwrap()
        .into_iter()
        .find_map(|(row, col, line)| {
            line.find(text)
                .map(|byte| (row, col + line[..byte].width()))
        })
        .unwrap();
    tui.click_at(row, col);
}

#[test]
fn local_reminder_reaches_native_badge_inbox_and_delivered_history_without_palette() {
    let directory = tempfile::tempdir().unwrap();
    let mut host =
        super::super::super::candidate::CandidateFixture::new(directory.path().join("root"));
    host.child = Some(
        Command::new(env!("CARGO_BIN_EXE_maka"))
            .args(["host", "serve", "--root"])
            .arg(&host.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    host.wait_for_registration();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    // This observer publishes no presentation service. Only the real TUI's
    // Client NativeServices can receive and acknowledge the notification.
    let observer = runtime.block_on(support::client(&host.root));
    let mut tui = Pty::spawn(&["--root", host.root.to_str().unwrap()]);
    tui.wait_for("Scheduled tasks");
    tui.click_text("Scheduled tasks");
    tui.wait_for("New task");
    tui.click_text("New task");
    tui.wait_for("Local notification");
    tui.click_text("Local notification");
    tui.wait_for("Content");
    tui.click_text("Title");
    tui.send(format!("\x1b[200~{TITLE}\x1b[201~").as_bytes());
    tui.click_text("Content");
    tui.send(format!("\x1b[200~{BODY}\x1b[201~").as_bytes());
    tui.click_text("Create task");
    tui.wait_for("Allow plugin access?");
    tui.click_text("Allow and continue");
    tui.wait_until(|screen| {
        screen.contains(TITLE) && screen.contains("Run now") && badge(screen, "●0").is_some()
    });
    let list = runtime.block_on(remote(
        &observer,
        "request",
        json!({"kind":"query","query":{"kind":"list"}}),
    ));
    let tasks = list["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1, "the page created exactly one reminder");
    let created = &tasks[0];
    assert_eq!(created["title"], TITLE);
    assert_eq!(created["intent"]["body"], BODY);
    assert_eq!(
        created["effect"],
        json!({"kind":"notify","channel":"local"})
    );
    assert_eq!(
        created["fireCount"], 0,
        "the default future schedule has not run"
    );
    let id = created["id"].as_str().unwrap().to_owned();
    let query = || json!({"kind":"query","query":{"kind":"get","taskId":id}});
    tui.click_text("Run now");
    // These are committed renderer frames. Merely accepting/queuing a Client
    // capability offer cannot satisfy either the badge or the scheduler result.
    tui.wait_until(|screen| badge(screen, "●1").is_some() && screen.contains("Completed"));
    let delivered = runtime.block_on(remote(&observer, "request", query()))["task"].clone();
    assert_eq!(delivered["fireCount"], 1);
    assert_eq!(delivered["status"], "completed");
    assert!(delivered["lastError"].is_null());
    let runs = delivered["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["outcome"], "ok");
    assert_eq!(runs[0]["message"], "Notification sent");
    assert!(runs[0]["sessionId"].is_null() && runs[0]["runId"].is_null());
    let fire_id = runs[0]["id"].as_str().unwrap().to_owned();

    // Use the visible badge text and actual PTY geometry, never a hover hint.
    let col = badge(&tui.screen.snapshot().unwrap().screen, "●1").unwrap();
    tui.click_at(0, col);
    tui.wait_until(|screen| {
        notifications_show(screen, &[TITLE, "maka.scheduler", "Mark read", "Close"])
    });
    click_notification(&mut tui, TITLE);
    tui.wait_until(|screen| {
        notifications_show(
            screen,
            &[
                TITLE,
                "maka.scheduler",
                "Native notification body 197.",
                "Presented by the actual TUI.",
                "Back",
            ],
        )
    });
    click_notification(&mut tui, "Back");
    tui.wait_until(|screen| {
        notifications_show(screen, &[TITLE, "maka.scheduler", "Mark read", "Dismiss"])
            && badge(screen, "●0").is_some()
    });
    click_notification(&mut tui, "Dismiss");
    tui.wait_until(|screen| notifications_show(screen, &["No notifications.", "Close"]));
    tui.resize(55, 24);
    tui.wait_until(|screen| {
        notifications_show(screen, &["No notifications.", "Close"]) && badge(screen, "●0").is_some()
    });
    click_notification(&mut tui, "Close");
    tui.wait_until(|screen| notifications(screen).is_none() && badge(screen, "●0").is_some());
    let col = badge(&tui.screen.snapshot().unwrap().screen, "●0").unwrap();
    tui.click_at(0, col);
    tui.wait_until(|screen| notifications_show(screen, &["No notifications.", "Close"]));
    click_notification(&mut tui, "Close");
    tui.wait_until(|screen| notifications(screen).is_none());
    tui.resize(120, 40);
    tui.wait_until(|screen| screen.contains("Completed") && screen.contains("Runs and history"));
    tui.click_text("Runs and history");
    tui.wait_for("Retained history: 1");
    tui.wait_for("Notification sent");
    tui.click_text("Notification sent");
    tui.wait_for("Fire ID");
    tui.wait_for(&fire_id);
    assert!(
        !tui.screen
            .snapshot()
            .unwrap()
            .screen
            .contains("Search commands")
    );
    assert_eq!(
        runtime.block_on(remote(&observer, "request", query()))["task"],
        delivered,
        "reading and dismissing local attention never runs or rewrites the task"
    );

    tui.close_terminal();
    tui.finish();
    runtime.block_on(async {
        remote(
            &observer,
            "request",
            json!({"kind":"mutate","mutation":{"kind":"delete","taskId":id}}),
        )
        .await;
        let remaining = remote(
            &observer,
            "request",
            json!({"kind":"query","query":{"kind":"list"}}),
        )
        .await;
        assert!(remaining["tasks"].as_array().unwrap().is_empty());
    });
    observer.disconnect();
    host.retire_registered();
    assert!(host.wait_for_exit().success());
}
