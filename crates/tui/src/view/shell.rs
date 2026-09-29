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

use super::{action_label, activity, icon, safe, session};
use crate::{
    app::{Action, App, ConnectionState, Focus, Notice},
    navigation::Route,
    pages::sessions::Detail,
    ui::{Align, Context, Node, On, Size, Tone},
};
use ratatui::{Frame, layout::Rect};

pub(crate) mod controls;
mod input;
pub(crate) use session::composer::EDITOR;
#[cfg(test)]
pub(crate) use session::composer::SEND;

pub(crate) fn action(
    app: &App,
    key: impl Into<std::borrow::Cow<'static, str>>,
    action: Action,
) -> Node<Action> {
    let tone = if matches!(
        action,
        Action::SendMessage | Action::SteerMessage | Action::StopTurn(_)
    ) {
        Tone::Accent
    } else {
        Tone::Muted
    };
    controls::compact(
        key,
        icon(app, &action).into(),
        tone,
        action.clone(),
        app.enabled(&action),
        action_label(app, &action),
    )
}

pub(super) fn header(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let session = matches!(app.navigation.current(), Route::Session(_));
    let mut actions = app.header_actions();
    if session {
        // The task remains visible; secondary controls live with this session.
        actions.retain(|action| matches!(action, Action::OpenInteraction | Action::LatestMessages));
        actions.push(Action::Search(crate::ui::transcript::search::Command::Open));
    }
    if app.navigation.current() == Route::Connections {
        actions.extend(app.oauth_commands().into_iter().filter_map(|(action, _)| {
            matches!(&action, Action::Manage(crate::pages::manage::Command::Open(target, crate::pages::manage::Kind::Oauth)) if target.is_enrollment()).then_some(action)
        }));
    }
    let object_menu = match app.navigation.current() {
        Route::Session(ref id) => Some(crate::pages::actions::menu(
            app,
            "session-actions",
            &crate::pages::actions::session_scope(app, id),
            "session-actions",
            crate::pages::actions::session_commands(app),
        )),
        Route::Connections => app
            .connections
            .rows
            .iter()
            .find(|row| app.connections.selected.as_ref() == Some(&row.id))
            .map(|row| {
                crate::pages::actions::menu(
                    app,
                    "connection-actions",
                    &format!("connection/{}", row.id),
                    "connection-actions",
                    crate::pages::actions::connection_commands(app, row),
                )
            }),
        Route::Projects => app
            .projects
            .items
            .iter()
            .find(|item| app.projects.selected.as_ref() == Some(&item.id))
            .map(|item| {
                crate::pages::actions::menu(
                    app,
                    "project-actions",
                    &format!("project/{}", item.id),
                    "project-actions",
                    crate::pages::actions::project_commands(app, item),
                )
            }),
        _ => None,
    };
    let actions: Vec<_> = actions
        .into_iter()
        .map(|value| (action(app, format!("{value:?}"), value), 3))
        .collect();
    let right_width = actions.iter().map(|(_, width)| width).sum::<u16>()
        + (1 + u16::from(object_menu.is_some())) * 3
        + 5;
    let side = right_width.max(6);
    let balanced = session && area.width >= side.saturating_mul(2).saturating_add(8);
    let left_width = if balanced { side } else { 6 };
    let title_width = area
        .width
        .saturating_sub(left_width + if balanced { side } else { right_width });
    let mut title = match (&app.navigation.current(), &app.sessions.detail) {
        (Route::Session(id), Detail::Ready(item)) if *id == item.id => safe(&item.name),
        (Route::App(key), _) => app
            .apps
            .instance(key)
            .and_then(|instance| instance.title(app.i18n.locale().id()))
            .map(|title| safe(&title))
            .unwrap_or_else(|| app.i18n.text(Route::Extensions.title())),
        _ => app.i18n.text(app.navigation.current().title()),
    };
    if !matches!(app.connection, ConnectionState::Connected { .. }) {
        let connection = match app.connection {
            ConnectionState::Disconnected => "connection-disconnected",
            ConnectionState::Connecting => "connection-connecting",
            ConnectionState::Connected { .. } => "connection-connected",
            ConnectionState::Failed(_) | ConnectionState::WrongEpoch => "connection-failed",
        };
        title = format!("{title} · {}", app.i18n.text(connection));
    }
    let spans = if let Route::Session(id) = app.navigation.current()
        && title_width >= 16
    {
        let activity = app.session_activity(&id);
        let face = match activity {
            activity::Activity::Working => app
                .chrome
                .animation
                .frame(crate::motion::Loop::Spring, app.chrome.ascii),
            activity::Activity::Waiting => "-_-",
            activity::Activity::Idle => app
                .chrome
                .animation
                .frame(crate::motion::Loop::Familiar, app.chrome.ascii),
            activity::Activity::Unknown => "o_o",
        };
        vec![
            (
                face.to_owned(),
                if activity == activity::Activity::Waiting {
                    Tone::Warning
                } else {
                    Tone::Accent
                },
            ),
            (
                format!(
                    " {}    ",
                    session::fit(&title, usize::from(title_width - 8))
                ),
                Tone::Strong,
            ),
        ]
    } else {
        vec![(title, Tone::Strong)]
    };
    let left = Node::row(
        "left",
        vec![
            action(app, "sidebar", Action::ToggleSidebar),
            action(app, "back", Action::Back),
        ],
    )
    .size(Size::Fixed(left_width));
    let mut right = Vec::new();
    if balanced && side > right_width {
        right.push(Node::text("space", vec![]).size(Size::Fixed(side - right_width)));
    }
    right.extend(actions.into_iter().map(|(node, _)| node));
    if let Some(menu) = object_menu {
        right.push(menu);
    }
    let unread = app.attention.unread();
    let count = if unread > 99 {
        "99+".to_owned()
    } else {
        unread.to_string()
    };
    right.push(
        Node::text(
            "attention",
            vec![(
                format!("{}{count}", app.chrome.symbol("●", "N")),
                if unread > 0 {
                    Tone::Accent
                } else {
                    Tone::Muted
                },
            )],
        )
        .align(Align::Center)
        .clip()
        .size(Size::Fixed(5))
        .on(On::Activate(Action::Attention(
            crate::pages::attention::Command::Open,
        )))
        .hint(app.i18n.text("attention-title")),
    );
    right.push(action(app, "palette", Action::Palette));
    let tree = Node::row(
        "header",
        vec![
            left,
            Node::text("title", spans)
                .clip()
                .align(if balanced {
                    Align::Center
                } else {
                    Align::Start
                })
                .size(Size::Fill),
            Node::row("right", right).size(Size::Fixed(if balanced { side } else { right_width })),
        ],
    );
    let context = Context {
        colors: app.theme.colors(),
        ascii: app.chrome.ascii,
        focused: app.focus == Focus::Header && app.overlay().is_none(),
    };
    app.chrome.header.render(frame, area, tree, context);
}

