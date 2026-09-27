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

fn route(node: &Node, key: &str) -> Option<Value> {
    if let Node::Item {
        key: own,
        target: view::Target::Route { route },
        ..
    } = node
        && own == key
    {
        return Some(route.clone());
    }
    node.children()
        .into_iter()
        .find_map(|node| route(node, key))
}

fn attempts(count: usize) -> maka_plugins::usage::Page {
    serde_json::from_value(json!({"cursor":"fixed-snapshot","nextCursor":"older-snapshot","total":count + 1,
        "attempts":(0..count).map(|index| json!({"kind":"model","attempt":{
            "requestId":format!("request-{index}"),"origin":{"kind":"auxiliary","source":{"kind":"host_effect","id":uuid::Uuid::nil()}},
            "sessionId":"source","binding":null,"modelId":format!("model-{index}"),"startedAt":1000,"completedAt":2000,
            "outcome":"success","usage":{"input_tokens":7,"output_tokens":2,"cache_read_tokens":null,"cache_write_tokens":null,"reasoning_tokens":null},
            "quote":null,"costUsd":null}})).collect::<Vec<_>>()})).unwrap()
}

#[test]
fn activity_windows_keep_the_snapshot_and_every_exact_record_reachable() {
    let page = attempts(37);
    for locale in ["en", "zh-CN", "zh-TW"] {
        let words = Words::new(locale);
        let mut place = Place {
            tab: Some("activity".into()),
            cursor: Some(page.cursor.clone()),
            ..Default::default()
        };
        let mut seen = BTreeSet::new();
        loop {
            let view = activity::view(&words, &place, &page).unwrap();
            view.validate().unwrap();
            for index in 0..8 {
                let Some(target) = route(&view.root, &format!("attempt-{index}")) else {
                    continue;
                };
                let detail: Place = serde_json::from_value(target).unwrap();
                assert_eq!(detail.cursor.as_deref(), Some("fixed-snapshot"));
                seen.insert(detail.attempt.clone().unwrap());
                let detail = activity::view(&words, &detail, &page).unwrap();
                detail.validate().unwrap();
                assert!(
                    serde_json::to_string(&detail)
                        .unwrap()
                        .contains("1970-01-01T00:00:02Z")
                );
            }
            let Some(next) = route(&view.root, "next") else {
                let older: Place =
                    serde_json::from_value(route(&view.root, "older").unwrap()).unwrap();
                assert_eq!(older.cursor.as_deref(), Some("older-snapshot"));
                break;
            };
            place = serde_json::from_value(next).unwrap();
        }
        assert_eq!(seen.len(), 37);
    }
}

