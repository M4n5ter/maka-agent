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

//! Home: what the main area shows when no session is open. A calm, centered
//! starting point with one primary action, recent sessions, and a readable
//! recovery path when the Host is unavailable.
use crate::{
    app::{Action, App, ConnectionState, Focus},
    navigation::Route,
    ui::{self, Align, Node, On, Size, Tone},
};
use ratatui::{Frame, layout::Rect};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    New,
    RetrySetup,
    Open(String),
    Connect,
    Host,
    Action(Box<Action>),
}

#[derive(Default)]
pub struct State {
    pub surface: ui::Surface<Message>,
    /// Where new sessions start: this process's directory, read once.
    place: Option<String>,
}

const RECENT: usize = 5;
const WIDTH: u16 = 52;

/// `directory` tells whether the sidebar is showing the session list; when it
/// is not, home offers recent sessions itself.
pub fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect, directory: bool) {
    if app.home.place.is_none() {
        app.home.place = Some(std::env::current_dir().map_or_else(
            |_| String::new(),
            |path| crate::view::safe(&path.to_string_lossy()),
        ));
    }
    let tree = tree(app, area.width, directory);
    let context = ui::Context {
        colors: app.theme.colors(),
        ascii: app.chrome.ascii,
        focused: app.focus == Focus::Page && app.overlay().is_none(),
    };
    app.home.surface.render(frame, area, tree, context);
}

