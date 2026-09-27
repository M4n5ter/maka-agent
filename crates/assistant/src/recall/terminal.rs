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

//! Recall as an app: find what was said in earlier conversations, and
//! open the session each match came from.

mod filters;

use super::{Recall, remote::Search};
use filters::Filters;
use futures_util::future::BoxFuture;
use maka_plugins::{
    contributions::Staged,
    remote::{Error, Method},
    terminal_ui::{
        Context, Descriptor, Text, VERSION,
        app::{self, App, Cx, Submission, Words},
        view::{self, Action, Node, Reply, Role, Target, Tone, View, build::*},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) fn publish(
    staged: &mut Staged,
    package: &str,
    recall: Arc<Recall>,
) -> Result<(), String> {
    let endpoint = app::endpoint(
        Finder(Arc::new(Search(recall))),
        Descriptor::new(
            Text::localized("Recall", "回忆", "回憶"),
            Context::Application,
        )
        .icon("⌕", "R")
        .order(30),
    )
    .map_err(|error| error.to_string())?;
    staged
        .insert(
            maka_plugins::remote::key(package, "terminal").map_err(|error| error.to_string())?,
            endpoint,
        )
        .map_err(|error| error.to_string())
}

struct Finder(Arc<Search>);

#[derive(Deserialize)]
struct Found {
    matches: Vec<Match>,
    complete: bool,
}
#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum Match {
    Title {
        session_id: String,
        title: String,
    },
    Passage {
        session_id: String,
        title: String,
        text: String,
        truncated: bool,
        turn_id: String,
        message_id: String,
        sequence: u64,
    },
}

/// Words to look for: at most eight, each a whole word of the query.
fn terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .take(8)
        .map(str::to_owned)
        .collect()
}

impl App for Finder {
    fn read(&self, route: Value, cx: Cx) -> BoxFuture<'static, Result<View, Error>> {
        let search = self.0.clone();
        Box::pin(async move {
            let mut filters = Filters::read(&route)?;
            if let Some(session) = &cx.caller.session_id {
                filters.session = Some(filters::Selected {
                    id: session.clone(),
                    title: cx.t("Current conversation", "当前对话", "目前對話"),
                });
            }
            if route["pickSession"] == true {
                return filters::sessions(&search.0, &route, &filters, cx).await;
            }
            let found = if terms(&filters.query).is_empty() {
                None
            } else {
                let value = search
                    .call(
                        filters.request(cx.caller.session_id.as_deref())?,
                        cx.caller.clone(),
                    )
                    .await;
                Some(value.and_then(|value| {
                    serde_json::from_value::<Found>(value)
                        .map_err(|error| Error::Provider(error.to_string()))
                }))
            };
            Ok(finder(&cx.words, &filters, found))
        })
    }

    fn submit(&self, submission: Submission, cx: Cx) -> BoxFuture<'static, Result<Reply, Error>> {
        Box::pin(async move {
            if !matches!(submission.action.as_str(), "find" | "choose-session") {
                return Err(Error::Invalid("Unknown recall action".into()));
            }
            let mut filters = Filters::read(&submission.route)?;
            filters.query = submission.text("query")?.trim().to_owned();
            filters.since = submission.text("since")?.trim().to_owned();
            filters.until = submission.text("until")?.trim().to_owned();
            if let Err(error) = filters.validate() {
                return Ok(Reply::Rejected {
                    message: cx.t(
                        &error,
                        "请输入最多八个词；日期使用 YYYY-MM-DD（UTC），起始日期不得晚于结束日期。",
                        "請輸入最多八個詞；日期使用 YYYY-MM-DD（UTC），起始日期不得晚於結束日期。",
                    ),
                });
            }
            let mut route = serde_json::to_value(filters).expect("recall filters");
            if submission.action == "choose-session" {
                route["pickSession"] = json!(true);
            }
            Ok(Reply::Applied { route })
        })
    }
}

