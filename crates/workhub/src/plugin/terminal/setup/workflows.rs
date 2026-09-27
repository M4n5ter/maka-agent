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

fn model(index: usize) -> llm::Choice {
    llm::Choice {
        model: maka_runtime::execution::ModelBinding {
            connection_id: format!("connection-{index}"),
            connection_slug: "local".into(),
            model: format!("family-{index}"),
        },
        connection_name: "Same connection label".into(),
        display_name: "Same model label".into(),
        thinking_levels: vec![ThinkingLevel::Low, ThinkingLevel::High],
        default_thinking_level: Some(ThinkingLevel::Low),
        is_default: index == 0,
    }
}

fn targets(node: &Node, seen: &mut BTreeSet<String>) {
    if let Node::Item {
        target: view::Target::Route { route },
        ..
    } = node
    {
        let place: Place = serde_json::from_value(route.clone()).unwrap();
        if let Some(target) = place.setup.and_then(|setup| setup.target) {
            seen.insert(serde_json::to_string(&target).unwrap());
        }
    }
    for child in node.children() {
        targets(child, seen);
    }
}

#[test]
fn every_returned_model_and_executor_is_reachable_without_query_narrowing() {
    for locale in ["en", "zh-CN", "zh-TW"] {
        let mut seen = BTreeSet::new();
        for page in 0..5 {
            let current = Route {
                purpose: Purpose::Creation,
                query: "family".into(),
                page,
                models_cursor: None,
                executors_cursor: None,
                target: None,
            };
            let view = picker(
                &Words::new(locale),
                &current,
                llm::Choices {
                    revision: 4,
                    complete: true,
                    next_cursor: None,
                    models: (0..50).map(model).collect(),
                },
                Some(executor::Choices {
                    revision: 5,
                    complete: true,
                    next_cursor: None,
                    executors: (0..50)
                        .map(|index| executor::Choice {
                            id: format!("family.agent-{index}").try_into().unwrap(),
                            display_name: "Same executor label".into(),
                            capabilities: Default::default(),
                        })
                        .collect(),
                }),
            );
            view.validate().unwrap();
            targets(&view.root, &mut seen);
        }
        assert_eq!(seen.len(), 100);
    }
}

#[test]
fn thinking_choices_follow_the_model_and_keep_an_explicit_default() {
    let choice = model(0);
    for locale in ["en", "zh-CN", "zh-TW"] {
        let current = Route {
            purpose: Purpose::Coordinator,
            query: String::new(),
            page: 0,
            models_cursor: None,
            executors_cursor: None,
            target: Some(Target::Model {
                model: choice.model.clone(),
                thinking_level: Some(ThinkingLevel::High),
            }),
        };
        let view = form(&Words::new(locale), &current, None, None, Some(&choice)).unwrap();
        view.validate().unwrap();
        let view::Control::Choice { value, options } = &view.field("thinking").unwrap().control
        else {
            panic!("thinking choices")
        };
        assert_eq!(value, "high");
        assert_eq!(
            options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["default", "low", "high"]
        );
        assert!(
            view.action("save-target")
                .unwrap()
                .fields
                .contains(&"thinking".into())
        );
    }
}
