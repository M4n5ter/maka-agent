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

#[test]
fn archived_continuations_and_project_destinations_keep_user_choices() {
    let sources = Sources {
        revision: Some(3),
        configuration: Configuration {
            sources: vec![Source {
                id: uuid::Uuid::nil(),
                name: "Codex".into(),
                location: Location::Codex {
                    root: "/synthetic".into(),
                },
            }],
        },
    };
    let entry = Entry {
        id: "archived-a".into(),
        path: "archived_sessions/a.jsonl".into(),
        title: "Archived conversation".into(),
        cwd: Some("/old".into()),
        archived: true,
    };
    let model = Choice {
        model: maka_runtime::execution::ModelBinding {
            connection_id: "connection-a".into(),
            connection_slug: "local".into(),
            model: "model".into(),
        },
        display_name: "Model".into(),
        connection_name: "Local".into(),
        thinking_levels: vec![],
        default_thinking_level: None,
        is_default: true,
    };
    let models = Choices {
        revision: 9,
        models: vec![model],
        complete: false,
        next_cursor: Some(maka_plugins::llm::Cursor {
            query: "model".into(),
            generation: uuid::Uuid::nil(),
            configuration_revision: 9,
            provider_revision: 4,
            offset: 50,
        }),
    };
    for locale in ["en", "zh-CN", "zh-TW"] {
        let words = Words::new(locale);
        let view = browse(
            &words,
            &sources,
            sources.configuration.sources.first(),
            "archive",
            true,
            Some(&Catalog {
                entries: vec![entry.clone()],
                next: Some("next-archive".into()),
            }),
        );
        view.validate().unwrap();
        assert!(matches!(
            view.field("archived").unwrap().control,
            view::Control::Toggle { value: true }
        ));
        let next = route(&view.root, "more").unwrap();
        assert_eq!(next["archived"], true);
        assert_eq!(next["cursor"], "next-archive");
        let query = ModelQuery {
            query: "model".into(),
            project: Some(serde_json::from_value(json!({"id":"project-a","name":"Chosen project","revision":"f".repeat(64),"page":{"kind":"start"}})).unwrap()),
            destination: Some("/chosen".into()),
            ..Default::default()
        };
        let view = import(&words, &sources, uuid::Uuid::nil(), &entry, &query, &models);
        view.validate().unwrap();
        for action in ["search-models", "models-more", "import"] {
            assert!(
                view.action(action)
                    .unwrap()
                    .fields
                    .contains(&"sandbox".into())
            );
            assert!(
                view.action(action)
                    .unwrap()
                    .fields
                    .contains(&"model".into())
            );
        }
        assert!(view.field("destination").is_none());
        assert_eq!(
            destination_target(&query, "").unwrap(),
            maka_runtime::execution::WorkspaceTarget::Project {
                project_id: "project-a".into()
            }
        );
    }
    assert!(destination_target(&ModelQuery::default(), " ").is_err());
    assert_eq!(
        destination_target(&ModelQuery::default(), "/chosen").unwrap(),
        maka_runtime::execution::WorkspaceTarget::HostPath {
            path: "/chosen".into()
        }
    );
}

#[test]
fn identically_named_models_from_different_connections_have_distinct_input_identity() {
    let mut model = Choice {
        model: maka_runtime::execution::ModelBinding {
            connection_id: "a".into(),
            connection_slug: "local".into(),
            model: "family".into(),
        },
        display_name: "Family".into(),
        connection_name: "Local".into(),
        thinking_levels: vec![],
        default_thinking_level: None,
        is_default: false,
    };
    let first = model_id(&model);
    model.model.connection_id = "b".into();
    assert_ne!(first, model_id(&model));
}
