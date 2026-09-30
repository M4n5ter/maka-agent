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

use super::{MAX_MESSAGE, RenderState, RenderStatus, Request, Spec};
use cursor_overlay::{CursorConfig, OverlayCommand, ReducedMotion};
use std::{
    collections::HashMap,
    io::{BufRead, Read, Write},
    time::Duration,
};

#[cfg(target_os = "linux")]
use platform_linux::overlay as platform;
#[cfg(target_os = "macos")]
use platform_macos::cursor::overlay as platform;
#[cfg(target_os = "windows")]
use platform_windows::overlay as platform;

fn send(id: &str, command: OverlayCommand) {
    #[cfg(target_os = "linux")]
    platform::send_command_for(id.into(), command);
    #[cfg(not(target_os = "linux"))]
    platform::send_command(id.into(), command);
}
fn validate(cursor: &Spec, point: Option<[f64; 2]>) -> Result<(), String> {
    if uuid::Uuid::parse_str(&cursor.id).is_err()
        || cursor.label.chars().count() > 48
        || cursor.label.chars().any(char::is_control)
        || point.is_some_and(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0))
    {
        return Err("invalid cursor update".into());
    }
    Ok(())
}
struct Entry {
    spec: Spec,
    point: Option<[f64; 2]>,
}
fn active_theme(id: &str) -> Option<(String, Option<String>)> {
    #[cfg(target_os = "linux")]
    let state = platform::current_theme_state_for(id);
    #[cfg(not(target_os = "linux"))]
    let state = platform::current_theme_state(id);
    state.map(|(id, _, _, fallback, _)| (id, fallback))
}
fn status(id: &str, entries: &HashMap<String, Entry>) -> RenderState {
    let entry = entries.get(id);
    let visible = platform::is_visible_for_session(id);
    let theme = entry.and_then(|_| active_theme(id));
    RenderState {
        status: RenderStatus::Ready,
        visible,
        requested_position: entry.and_then(|e| e.point),
        theme: theme.as_ref().map(|(id, _)| id.clone()),
        renderer_pid: Some(std::process::id()),
        error: theme.and_then(|(_, fallback)| fallback),
    }
}
pub(super) fn run() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if !platform_macos::session::has_graphic_access() {
        return Err("cursor_unavailable: no interactive graphic session".into());
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return Err(
            "cursor_unavailable: native overlay requires X11; Wayland overlay is not enabled"
                .into(),
        );
    }
    platform::init(CursorConfig {
        motion: cursor_overlay::MotionConfig {
            // Visibility belongs to the Session, not to an individual action.
            idle_hide_ms: 0.0,
            ..Default::default()
        },
        ..Default::default()
    });
    #[cfg(not(target_os = "macos"))]
    platform::run_on_thread();
    let reader = || {
        let result = std::panic::catch_unwind(serve)
            .unwrap_or_else(|_| Err("cursor control worker panicked".into()));
        if let Err(error) = &result {
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                serde_json::to_string(&RenderState::unavailable(error)).unwrap()
            );
            let _ = writeln!(std::io::stderr(), "Maka cursor: {error}");
        }
        // This process owns only display surfaces. EOF removes every surface
        // even if its parent was killed before sending session retirement.
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    };
    #[cfg(target_os = "macos")]
    {
        std::thread::Builder::new()
            .name("maka-cursor-control".into())
            .spawn(reader)
            .map_err(|e| e.to_string())?;
        platform::run_on_main_thread();
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        reader()
    }
}
fn serve() -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let mut entries: HashMap<String, Entry> = HashMap::new();
    loop {
        let mut line = String::new();
        let count = reader
            .by_ref()
            .take(MAX_MESSAGE + 1)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(());
        }
        if count as u64 > MAX_MESSAGE {
            return Err("cursor request exceeds size limit".into());
        }
        let request: Request = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let response = match request {
            Request::State { id } => status(&id, &entries),
            Request::Remove { id } => {
                platform::remove_cursor(id.clone());
                entries.remove(&id);
                let end = std::time::Instant::now() + Duration::from_millis(300);
                while platform::is_visible_for_session(&id) && std::time::Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(8));
                }
                status(&id, &entries)
            }
            Request::Update {
                cursor,
                point,
                action,
                window,
            } => {
                validate(&cursor, point)?;
                if entries.len() >= 64 && !entries.contains_key(&cursor.id) {
                    return Err("cursor capacity reached".into());
                }
                let id = &cursor.id;
                let fresh = !entries.contains_key(id);
                if fresh {
                    platform::revive_cursor(id.clone());
                }
                send(
                    id,
                    OverlayCommand::SetTheme {
                        theme_id: cursor.theme_id(),
                        reduced_motion: if cursor.reduced_motion {
                            ReducedMotion::On
                        } else {
                            ReducedMotion::Auto
                        },
                    },
                );
                send(id, OverlayCommand::SetSessionLabel(cursor.label.clone()));
                if let Some(window) = window {
                    send(id, OverlayCommand::PinAbove(window));
                }
                if let Some([x, y]) = point {
                    if fresh || cursor.reduced_motion {
                        send(id, cursor_overlay::track_pointer_command(x, y));
                    } else {
                        send(
                            id,
                            OverlayCommand::MoveTo {
                                x,
                                y,
                                end_heading_radians: std::f64::consts::FRAC_PI_4,
                            },
                        );
                    }
                }
                send(
                    id,
                    OverlayCommand::BeginAction {
                        action,
                        delivery: None,
                        target: None,
                    },
                );
                send(id, OverlayCommand::SetEnabled(cursor.enabled));
                let previous = entries.get(id).and_then(|e| e.point);
                let id = id.clone();
                entries.insert(
                    id.clone(),
                    Entry {
                        spec: cursor,
                        point: point.or(previous),
                    },
                );
                let desired_theme = entries[&id].spec.theme_id();
                let end = std::time::Instant::now() + Duration::from_millis(400);
                while active_theme(&id).is_none_or(|(theme, _)| theme != desired_theme)
                    && std::time::Instant::now() < end
                {
                    std::thread::sleep(Duration::from_millis(8));
                }
                if active_theme(&id).is_none_or(|(theme, _)| theme != desired_theme) {
                    return Err("cursor renderer did not apply the requested theme".into());
                }
                // Configuration can create a cursor before it has a position.
                // Its first position needs the same visibility acknowledgement
                // as a newly created cursor.
                if point.is_some() && entries[&id].spec.enabled {
                    let end = std::time::Instant::now() + Duration::from_millis(400);
                    while !platform::is_visible_for_session(&id) && std::time::Instant::now() < end
                    {
                        std::thread::sleep(Duration::from_millis(8));
                    }
                }
                if !entries[&id].spec.enabled {
                    let end = std::time::Instant::now() + Duration::from_millis(300);
                    while platform::is_visible_for_session(&id) && std::time::Instant::now() < end {
                        std::thread::sleep(Duration::from_millis(8));
                    }
                }
                status(&id, &entries)
            }
        };
        serde_json::to_writer(&mut writer, &response).map_err(|e| e.to_string())?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|e| e.to_string())?;
    }
}