#[test]
fn long_filters_and_cursors_use_bytes_without_hiding_following_records() {
    let mut page = attempts(17);
    page.cursor = "x".repeat(3200);
    let mut place = Place {
        tab: Some("activity".into()),
        cursor: Some(page.cursor.clone()),
        selection: maka_plugins::usage::Selection {
            search: "\\".repeat(1024),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut seen = BTreeSet::new();
    loop {
        let view = activity::view(&Words::new("en"), &place, &page).unwrap();
        view.validate().unwrap();
        for index in 0..8 {
            if let Some(target) = route(&view.root, &format!("attempt-{index}")) {
                seen.insert(target["attempt"].as_str().unwrap().to_owned());
            }
        }
        let Some(next) = route(&view.root, "next") else {
            break;
        };
        place = serde_json::from_value(next).unwrap();
    }
    assert_eq!(seen.len(), 17);
}

#[test]
fn list_filters_refine_the_fence_but_scope_changes_start_a_new_snapshot() {
    let old = Place {
        tab: Some("activity".into()),
        cursor: Some("original".into()),
        range: Some("7d".into()),
        ..Default::default()
    };
    let submit = |session: &str| Submission {
        route: old.route(),
        revision: "snapshot".into(),
        action: "filter".into(),
        grant: None,
        fields: [
            ("range".into(), json!("7d")),
            ("session".into(), json!(session)),
            ("kind".into(), json!("model")),
            ("status".into(), json!("error")),
            ("search".into(), json!("family")),
        ]
        .into(),
    };
    let Reply::Applied { route: filtered } = activity::filter(submit("")).unwrap() else {
        panic!("applied")
    };
    let filtered: Place = serde_json::from_value(filtered).unwrap();
    assert!(filtered.refine);
    assert_eq!(filtered.cursor.as_deref(), Some("original"));
    let Reply::Applied { route: scoped } = activity::filter(submit("session-a")).unwrap() else {
        panic!("applied")
    };
    let scoped: Place = serde_json::from_value(scoped).unwrap();
    assert!(!scoped.refine && scoped.cursor.is_none());
    assert_eq!(scoped.session.as_deref(), Some("session-a"));
}

#[test]
fn prices_after_both_window_and_domain_boundaries_keep_revision_and_identity() {
    let entries: Vec<_> = (0..31)
        .map(|index| Entry::Custom {
            pricing: Pricing {
                model_key: format!("vendor:model-{index}"),
                input: index as f64,
                output: 4.0,
                cache_read: Some(0.5),
                cache_write: None,
            },
        })
        .collect();
    for locale in ["en", "zh-CN", "zh-TW"] {
        let words = Words::new(locale);
        let mut page = 0;
        let mut seen = BTreeSet::new();
        loop {
            let view = pricing(
                &words,
                42,
                &entries,
                Some(256),
                &Place {
                    offset: Some(128),
                    page,
                    ..Default::default()
                },
            );
            view.validate().unwrap();
            for index in 0..31 {
                let Some(target) = route(&view.root, &price_key(&format!("vendor:model-{index}")))
                else {
                    continue;
                };
                let target: Place = serde_json::from_value(target).unwrap();
                assert_eq!((target.offset, target.revision), (Some(128), Some(42)));
                let model = target.model.unwrap();
                seen.insert(model.clone());
                let entry = entries
                    .iter()
                    .find(|entry| entry.pricing().model_key == model);
                let detail = price(&words, 42, &model, entry);
                assert!(detail.action("reset").is_some());
                assert!(
                    matches!(&detail.field("input").unwrap().control, view::Control::Text { value, .. } if value == &index.to_string())
                );
            }
            let Some(next) = route(&view.root, "more-local") else {
                assert_eq!(route(&view.root, "more").unwrap()["offset"], 256);
                break;
            };
            page = serde_json::from_value::<Place>(next).unwrap().page;
        }
        assert_eq!(seen.len(), 31);
    }
}

#[test]
fn grouped_accounting_keeps_every_provider_model_and_tool_under_one_fence() {
    let totals = json!({"calls":1,"error":0,"input":{"known":7,"missing":0},"output":{"known":2,"missing":0},"cacheRead":{"known":0,"missing":0},"cost":{"knownUsd":0.1,"unvalued":0}});
    let summary: Summary = serde_json::from_value(json!({"models":totals,"tools":{"calls":31,"error":0},
        "byProvider":(0..31).map(|index| json!({"providerId":format!("provider-{index}"),"totals":totals})).collect::<Vec<_>>(),
        "byModel":(0..31).map(|index| json!({"modelId":format!("model-{index}"),"totals":totals})).collect::<Vec<_>>(),
        "byTool":(0..31).map(|index| json!({"name":format!("tool-{index}"),"totals":{"calls":1,"error":0}})).collect::<Vec<_>>()
    })).unwrap();
    fn names(node: &Node, seen: &mut BTreeSet<String>) {
        if let Node::Text { key, spans, .. } = node
            && key == "name"
        {
            seen.insert(spans.iter().map(|span| span.text.as_str()).collect());
        }
        for child in node.children() {
            names(child, seen);
        }
    }
    for locale in ["en", "zh-CN", "zh-TW"] {
        for tab in ["providers", "models", "tools"] {
            let mut place = Place {
                tab: Some(tab.into()),
                cursor: Some("fixed-groups".into()),
                ..Default::default()
            };
            let mut seen = BTreeSet::new();
            loop {
                let view = activity::groups(&Words::new(locale), &place, &summary);
                view.validate().unwrap();
                names(&view.root, &mut seen);
                let Some(next) = route(&view.root, "next") else {
                    break;
                };
                place = serde_json::from_value(next).unwrap();
                assert_eq!(place.cursor.as_deref(), Some("fixed-groups"));
            }
            assert_eq!(seen.len(), 31);
        }
    }
}
