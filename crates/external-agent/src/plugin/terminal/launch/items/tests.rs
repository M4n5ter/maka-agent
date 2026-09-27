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
mod fixture;
use fixture::*;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn public_app_reads_and_edits_the_maximum_valid_command_without_truncation() {
    let mut original = agent();
    let empty_bytes = serde_json::to_vec(&(&original.executable, [""], &original.env))
        .unwrap()
        .len();
    let remaining = 65_536 - empty_bytes;
    original.args = vec![format!(
        "{}{}",
        "\"".repeat(remaining / 2),
        if !remaining.is_multiple_of(2) {
            "x"
        } else {
            ""
        }
    )];
    assert_eq!(
        serde_json::to_vec(&(&original.executable, &original.args, &original.env))
            .unwrap()
            .len(),
        65_536
    );
    let fixture = Fixture::new(original.clone());
    let editor = fixture.read(json!({"agent":"fixture"})).await;
    let advance_route = destination(&editor, "advanced-args");
    let advanced = fixture.read(advance_route).await;
    let list_route = destination(&advanced, "individual");
    let list = fixture.read(list_route).await;
    let mut route = destination(&list, "item-0");
    let first = fixture.read(route.clone()).await;
    let mut assembled = String::new();
    loop {
        let shown = fixture.read(route.clone()).await;
        let fragment: String = serde_json::from_str(&text_field(&shown, "fragment")).unwrap();
        assembled.push_str(&fragment);
        if assembled.len() == original.args[0].len() {
            break;
        }
        route = destination(&shown, "next-fragment");
    }
    assert_eq!(assembled, original.args[0]);
    let route = destination(&list, "item-0");
    let replacement = "reviewed\nfragment\t";
    let next = applied(
        fixture
            .submit(
                &first,
                route,
                "launch-item-save",
                [("fragment".into(), json!(document(&replacement)))].into(),
            )
            .await
            .unwrap(),
    );
    let expected = format!("{replacement}{}", &original.args[0][CHUNK..]);
    assert_eq!(fixture.stored().args, [expected]);
    assert_eq!(fixture.backend.writes.load(Ordering::SeqCst), 1);
    fixture.read(next).await;
}

#[tokio::test]
async fn public_app_keeps_utf8_fragments_and_pages_every_variable_then_appends_and_deletes() {
    let mut original = agent();
    original.args = vec![String::new(); 19];
    let large_name = format!("LONG{}", "🌍\n".repeat(5000));
    original
        .env
        .insert(large_name.clone(), "line\n\u{202e}\tend".into());
    let fixture = Fixture::new(original.clone());
    let route = json!({"agent":"fixture","launch":"env","items":true});
    let list = fixture.read(route.clone()).await;
    let item_route = destination(&list, "item-0");
    let detail = fixture.read(item_route).await;
    let mut name_route = destination(&detail, "component");
    let mut name = String::new();
    loop {
        let shown = fixture.read(name_route.clone()).await;
        let fragment: String = serde_json::from_str(&text_field(&shown, "fragment")).unwrap();
        name.push_str(&fragment);
        if name.len() == large_name.len() {
            break;
        }
        name_route = destination(&shown, "next-fragment");
    }
    assert_eq!(name, large_name);
    let appended = applied(
        fixture
            .submit(
                &list,
                route,
                "launch-item-append",
                [
                    ("new-name".into(), json!("\"EMPTY\"")),
                    ("new-value".into(), json!("\"\"")),
                ]
                .into(),
            )
            .await
            .unwrap(),
    );
    assert_eq!(fixture.stored().env["EMPTY"], "");
    let shown = fixture.read(appended.clone()).await;
    assert!(
        shown
            .action("launch-item-remove")
            .unwrap()
            .confirm
            .is_some()
    );
    let listing = applied(
        fixture
            .submit(&shown, appended, "launch-item-remove", BTreeMap::new())
            .await
            .unwrap(),
    );
    fixture.read(listing).await;
    assert_eq!(fixture.stored(), original);
    let list = fixture
        .read(json!({"agent":"fixture","launch":"env","items":true}))
        .await;
    let value = fixture.read(destination(&list, "item-0")).await;
    let name_route = destination(&value, "component");
    let shown = fixture.read(name_route.clone()).await;
    let end = bounds(&large_name)[0].1;
    let updated_name = format!("RENAMED{}", &large_name[end..]);
    let next = applied(
        fixture
            .submit(
                &shown,
                name_route,
                "launch-item-save",
                [("fragment".into(), json!("\"RENAMED\""))].into(),
            )
            .await
            .unwrap(),
    );
    fixture.read(next).await;
    assert!(!fixture.stored().env.contains_key(&large_name));
    assert_eq!(
        fixture.stored().env[&updated_name],
        original.env[&large_name]
    );
    let list = fixture
        .read(json!({"agent":"fixture","launch":"args","items":true}))
        .await;
    let second = fixture.read(destination(&list, "next")).await;
    let last = fixture.read(destination(&second, "item-18")).await;
    assert_eq!(
        serde_json::from_str::<String>(&text_field(&last, "fragment")).unwrap(),
        ""
    );
}

