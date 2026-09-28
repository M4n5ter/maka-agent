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
use crate::session::{PreparedSession, SessionModel};
use maka_protocol::session::{WorkspaceProjection, WorkspaceTarget};
use serde_json::json;

fn scope(workspace: &Path, parent: &Path) -> Scope {
    let parent = parent.canonicalize().unwrap();
    let cwd = maka_fs_tools::workspace::project::host_path(&workspace.canonicalize().unwrap())
        .unwrap()
        .to_owned();
    let configuration = PreparedSession::new(serde_json::from_value(json!({
        "sessionId":"session","workspace":{"kind":"host_path","path":cwd},"modelTarget":{"kind":"default"}
    })).unwrap()).unwrap().bind(
        WorkspaceProjection { target: WorkspaceTarget::HostPath { path: cwd.clone() }, host_cwd: cwd },
        SessionModel { connection_id: "fixture".into(), connection_slug: "fixture".into(), model: "fixture".into() },
        SandboxMode::DangerFullAccess);
    Scope::open(
        "root".into(),
        "session".into(),
        configuration,
        parent.join("state"),
        parent.join("control"),
        CancellationToken::new(),
    )
    .unwrap()
}
fn query(cursor: Option<api::Cursor>) -> api::Query {
    api::Query {
        session_id: "session".into(),
        directory: String::new(),
        filter: String::new(),
        cursor,
    }
}

#[test]
fn workspace_pages_exhaust_names_and_capture_real_unicode_context() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    for index in 0..301 {
        std::fs::write(
            workspace.join(format!("source-{index:03}.txt")),
            "真实 content 🦀",
        )
        .unwrap();
    }
    let mut cursor = None;
    let mut names = Vec::new();
    let basis = loop {
        let page = scope(&workspace, directory.path())
            .query(query(cursor))
            .unwrap();
        api::decode_page(&serde_json::to_value(&page).unwrap()).unwrap();
        names.extend(page.entries.iter().map(|entry| entry.path.clone()));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break page.basis;
        }
    };
    assert_eq!(names.len(), 301);
    assert_eq!(names.first().unwrap(), "source-000.txt");
    assert_eq!(names.last().unwrap(), "source-300.txt");
    let captured = scope(&workspace, directory.path())
        .capture(api::Capture {
            basis,
            path: "source-300.txt".into(),
            kind: api::Kind::File,
        })
        .unwrap();
    api::decode_captured(&serde_json::to_value(&captured).unwrap()).unwrap();
    assert!(captured.quote.text.contains("真实 content 🦀"));
    assert!(!captured.truncated);
}

#[test]
fn inventory_changes_invalidate_continuations_and_captures_are_explicitly_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    for index in 0..150 {
        std::fs::write(workspace.join(format!("item-{index:03}")), "x").unwrap();
    }
    let page = scope(&workspace, directory.path())
        .query(query(None))
        .unwrap();
    std::fs::write(workspace.join("added"), "🦀".repeat(api::CAPTURE_BYTES)).unwrap();
    assert_eq!(
        scope(&workspace, directory.path())
            .query(query(page.next_cursor))
            .unwrap_err()
            .code,
        Code::CandidateSetStale
    );
    let capture = scope(&workspace, directory.path())
        .capture(api::Capture {
            basis: page.basis,
            path: "added".into(),
            kind: api::Kind::File,
        })
        .unwrap();
    assert!(capture.truncated);
    assert!(capture.quote.text.len() < 32_000);
    api::decode_captured(&serde_json::to_value(&capture).unwrap()).unwrap();
    let cancelled = scope(&workspace, directory.path());
    cancelled.cancellation.cancel();
    assert!(cancelled.query(query(None)).is_err());
}

