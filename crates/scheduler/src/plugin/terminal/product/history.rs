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

use super::super::history as scheduler_history;
use super::*;
use crate::task::{Outcome, Run};
use maka_plugins::terminal_ui::view::{self, Node, Target};

fn history_targets(node: &Node, found: &mut Vec<Value>) {
    match node {
        Node::Column { children, .. } => {
            for child in children {
                history_targets(child, found);
            }
        }
        Node::Item {
            target: Target::Route { route },
            ..
        } => found.push(route.clone()),
        _ => {}
    }
}

#[test]
fn all_retained_runs_errors_and_exact_result_sessions_are_reachable() {
    let mut task = plan().task;
    task.fire_count = 20;
    task.last_error = Some("provider failed\nsecond line".into());
    task.runs = (0..20)
        .map(|index| Run {
            id: format!("fire-{index}"),
            at: 1000 + index,
            outcome: Outcome::Ok,
            message: format!("Execution admitted {index}"),
            session_id: Some(format!("result-session-{index}")),
            run_id: Some(format!("run-{index}")),
        })
        .collect();
    let mut ids = std::collections::BTreeSet::new();
    for offset in [0, 8, 16] {
        let view::Reply::View { view } = scheduler_history::read(
            &task,
            42,
            scheduler_history::Route::List {
                offset,
                revision: Some(42),
            },
            "en",
        )
        .unwrap() else {
            panic!("view")
        };
        view.validate().unwrap();
        let mut routes = vec![];
        history_targets(&view.root, &mut routes);
        for value in routes {
            let route: Route = serde_json::from_value(value).unwrap();
            if let Some(scheduler_history::Route::Run { id }) = route.history {
                ids.insert(id);
            }
        }
    }
    assert_eq!(ids.len(), 20);
    for locale in ["en", "zh-CN", "zh-TW"] {
        let view::Reply::View { view } = scheduler_history::read(
            &task,
            42,
            scheduler_history::Route::Run {
                id: "fire-19".into(),
            },
            locale,
        )
        .unwrap() else {
            panic!("view")
        };
        view.validate().unwrap();
        let Node::Column { children, .. } = view.root else {
            panic!("column")
        };
        assert!(
            matches!(children.last(), Some(Node::Item {target: Target::Session {session},..}) if session == "result-session-19")
        );
        let view::Reply::View { view } =
            scheduler_history::read(&task, 42, scheduler_history::Route::Error, locale).unwrap()
        else {
            panic!("view")
        };
        view.validate().unwrap();
        assert!(
            serde_json::to_string(&view)
                .unwrap()
                .contains("second line")
        );
    }
    assert!(matches!(
        scheduler_history::read(
            &task,
            43,
            scheduler_history::Route::List {
                offset: 8,
                revision: Some(42)
            },
            "en"
        )
        .unwrap(),
        view::Reply::Conflict
    ));
    assert!(matches!(
        scheduler_history::read(
            &task,
            42,
            scheduler_history::Route::Run {
                id: "retired-fire".into()
            },
            "en"
        )
        .unwrap(),
        view::Reply::Rejected { .. }
    ));
}