#[tokio::test]
async fn public_app_rejects_stale_revision_and_reordered_values_and_preserves_unknown_writes() {
    let mut original = agent();
    original.args = vec!["first".into(), "second".into()];
    let fixture = Fixture::new(original);
    let list = fixture
        .read(json!({"agent":"fixture","launch":"args","items":true}))
        .await;
    let route = destination(&list, "item-0");
    let first = fixture.read(route.clone()).await;
    // Even a backend whose revision did not advance cannot retarget an old row.
    fixture.backend.state.lock().unwrap().1[0].args.swap(0, 1);
    let rejected = fixture
        .submit(
            &first,
            route.clone(),
            "launch-item-save",
            [("fragment".into(), json!("\"wrong\""))].into(),
        )
        .await
        .unwrap();
    assert!(matches!(rejected, Reply::Conflict));
    assert_eq!(fixture.backend.writes.load(Ordering::SeqCst), 0);
    let stale = fixture.read(route).await;
    assert!(stale.action("launch-item-save").is_none());
    let list = fixture
        .read(json!({"agent":"fixture","launch":"args","items":true}))
        .await;
    let route = destination(&list, "item-0");
    let current = fixture.read(route.clone()).await;
    fixture.backend.state.lock().unwrap().0 += 1;
    assert!(matches!(
        fixture
            .submit(&current, route, "launch-item-remove", BTreeMap::new())
            .await
            .unwrap(),
        Reply::Conflict
    ));
    let list = fixture
        .read(json!({"agent":"fixture","launch":"args","items":true}))
        .await;
    let route = destination(&list, "item-0");
    let current = fixture.read(route.clone()).await;
    fixture.backend.unknown.store(true, Ordering::SeqCst);
    let result = fixture
        .submit(
            &current,
            route,
            "launch-item-save",
            [("fragment".into(), json!("\"saved once\""))].into(),
        )
        .await;
    assert!(matches!(result, Err(Error::OutcomeUnknown(_))));
    assert_eq!(fixture.stored().args[0], "saved once");
    assert_eq!(fixture.backend.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn public_app_pages_all_environment_entries_and_can_add_an_empty_argument() {
    let mut original = agent();
    original.env = (0..128)
        .map(|index| (format!("K{index:03}"), format!("value-{index}")))
        .collect();
    let fixture = Fixture::new(original);
    let mut route = json!({"agent":"fixture","launch":"env","items":true});
    for page in 0..8 {
        let shown = fixture.read(route).await;
        for index in page * PAGE..(page + 1) * PAGE {
            let detail = fixture
                .read(destination(&shown, &format!("item-{index}")))
                .await;
            assert_eq!(
                serde_json::from_str::<String>(&text_field(&detail, "fragment")).unwrap(),
                format!("value-{index}")
            );
        }
        if page < 7 {
            route = destination(&shown, "next");
        } else {
            break;
        }
    }
    let route = json!({"agent":"fixture","launch":"args","items":true});
    let shown = fixture.read(route.clone()).await;
    let appended = applied(
        fixture
            .submit(
                &shown,
                route,
                "launch-item-append",
                [("new-value".into(), json!("\"\""))].into(),
            )
            .await
            .unwrap(),
    );
    assert_eq!(fixture.stored().args, [""]);
    let shown = fixture.read(appended.clone()).await;
    fixture
        .submit(&shown, appended, "launch-item-remove", BTreeMap::new())
        .await
        .unwrap();
    assert!(fixture.stored().args.is_empty());
    assert_eq!(fixture.stored().env.len(), 128);
}

mod path;

mod auth_composition;