fn finder(words: &Words, filters: &Filters, found: Option<Result<Found, Error>>) -> View {
    let mut children = vec![
        text(
            "intro",
            words.t(
                "Find what was said in earlier conversations.",
                "在以前的对话中查找说过的内容。",
                "在以前的對話中尋找說過的內容。",
            ),
            Tone::Muted,
        ),
        row(
            "search",
            vec![
                input("query", "query", words.t("Find", "查找", "尋找")),
                button("find", "find", Role::Primary),
            ],
        ),
    ];
    children.push(filters.controls(words));
    match found {
        None => {}
        Some(Ok(found)) if found.matches.is_empty() => children.push(text(
            "none",
            words.t("Nothing matched.", "没有匹配的内容。", "沒有符合的內容。"),
            Tone::Muted,
        )),
        Some(Ok(found)) => {
            let rows: Vec<Node> = found
                .matches
                .iter()
                .map(|found| {
                    use sha2::{Digest, Sha256};
                    let (title, detail, target) = match found {
                        Match::Title { session_id, title } => (
                            title,
                            String::new(),
                            Target::Session {
                                session: session_id.clone(),
                            },
                        ),
                        Match::Passage {
                            session_id,
                            title,
                            text,
                            truncated,
                            turn_id,
                            message_id,
                            sequence,
                        } => (
                            title,
                            format!(
                                "{}{}",
                                view::build::clean(text, false),
                                if *truncated { "…" } else { "" }
                            ),
                            Target::SessionMessage {
                                session: session_id.clone(),
                                turn: turn_id.clone(),
                                message: message_id.clone(),
                                sequence: *sequence,
                            },
                        ),
                    };
                    let identity = serde_json::to_vec(&target).expect("recall target");
                    Node::Item {
                        key: format!("match-{:x}", Sha256::digest(identity)),
                        title: view::build::clean(title, false),
                        detail,
                        meta: String::new(),
                        tone: Tone::Normal,
                        current: false,
                        target,
                    }
                })
                .collect();
            children.push(scroll("matches", 40, stack("list", rows)));
            if !found.complete {
                children.push(text(
                    "more",
                    words.t(
                        "More matched; add words to narrow it down.",
                        "还有更多匹配；再加一些词可以缩小范围。",
                        "還有更多符合；再加一些詞可以縮小範圍。",
                    ),
                    Tone::Subtle,
                ));
            }
        }
        Some(Err(error)) => children.push(text(
            "failed",
            view::build::clean(&error.to_string(), false)
                .chars()
                .take(512)
                .collect::<String>(),
            Tone::Warning,
        )),
    }
    View {
        version: VERSION,
        title: words.t("Recall", "回忆", "回憶"),
        revision: "recall".into(),
        fields: filters.fields(),
        actions: vec![
            Action {
                fields: vec!["query".into(), "since".into(), "until".into()],
                ..view::build::action("find", words.t("Find", "查找", "尋找"))
            },
            Action {
                fields: vec!["query".into(), "since".into(), "until".into()],
                ..view::build::action(
                    "choose-session",
                    words.t("Choose session", "选择会话", "選擇對話"),
                )
            },
        ],
        root: column("root", children),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_open_the_session_they_came_from() {
        let words = Words::new("en");
        finder(&words, &Filters::default(), None)
            .validate()
            .unwrap();
        let found = Found {
            matches: vec![
                Match::Title {
                    session_id: "a".into(),
                    title: "Release plan".into(),
                },
                Match::Passage {
                    session_id: "b".into(),
                    title: "Refactor".into(),
                    text: "We chose the kernel".into(),
                    truncated: true,
                    turn_id: "turn-b".into(),
                    message_id: "message-b".into(),
                    sequence: 17,
                },
            ],
            complete: false,
        };
        let view = finder(
            &words,
            &Filters {
                query: "kernel".into(),
                ..Default::default()
            },
            Some(Ok(found)),
        );
        view.validate().unwrap();
        fn anchor(node: &Node) -> Option<&Target> {
            if let Node::Item {
                target: target @ Target::SessionMessage { .. },
                ..
            } = node
            {
                return Some(target);
            }
            node.children().into_iter().find_map(anchor)
        }
        assert!(
            matches!(anchor(&view.root), Some(Target::SessionMessage { session, turn, message, sequence })
            if session == "b" && turn == "turn-b" && message == "message-b" && *sequence == 17)
        );
        assert_eq!(terms("a b c d e f g h i j").len(), 8);
    }
}
