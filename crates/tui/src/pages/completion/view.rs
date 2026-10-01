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

use super::model::{Origin, Payload, Pick};
use super::*;
use crate::ui::{self, Node};
mod tree;
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Clear, Paragraph},
};
use tree::tree;

pub(super) fn origin(app: &App, origin: &Origin) -> String {
    match origin {
        Origin::Workspace => app.i18n.text("completion-workspace"),
        Origin::Skill => app.i18n.text("completion-plugins"),
        Origin::Session { name } => name.clone(),
        Origin::Plugin { title, package } => format!("{title} · {package}"),
    }
}

pub(crate) fn chips(app: &App, session: &str) -> Option<Node<Action>> {
    let key = DraftKey {
        session: session.into(),
        input: None,
        display: false,
    };
    let editor = app.drafts.get(session)?;
    if editor.marks().is_empty() {
        return None;
    }
    let names = editor
        .marks()
        .iter()
        .filter_map(|mark| {
            app.completion
                .bindings
                .get(&key)?
                .get(&mark.id)
                .map(|binding| binding.label.clone())
        })
        .collect::<Vec<_>>()
        .join(" · ");
    Some(crate::view::shell::controls::summary(
        "context-bindings",
        "@ ".into(),
        names,
        String::new(),
        Action::Completion(Command::Added),
        true,
        app.i18n.text("completion-added"),
    ))
}

pub(crate) fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(popup) = &app.completion.popup else {
        return;
    };
    if !app.completion_current(&popup.context, popup.explicit) {
        return;
    }
    let caret = app
        .completion_editor(&popup.context.draft)
        .and_then(Editor::cursor_position)
        .unwrap_or(Position::new(area.x + 2, area.bottom().saturating_sub(2)));
    let above = caret.y.saturating_sub(area.y);
    let below = area.bottom().saturating_sub(caret.y + 1);
    let height = 18.min(above.max(below));
    if height < 5 || area.width < 12 {
        app.completion.invalidate_geometry();
        return;
    }
    let width = area.width.saturating_sub(2).min(76);
    let inner_width = width.saturating_sub(2);
    let tree = tree(app, inner_width, height.saturating_sub(2));
    let height = if popup.preview.is_some() && popup.error.is_none() {
        height
    } else {
        height.min(tree.required_height(inner_width).saturating_add(2))
    };
    let x = caret
        .x
        .clamp(area.x + 1, area.right().saturating_sub(width + 1));
    let y = if above >= below {
        caret.y.saturating_sub(height)
    } else {
        caret.y + 1
    };
    let rect = Rect::new(x, y, width, height);
    let inner = Rect::new(
        x + 1,
        y + 1,
        width.saturating_sub(2),
        height.saturating_sub(2),
    );
    let colors = app.theme.colors();
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Block::bordered()
            .border_type(if app.chrome.ascii {
                BorderType::Plain
            } else {
                BorderType::Rounded
            })
            .border_style(Style::default().fg(colors.border))
            .style(colors.base()),
        rect,
    );
    let context = ui::Context {
        colors,
        ascii: app.chrome.ascii,
        focused: true,
    };
    let popup = app.completion.popup.as_mut().unwrap();
    popup.area = Some(rect);
    if !popup.surface.captures() && !popup.controls {
        if popup.preview.is_some() {
            popup.surface.focus("completion/preview".into());
        } else if let Some(id) = &popup.selected {
            popup
                .surface
                .focus(format!("completion/candidates/{id}/title/name"));
        }
    }
    popup.surface.render(frame, inner, tree, context);
    if let Some(binding) = &popup.preview
        && let Some(label) = popup
            .surface
            .rect("completion/label")
            .filter(|rect| !rect.is_empty())
    {
        frame.render_widget(
            Paragraph::new(crate::view::safe(&binding.label)).style(
                Style::default()
                    .fg(colors.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
            label,
        );
    }
    popup.query_area = popup
        .surface
        .rect("completion/query")
        .filter(|area| !area.is_empty());
    if let Some(area) = popup.query_area {
        popup.query.draw(frame, area, !popup.controls, colors);
    }
    popup.surface.repaint_popover(frame, &context);
}