#[test]
fn unrestricted_sessions_still_cannot_capture_host_state_or_control_directories() {
    let parent = tempfile::tempdir().unwrap();
    for name in ["state", "control", "visible"] {
        std::fs::create_dir(parent.path().join(name)).unwrap();
        std::fs::write(parent.path().join(name).join("file"), "private data").unwrap();
    }
    let page = scope(parent.path(), parent.path())
        .query(query(None))
        .unwrap();
    assert_eq!(
        page.entries
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        ["visible"]
    );
    for path in ["state/file", "control/file"] {
        assert!(
            scope(parent.path(), parent.path())
                .capture(api::Capture {
                    basis: page.basis.clone(),
                    path: path.into(),
                    kind: api::Kind::File
                })
                .is_err()
        );
    }
    let captured = scope(parent.path(), parent.path())
        .capture(api::Capture {
            basis: page.basis,
            path: "visible".into(),
            kind: api::Kind::Directory,
        })
        .unwrap();
    assert!(captured.quote.text.contains("visible/file"));
    assert_eq!(captured.directory_reference.unwrap().host_id, "root");
}

// macOS filesystems reject invalid UTF-8 names at creation.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn non_utf8_names_do_not_hide_valid_files_or_create_lossy_aliases() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(
        workspace.join(OsString::from_vec(b"unaddressable-\xff.txt".to_vec())),
        "invalid-name content",
    )
    .unwrap();
    std::fs::write(workspace.join("valid.txt"), "valid content").unwrap();
    std::fs::write(workspace.join("unaddressable-�.txt"), "exact UTF-8 file").unwrap();
    let page = scope(&workspace, directory.path())
        .query(query(None))
        .unwrap();
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["unaddressable-�.txt", "valid.txt"]
    );
    let mut filtered = query(None);
    filtered.filter = "valid".into();
    assert_eq!(
        scope(&workspace, directory.path())
            .query(filtered)
            .unwrap()
            .entries,
        [api::Entry {
            path: "valid.txt".into(),
            kind: api::Kind::File
        }]
    );
    for (path, expected) in [
        ("valid.txt", "valid content"),
        ("unaddressable-�.txt", "exact UTF-8 file"),
    ] {
        let capture = scope(&workspace, directory.path())
            .capture(api::Capture {
                basis: page.basis.clone(),
                path: path.into(),
                kind: api::Kind::File,
            })
            .unwrap();
        assert!(capture.quote.text.contains(expected));
        assert!(!capture.quote.text.contains("invalid-name content"));
    }
    let captured = scope(&workspace, directory.path())
        .capture(api::Capture {
            basis: page.basis,
            path: String::new(),
            kind: api::Kind::Directory,
        })
        .unwrap();
    assert_eq!(
        captured.quote.text.lines().skip(1).collect::<Vec<_>>(),
        ["unaddressable-�.txt", "valid.txt"]
    );
    assert!(!captured.truncated);
}

#[cfg(unix)]
#[test]
fn links_special_nodes_and_replaced_workspace_never_escape_the_read_scope() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(directory.path().join("outside"), "PRIVATE").unwrap();
    std::os::unix::fs::symlink(directory.path().join("outside"), workspace.join("link")).unwrap();
    std::fs::write(workspace.join("binary"), [0, 1, 2]).unwrap();
    let page = scope(&workspace, directory.path())
        .query(query(None))
        .unwrap();
    assert!(!page.entries.iter().any(|entry| entry.path == "link"));
    for path in ["link", "binary"] {
        assert!(
            scope(&workspace, directory.path())
                .capture(api::Capture {
                    basis: page.basis.clone(),
                    path: path.into(),
                    kind: api::Kind::File
                })
                .is_err()
        );
    }
    let held = scope(&workspace, directory.path());
    std::fs::rename(&workspace, directory.path().join("old")).unwrap();
    std::fs::create_dir(&workspace).unwrap();
    assert_eq!(
        held.query(query(None)).unwrap_err().code,
        Code::CandidateSetStale
    );
    assert_eq!(
        scope(&workspace, directory.path())
            .capture(api::Capture {
                basis: page.basis,
                path: "binary".into(),
                kind: api::Kind::File
            })
            .unwrap_err()
            .code,
        Code::CandidateSetStale
    );
}
