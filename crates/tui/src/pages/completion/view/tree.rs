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
use crate::ui::{Align, On, Role, Size, Tone};

pub(super) fn tree(app: &App, width: u16, height: u16) -> Node<Command> {
    let popup = app.completion.popup.as_ref().unwrap();
    let current = popup.source.category();
    let categories = if popup.context.token.kind == Kind::Command {
        vec![Category::Commands, Category::Plugins, Category::Added]
    } else {
        vec![
            Category::Workspace,
            Category::Sessions,
            Category::Plugins,
            Category::Added,
        ]
    };
    let mut category = Node::text(
        "category",
        vec![(
            format!(
                "{} {}  Shift+Tab",
                app.i18n.text(current.label()),
                app.chrome.symbol("▾", "v")
            ),
            Tone::Strong,
        )],
    )
    .clip()
    .size(Size::Fill)
    .on(On::Choose {
        current: categories.iter().position(|category| *category == current),
        choices: categories
            .into_iter()
            .map(|category| ui::Choice {
                label: app.i18n.text(category.label()),
                action: Command::Category(category),
            })
            .collect(),
    });
    if app.completion.reselect.is_some() {
        category = Node::text(
            "category",
            vec![(app.i18n.text("completion-reselect"), Tone::Strong)],
        )
        .size(Size::Fill);
    }
    let mut rows = vec![Node::row(
        "header",
        vec![
            category,
            Node::text(
                "close",
                vec![(app.chrome.symbol("×", "x").into(), Tone::Muted)],
            )
            .size(Size::Fixed(3))
            .align(Align::Center)
            .on(On::Activate(Command::Close))
            .hint(app.i18n.text("session-cancel")),
        ],
    )];
    rows.push(
        Node::text(
            "keys",
            vec![(
                format!(
                    "F6 · {}",
                    app.i18n.text(if popup.controls {
                        "completion-results"
                    } else {
                        "completion-controls"
                    })
                ),
                Tone::Subtle,
            )],
        )
        .clip(),
    );
    if popup.explicit && app.completion.reselect.is_none() {
        rows.push(Node::slot("query", 1).on(On::Activate(Command::Retry)));
    }
    let loading = popup.requested
        || app
            .completion
            .pending
            .as_ref()
            .is_some_and(|pending| pending.generation == popup.generation);
    let mut controls = vec![];
    if !popup.previous.is_empty() {
        controls.push(
            Node::text(
                "previous",
                vec![(app.chrome.symbol("‹", "<").into(), Tone::Muted)],
            )
            .size(Size::Fixed(3))
            .on(On::Activate(Command::Previous))
            .hint(app.i18n.text("directory-previous")),
        );
    }
    if popup.next.is_some() {
        controls.push(
            Node::text(
                "next",
                vec![(app.chrome.symbol("›", ">").into(), Tone::Muted)],
            )
            .size(Size::Fixed(3))
            .on(On::Activate(Command::Next))
            .hint(app.i18n.text("directory-next")),
        );
    }
    if let Some(candidate) = popup
        .candidates
        .iter()
        .find(|candidate| popup.selected.as_ref() == Some(&candidate.id))
    {
        if let Pick::Bound(id) = &candidate.pick {
            let target = SelectionTarget {
                draft: popup.context.draft.clone(),
                key: ReferenceKey::Mark(id.clone()),
            };
            for (key, command) in [
                ("reselect", Command::Reselect(target.clone())),
                ("excerpt", Command::KeepExcerpt(target)),
            ] {
                if app.completion_enabled(&command) {
                    controls.push(
                        Node::text(key, vec![(app.i18n.text(command.label()), Tone::Muted)])
                            .on(On::Activate(command)),
                    );
                }
            }
            controls.push(
                Node::text(
                    "remove",
                    vec![(app.i18n.text("completion-remove"), Tone::Muted)],
                )
                .on(On::Activate(Command::Remove {
                    draft: popup.context.draft.clone(),
                    id: id.clone(),
                })),
            );
        } else if !matches!(
            candidate.pick,
            Pick::Native(_) | Pick::Command(_) | Pick::Session { .. } | Pick::Provider(_)
        ) {
            controls.push(
                Node::text(
                    "preview",
                    vec![(app.i18n.text("completion-preview"), Tone::Muted)],
                )
                .on(On::Activate(Command::Preview {
                    generation: popup.generation,
                    id: candidate.id.clone(),
                })),
            );
        }
        controls.push(Node::text("space", vec![]).size(Size::Fill));
        if !matches!(candidate.pick, Pick::Bound(_)) {
            controls.push(
                Node::text(
                    "select",
                    vec![(app.i18n.text("completion-select"), Tone::Accent)],
                )
                .on(On::Activate(Command::Choose {
                    generation: popup.generation,
                    id: candidate.id.clone(),
                }))
                .enabled(candidate.enabled),
            );
        }
    }
    if loading && !popup.candidates.is_empty() {
        controls.push(Node::text("loading", vec![("…".into(), Tone::Subtle)]));
    }
    let header_height = Node::column("headers", rows.clone()).required_height(width);
    let footer_height = Node::row("controls", controls.clone())
        .gap(1)
        .required_height(width);
    let candidate_capacity =
        usize::from(height.saturating_sub(header_height.saturating_add(footer_height)) / 2);
    let selection_presentable =
        popup.error.is_none() && (popup.preview.is_some() || candidate_capacity > 0);
    for control in &mut controls {
        if matches!(
            &control.on,
            Some(On::Activate(
                Command::Choose { .. }
                    | Command::Preview { .. }
                    | Command::Browse { .. }
                    | Command::Remove { .. }
                    | Command::Reselect(_)
                    | Command::KeepExcerpt(_)
                    | Command::DropSelection(_)
            ))
        ) {
            control.enabled &= selection_presentable;
        }
    }
    if let Some(error) = &popup.error {
        rows.push(Node::text("error", vec![(error.clone(), Tone::Warning)]).clip());
        rows.push(Node::row(
            "error-actions",
            vec![
                Node::button("retry", app.i18n.text("list-retry"), Role::Normal)
                    .on(On::Activate(Command::Retry)),
            ],
        ));
    } else if let Some(binding) = &popup.preview {
        rows.push(Node::slot("label", 1));
        rows.push(Node::text("source", vec![(origin(app, &binding.origin), Tone::Muted)]).clip());
        let quote = match &binding.payload {
            Payload::Context { quote, .. } => Some(quote),
            Payload::Selection { quote, .. } => quote.as_ref(),
        };
        if let Some(quote) = quote {
            let lines = quote
                .text
                .lines()
                .enumerate()
                .map(|(index, line)| {
                    Node::text(
                        index.to_string(),
                        vec![(crate::view::safe(line), Tone::Normal)],
                    )
                })
                .collect();
            rows.push(
                Node::scroll("preview", Node::column("lines", lines))
                    .size(Size::Fill)
                    .on(On::Scroll),
            );
            if quote.source.as_ref().is_some_and(|source| source.truncated) {
                rows.push(Node::text(
                    "truncated",
                    vec![(app.i18n.text("completion-truncated"), Tone::Warning)],
                ));
            }
        }
    } else {
        // Only complete title/detail pairs fit between this frame's measured
        // header and footer. No zero-height container may emit natural-height
        // children over another control row.
        let selected = popup
            .selected
            .as_ref()
            .and_then(|id| {
                popup
                    .candidates
                    .iter()
                    .position(|candidate| candidate.id == *id)
            })
            .unwrap_or(0);
        let visible = candidate_capacity;
        let first = selected.saturating_sub(visible.saturating_sub(1));
        let candidates = popup.candidates.iter().skip(first).take(visible).map(|candidate| {
            let command = || Command::Choose { generation: popup.generation, id: candidate.id.clone() };
            let mut title = vec![Node::text("name", vec![(candidate.title.clone(), Tone::Normal)]).clip().size(Size::Fill).on(On::Activate(command())).enabled(candidate.enabled)];
            if matches!(&candidate.pick, Pick::Workspace(input) if input.kind == maka_protocol::session::workspace_context::Kind::Directory) {
                title.push(Node::text("browse", vec![(app.chrome.symbol("›", ">").into(), Tone::Accent)]).size(Size::Fixed(3)).align(Align::Center)
                    .on(On::Activate(Command::Browse { generation: popup.generation, id: candidate.id.clone() })).hint(app.i18n.text("completion-browse")));
            }
            Node::column(candidate.id.clone(), vec![Node::row("title", title),
                Node::text("detail", vec![(format!("{}{}{}", candidate.source, if candidate.detail.is_empty() { "" } else { " · " }, candidate.detail), Tone::Muted)]).clip()])
                .current(popup.selected.as_ref() == Some(&candidate.id))
        }).collect();
        rows.push(Node::column("candidates", candidates).size(Size::Fill));
        if popup.candidates.is_empty() {
            rows.push(Node::text(
                "empty",
                vec![(
                    app.i18n.text(if loading {
                        "completion-loading"
                    } else {
                        "completion-empty"
                    }),
                    Tone::Subtle,
                )],
            ));
        }
    }
    rows.push(Node::row("controls", controls).gap(1));
    Node::column("completion", rows)
}
