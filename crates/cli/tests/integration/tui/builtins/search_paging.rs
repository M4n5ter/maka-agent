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
use std::collections::BTreeSet;

#[test]
fn real_host_model_search_and_normal_import_reach_past_two_domain_pages() {
    let fixture = Fixture::new();
    super::import_recovery::models(&fixture, 110);
    super::import_recovery::source(&fixture, 1);
    let search = |query: &str, cursor: Value| {
        fixture.read(
            "maka.session-import",
            "manage",
            json!({"kind":"models","query":{"query":query,"cursor":cursor}}),
        )["choices"]
            .clone()
    };
    let first = search("", Value::Null);
    assert_eq!(first["kind"], "page");
    assert_eq!(first["page"]["models"].as_array().unwrap().len(), 50);
    let cursor = first["page"]["nextCursor"].clone();
    assert_eq!(search("choice-1", cursor.clone())["kind"], "stale");
    for field in ["configurationRevision", "providerRevision"] {
        let mut stale = cursor.clone();
        stale[field] = json!(stale[field].as_u64().unwrap() + 1);
        assert_eq!(search("", stale)["kind"], "stale");
    }
    let mut foreign = cursor.clone();
    foreign["generation"] = json!(uuid::Uuid::new_v4());
    assert_eq!(search("", foreign)["kind"], "stale");
    let mut next = Value::Null;
    let mut names = BTreeSet::new();
    loop {
        let result = search("", next.clone());
        assert_eq!(result["kind"], "page");
        for model in result["page"]["models"].as_array().unwrap() {
            assert!(names.insert(model["model"]["model"].as_str().unwrap().to_owned()));
        }
        let more = result["page"]["nextCursor"].clone();
        if more.is_null() {
            assert_eq!(result["page"]["complete"], true);
            break;
        }
        assert!(more["offset"].as_u64().unwrap() > next["offset"].as_u64().unwrap_or(0));
        next = more;
    }
    assert_eq!(names.len(), 110);
    let mut tui = fixture.tui();
    category(&mut tui, "Conversation import", "Recovery source");
    tui.click_page_text("Recovery source");
    tui.wait_for("Recovery case 01");
    tui.click_page_text("Recovery case 01");
    tui.wait_for("choice-40");
    for expected in ["choice-31", "choice-50", "choice-82", "choice-100"] {
        tui.click_last_text("More models");
        tui.wait_for(expected);
    }
    tui.click_last_text("Import");
    tui.wait_for("Import history");
    let copied = fixture.read(
        "maka.session-import",
        "manage",
        json!({"kind":"copies","after":null}),
    );
    let session = copied["page"]["copies"][0]["receipt"]["sessionId"]
        .as_str()
        .unwrap();
    assert_eq!(
        fixture
            .runtime
            .block_on(fixture.client.session(session))
            .unwrap()
            .unwrap()
            .model,
        "choice-100"
    );
    fixture.finish(tui);
}

#[test]
fn archived_codex_conversation_import_is_available_from_its_normal_page() {
    let fixture = Fixture::new();
    let source = fixture.directory.path().join("archived-codex");
    std::fs::create_dir_all(source.join("archived_sessions")).unwrap();
    std::fs::create_dir_all(source.join("sessions")).unwrap();
    let path = source.join("archived_sessions/rollout-archived-page.jsonl");
    let text = [
        json!({"type":"session_meta","payload":{"id":"archived-page","cwd":fixture.directory.path(),"source":"cli"}}),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"Archived normal page case"}}),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"Archived historical result"}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(&path, &text).unwrap();
    let mut tui = fixture.tui();
    category(&mut tui, "Conversation import", "Add a source");
    tui.click_page_text("Add a source");
    tui.wait_for("Location");
    edit(&mut tui, "Name", "Archived Codex", "Location");
    edit(&mut tui, "Location", source.to_str().unwrap(), "Location");
    tui.click_page_text("Save");
    tui.wait_for("Add a source");
    tui.click_page_text("Archived Codex");
    tui.wait_for("No conversations found.");
    tui.click_page_text("Include archived");
    tui.click_page_text("Search");
    tui.wait_for("Archived normal page case");
    tui.click_page_text("Archived normal page case");
    tui.wait_for("Folder");
    tui.click_last_text("Import");
    tui.wait_for("Import history");
    let copied = fixture.read(
        "maka.session-import",
        "manage",
        json!({"kind":"copies","after":null}),
    );
    assert_eq!(
        copied["page"]["copies"][0]["sourceSessionId"],
        "archived-page"
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    fixture.finish(tui);
}

#[test]
fn normal_import_selects_a_named_project_and_preserves_its_host_identity() {
    use maka_protocol::project::Mutation;
    let fixture = Fixture::new();
    super::import_recovery::source(&fixture, 1);
    let directory = fixture.directory.path().join("named-project");
    std::fs::create_dir(&directory).unwrap();
    let project = fixture.runtime.block_on(async {
        let project = fixture
            .client
            .mutate_project(Mutation::Register {
                path: directory.to_str().unwrap().into(),
                prefer: Some(true),
            })
            .await
            .unwrap();
        fixture
            .client
            .mutate_project(Mutation::Rename {
                project_id: project.id,
                name: "Named import destination".into(),
            })
            .await
            .unwrap()
    });
    let mut tui = fixture.tui();
    category(&mut tui, "Conversation import", "Recovery source");
    tui.click_page_text("Recovery source");
    tui.wait_for("Recovery case 01");
    tui.click_page_text("Recovery case 01");
    tui.wait_for("Choose a project");
    tui.click_page_text("Choose a project");
    tui.wait_for("Named import destination");
    tui.click_page_text("Named import destination");
    tui.wait_for("Change project");
    tui.click_last_text("Import");
    tui.wait_for("Import history");
    let copied = fixture.read(
        "maka.session-import",
        "manage",
        json!({"kind":"copies","after":null}),
    );
    let imported = copied["page"]["copies"][0]["receipt"]["sessionId"]
        .as_str()
        .unwrap();
    let session = fixture
        .runtime
        .block_on(fixture.client.session(imported))
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&session).unwrap()["workspace"]["target"],
        json!({"kind":"project","projectId":project.id})
    );
    assert_eq!(
        session.workspace.host_cwd,
        directory.canonicalize().unwrap().to_str().unwrap()
    );
    fixture.finish(tui);
}