pub(super) fn footer(frame: &mut Frame<'_>, app: &mut App, area: Rect, hint: String) {
    let diagnostic = matches!(
        &app.notice,
        Some(Notice::Diagnostic(_) | Notice::CreateFailed(_))
    );
    let mut tree = Node::text(
        "footer",
        vec![(
            hint,
            if diagnostic {
                Tone::Warning
            } else {
                Tone::Subtle
            },
        )],
    )
    .align(Align::Center)
    .clip();
    if diagnostic {
        tree = tree.on(On::Activate(Action::Host));
    }
    let context = Context {
        colors: app.theme.colors(),
        ascii: app.chrome.ascii,
        focused: false,
    };
    app.chrome.footer.render(frame, area, tree, context);
}

/// Shell menus can extend beyond their one-line/short owner surface. Repaint
/// after all ordinary page content and before trusted sheets or tooltips.
pub(super) fn repaint_popovers(frame: &mut Frame<'_>, app: &mut App) {
    let context = Context {
        colors: app.theme.colors(),
        ascii: app.chrome.ascii,
        focused: true,
    };
    match app.navigation.current() {
        Route::Connections => app.connections.surface.repaint_popover(frame, &context),
        Route::Projects => app.projects.surface.repaint_popover(frame, &context),
        _ => {}
    }
    app.chrome.composer.repaint_popover(frame, &context);
    app.chrome.header.repaint_popover(frame, &context);
}
