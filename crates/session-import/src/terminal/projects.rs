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
use maka_plugins::remote::projects::{Output, Query};

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Selection {
    pub id: String,
    pub name: String,
    revision: String,
    page: Query,
}

pub(super) fn start() -> Query {
    Query::Start
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Location {
    source: uuid::Uuid,
    entry: Entry,
    models: ModelQuery,
    #[serde(default = "start")]
    pub page: Query,
    #[serde(default)]
    offset: usize,
    revision: Option<String>,
}

fn selected(selection: &Selection, result: &Output) -> bool {
    matches!(result, Output::Page { revision, projects, .. } if revision == &selection.revision
        && projects.iter().any(|project| project.available && project.id == selection.id && project.name == selection.name))
}

pub(super) async fn current(caller: &Caller, selection: &Selection) -> Result<bool, Error> {
    Ok(selected(
        selection,
        &caller.views.projects(selection.page.clone()).await?,
    ))
}

pub(super) fn changed(view: &mut View, words: &Words) {
    if let Some(import) = view.actions.iter_mut().find(|action| action.id == "import") {
        import.enabled = false;
    }
    if let Node::Column { children, .. } = &mut view.root {
        children.insert(0, text("project-changed", words.t(
            "The selected project changed or is unavailable. Choose it again before importing.",
            "所选项目已变化或不可用，请重新选择后导入。",
            "所選專案已變更或無法使用，請重新選擇後匯入。"), Tone::Warning));
    }
}

pub(super) fn view(words: &Words, sources: &Sources, location: &Location, result: Output) -> View {
    let Location {
        source,
        entry,
        models,
        page: query,
        offset,
        revision: expected,
    } = location;
    let expected = expected.as_deref();
    let offset = *offset;
    let route = |page: Query, offset: usize, revision: Option<&str>| json!({"kind":"projects","source":source,"entry":entry,"models":models,"page":page,"offset":offset,"revision":revision});
    let mut nodes = vec![
        link(
            "back",
            words.t("Back to import", "返回导入", "返回匯入"),
            json!({"kind":"import","source":source,"entry":entry,"models":models}),
        )
        .into(),
    ];
    let result = match result {
        Output::Page { ref revision, .. }
            if expected.is_some_and(|expected| expected != revision) =>
        {
            Output::Changed {
                revision: revision.clone(),
            }
        }
        result => result,
    };
    match result {
        Output::Changed { .. } => {
            nodes.push(text(
                "changed",
                words.t(
                    "Projects changed. Reload the list before choosing.",
                    "项目已变化，请重新加载列表后选择。",
                    "專案已變更，請重新載入清單後選擇。",
                ),
                Tone::Warning,
            ));
            nodes.push(
                link(
                    "reload",
                    words.t("Reload projects", "重新加载项目", "重新載入專案"),
                    route(Query::Start, 0, None),
                )
                .into(),
            );
        }
        Output::Page {
            revision,
            projects,
            next_cursor,
        } => {
            if projects.is_empty() {
                nodes.push(text(
                    "empty",
                    words.t("No projects available.", "没有可用项目。", "沒有可用專案。"),
                    Tone::Muted,
                ));
            }
            let count = projects.len();
            let offset = offset.min(count);
            let route_bytes = serde_json::to_vec(&route(query.clone(), offset, Some(&revision)))
                .expect("project route")
                .len();
            let budget = view::MAX_BYTES
                .saturating_sub(route_bytes.saturating_mul(3) + 4096)
                .min(24 * 1024);
            let mut bytes = 0;
            let mut shown = 0;
            for project in projects.into_iter().skip(offset).take(12) {
                let key = format!(
                    "project-{}",
                    maka_runtime::artifact::content_digest(project.id.as_bytes())
                );
                let node: Node = if !project.available {
                    stack(
                        key,
                        vec![
                            text("name", clean(&project.name), Tone::Muted),
                            text(
                                "unavailable",
                                words.t("Unavailable", "不可用", "無法使用"),
                                Tone::Warning,
                            ),
                        ],
                    )
                } else {
                    let mut target = models.clone();
                    target.project = Some(Selection {
                        id: project.id.clone(),
                        name: project.name.clone(),
                        revision: revision.clone(),
                        page: query.clone(),
                    });
                    link(
                        key,
                        choice_label(&project.name),
                        json!({"kind":"import","source":source,"entry":entry,"models":target}),
                    )
                    .detail(project.id)
                    .into()
                };
                let size = serde_json::to_vec(&node).expect("project choice").len();
                if shown > 0 && bytes + size > budget {
                    break;
                }
                bytes += size;
                shown += 1;
                nodes.push(node);
            }
            if offset + shown < count {
                nodes.push(
                    link(
                        "more",
                        words.t("More projects", "更多项目", "更多專案"),
                        route(query.clone(), offset + shown, Some(&revision)),
                    )
                    .into(),
                );
            } else if let Some(cursor) = next_cursor {
                nodes.push(
                    link(
                        "more",
                        words.t("More projects", "更多项目", "更多專案"),
                        route(
                            Query::Continue {
                                revision: revision.clone(),
                                cursor,
                            },
                            0,
                            Some(&revision),
                        ),
                    )
                    .into(),
                );
            }
            if !matches!(query, Query::Start) || offset > 0 {
                nodes.push(
                    link(
                        "first",
                        words.t("First projects", "项目首页", "專案首頁"),
                        route(Query::Start, 0, None),
                    )
                    .into(),
                );
            }
        }
    }
    view_of(
        words.t("Choose a project", "选择项目", "選擇專案"),
        sources,
        vec![],
        vec![],
        column("root", nodes),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use maka_plugins::remote::projects::Project;

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
    fn named_project_pages_keep_drafts_exact_identity_and_unavailability() {
        let sources = Sources {
            revision: Some(1),
            configuration: Configuration::default(),
        };
        let entry = Entry {
            id: "old".into(),
            path: "sessions/old.jsonl".into(),
            title: "Old conversation".into(),
            cwd: None,
            archived: false,
        };
        let models = ModelQuery {
            destination: Some("/chosen".into()),
            selected_model: Some("chosen-model".into()),
            sandbox: Some("read-only".into()),
            ..Default::default()
        };
        let query = Query::Continue {
            revision: "f".repeat(64),
            cursor: "project-31".into(),
        };
        let page = Output::Page {
            revision: "f".repeat(64),
            projects: vec![
                Project {
                    id: "project-32".into(),
                    name: "Same name".into(),
                    available: true,
                },
                Project {
                    id: "project-33".into(),
                    name: "Same name".into(),
                    available: false,
                },
            ],
            next_cursor: Some("project-33".into()),
        };
        for locale in ["en", "zh-CN", "zh-TW"] {
            let words = Words::new(locale);
            let view = view(
                &words,
                &sources,
                &Location {
                    source: uuid::Uuid::nil(),
                    entry: entry.clone(),
                    models: models.clone(),
                    page: query.clone(),
                    offset: 0,
                    revision: None,
                },
                page.clone(),
            );
            view.validate().unwrap();
            let chosen = route(
                &view.root,
                &format!(
                    "project-{}",
                    maka_runtime::artifact::content_digest(b"project-32")
                ),
            )
            .unwrap();
            let Route::Import { models: chosen, .. } = serde_json::from_value(chosen).unwrap()
            else {
                panic!("import")
            };
            assert_eq!(chosen.destination.as_deref(), Some("/chosen"));
            assert_eq!(chosen.selected_model.as_deref(), Some("chosen-model"));
            let selection = chosen.project.unwrap();
            assert_eq!(selection.id, "project-32");
            assert!(selected(&selection, &page));
            assert!(!selected(
                &selection,
                &Output::Changed {
                    revision: "g".repeat(64)
                }
            ));
            assert!(
                route(
                    &view.root,
                    &format!(
                        "project-{}",
                        maka_runtime::artifact::content_digest(b"project-33")
                    )
                )
                .is_none()
            );
            let Route::Projects(next) =
                serde_json::from_value(route(&view.root, "more").unwrap()).unwrap()
            else {
                panic!("continuation")
            };
            assert_eq!(
                next.page,
                Query::Continue {
                    revision: "f".repeat(64),
                    cursor: "project-33".into()
                }
            );
            assert_eq!(next.models.selected_model.as_deref(), Some("chosen-model"));
        }
    }

    #[test]
    fn project_windows_follow_complete_host_pages_and_expose_changed_refresh() {
        let sources = Sources {
            revision: Some(1),
            configuration: Configuration::default(),
        };
        let entry = Entry {
            id: "old".into(),
            path: "sessions/old.jsonl".into(),
            title: "Old".into(),
            cwd: None,
            archived: false,
        };
        let projects: Vec<_> = (0..69)
            .map(|index| Project {
                id: format!("project-{index:03}"),
                name: format!("Project {index} 中文"),
                available: index != 3,
            })
            .collect();
        for locale in ["en", "zh-CN", "zh-TW"] {
            let words = Words::new(locale);
            let mut location = Location {
                source: uuid::Uuid::nil(),
                entry: entry.clone(),
                models: ModelQuery::default(),
                page: Query::Start,
                offset: 0,
                revision: None,
            };
            let mut seen = std::collections::BTreeSet::new();
            loop {
                let start = match &location.page {
                    Query::Start => 0,
                    Query::Continue { cursor, .. } => {
                        projects
                            .iter()
                            .position(|project| &project.id == cursor)
                            .unwrap()
                            + 1
                    }
                };
                let end = (start + 32).min(projects.len());
                let output = Output::Page {
                    revision: "f".repeat(64),
                    projects: projects[start..end].to_vec(),
                    next_cursor: (end < projects.len()).then(|| projects[end - 1].id.clone()),
                };
                let shown = view(&words, &sources, &location, output);
                shown.validate().unwrap();
                for project in &projects {
                    let key = format!(
                        "project-{}",
                        maka_runtime::artifact::content_digest(project.id.as_bytes())
                    );
                    if let Some(target) = route(&shown.root, &key) {
                        let Route::Import { models, .. } = serde_json::from_value(target).unwrap()
                        else {
                            panic!("selected project")
                        };
                        assert!(project.available);
                        seen.insert(models.project.unwrap().id);
                    }
                }
                let Some(next) = route(&shown.root, "more") else {
                    break;
                };
                let Route::Projects(next) = serde_json::from_value(next).unwrap() else {
                    panic!("project page")
                };
                location = next;
                let changed = view(
                    &words,
                    &sources,
                    &location,
                    Output::Changed {
                        revision: "a".repeat(64),
                    },
                );
                changed.validate().unwrap();
                let Route::Projects(reload) =
                    serde_json::from_value(route(&changed.root, "reload").unwrap()).unwrap()
                else {
                    panic!("reload")
                };
                assert_eq!(reload.page, Query::Start);
                assert!(reload.revision.is_none());
            }
            assert_eq!(seen.len(), 68);
        }
    }
}