fn tree(app: &App, width: u16, directory: bool) -> Node<Message> {
    let i18n = &app.i18n;
    let mut content = vec![
        Node::text("title", vec![("Maka".into(), Tone::Strong)]).align(Align::Center),
        Node::text(
            "place",
            vec![(app.home.place.clone().unwrap_or_default(), Tone::Subtle)],
        )
        .align(Align::Center)
        .clip(),
        Node::text("gap", vec![]).size(Size::Fixed(1)),
    ];
    match &app.connection {
        ConnectionState::Connected { .. } => {
            use crate::pages::connections::Setup;
            match app.connections.setup() {
                Setup::Loading => content.push(
                    Node::text(
                        "setup-loading",
                        vec![(i18n.text("connections-loading"), Tone::Subtle)],
                    )
                    .align(Align::Center),
                ),
                Setup::Failed => {
                    content.push(
                        Node::text(
                            "setup-failed",
                            vec![(i18n.text("home-setup-failed"), Tone::Warning)],
                        )
                        .align(Align::Center),
                    );
                    content.push(button(
                        "retry-setup",
                        i18n.text("home-retry"),
                        "",
                        Message::RetrySetup,
                    ));
                }
                Setup::NeedsConnection | Setup::NeedsDefault => {
                    let (note, action) = if app.connections.setup() == Setup::NeedsConnection {
                        ("home-needs-connection", "route-connections")
                    } else {
                        ("home-needs-default", "default-model-title")
                    };
                    content.push(
                        Node::text("setup-note", vec![(i18n.text(note), Tone::Subtle)])
                            .align(Align::Center),
                    );
                    content.push(button("setup", i18n.text(action), "Ctrl+N", Message::New));
                }
                Setup::Ready => content.push(button(
                    "new",
                    format!("+ {}", i18n.text("sidebar-new-session")),
                    "Ctrl+N",
                    Message::New,
                )),
            }
            for (index, action) in app.current_workspace_launches().into_iter().enumerate() {
                content.push(
                    button(
                        format!("launch-{index}"),
                        crate::view::action_label(app, &action),
                        "",
                        Message::Action(Box::new(action.clone())),
                    )
                    .enabled(app.enabled(&action)),
                );
            }
            content.push(button(
                "projects",
                i18n.text("route-projects"),
                "",
                Message::Action(Box::new(Action::Visit(Route::Projects))),
            ));
            if let Some(action) = app.bundle_import_action() {
                content.push(
                    button(
                        "import",
                        crate::view::action_label(app, &action),
                        "",
                        Message::Action(Box::new(action.clone())),
                    )
                    .enabled(app.enabled(&action)),
                );
            }
            let recent: Vec<_> = crate::pages::sidebar::groups(app)
                .into_iter()
                .flat_map(|group| {
                    let place = group.name;
                    group
                        .members
                        .into_iter()
                        .map(move |item| (item, place.clone()))
                })
                .take(if directory { 0 } else { RECENT })
                .collect();
            if !recent.is_empty() {
                content.push(Node::text("gap-recent", vec![]).size(Size::Fixed(1)));
                content.push(Node::text(
                    "recent-title",
                    vec![(i18n.text("home-recent"), Tone::Subtle)],
                ));
                for (item, place) in recent {
                    content.push(
                        Node::row(
                            format!("recent-{}", item.id),
                            vec![
                                Node::text("name", vec![(item.name.clone(), Tone::Normal)])
                                    .clip()
                                    .size(Size::Fill),
                                Node::text("place", vec![(place, Tone::Subtle)]).clip(),
                            ],
                        )
                        .gap(2)
                        .on(On::Activate(Message::Open(item.id.clone())))
                        .context_menu(
                            crate::pages::actions::context(
                                app,
                                &format!("session/{}", item.id),
                                crate::pages::actions::catalog_session_commands(app, item),
                            )
                            .map(&|action| Message::Action(Box::new(action))),
                        )
                        .hint(item.name.clone()),
                    );
                }
            }
        }
        ConnectionState::Failed(error) => {
            content.push(
                Node::text(
                    "problem",
                    vec![(i18n.text("home-host-failed"), Tone::Warning)],
                )
                .align(Align::Center),
            );
            content.push(
                Node::text("detail", vec![(error.clone(), Tone::Subtle)]).align(Align::Center),
            );
            content.push(Node::text("gap-actions", vec![]).size(Size::Fixed(1)));
            content.push(button(
                "retry",
                i18n.text("home-retry"),
                "",
                Message::Connect,
            ));
            content.push(button("host", i18n.text("home-host"), "", Message::Host));
        }
        ConnectionState::WrongEpoch => {
            content.push(
                Node::text(
                    "problem",
                    vec![(i18n.text("connection-failed"), Tone::Warning)],
                )
                .align(Align::Center),
            );
            content.push(button("host", i18n.text("home-host"), "", Message::Host));
        }
        ConnectionState::Disconnected => {
            content.push(
                Node::text(
                    "problem",
                    vec![(i18n.text("sidebar-host-disconnected"), Tone::Subtle)],
                )
                .align(Align::Center),
            );
            content.push(button(
                "connect",
                i18n.text("home-connect"),
                "",
                Message::Connect,
            ));
        }
        ConnectionState::Connecting => content.push(
            Node::text(
                "connecting",
                vec![(i18n.text("home-connecting"), Tone::Subtle)],
            )
            .align(Align::Center),
        ),
    }
    // Vertically and horizontally centered, with a bounded reading width.
    let column = Node::column("content", content).size(Size::Fixed(WIDTH.min(width)));
    Node::column(
        "home",
        vec![
            Node::text("top", vec![]).size(Size::Fill),
            Node::row(
                "center",
                vec![
                    Node::text("left", vec![]).size(Size::Fill),
                    column,
                    Node::text("right", vec![]).size(Size::Fill),
                ],
            ),
            Node::text("bottom", vec![]).size(Size::Fill),
        ],
    )
}

/// A centered action row; its whole width is the hit target.
fn button(
    key: impl Into<String>,
    label: String,
    shortcut: &str,
    message: Message,
) -> Node<Message> {
    let mut spans = vec![(label, Tone::Accent)];
    if !shortcut.is_empty() {
        spans.push((format!("   {shortcut}"), Tone::Subtle));
    }
    Node::text(key.into(), spans)
        .align(Align::Center)
        .on(On::Activate(message))
}

impl App {
    pub(crate) fn home_action(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::New => self.apply(Action::CreateSession),
            Message::RetrySetup => {
                self.connections.refresh();
                None
            }
            Message::Open(id) => self.apply(Action::Visit(Route::Session(id))),
            Message::Connect => self.apply(Action::Connect),
            Message::Host => self.apply(Action::Host),
            Message::Action(action) => self.apply(*action),
        }
    }
}
