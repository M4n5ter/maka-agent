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

//! A read-only scan of the Host's paged session directory for the palette.
//! It keeps search results separate from the sidebar's loaded catalog.

use super::Label;
use crate::{app::Action, navigation::Route, view::safe};
use maka_protocol::session::{
    SessionCatalogProjection, SessionCatalogQueryInput as Query, SessionCatalogQueryResult as Page,
};

const PAGE_OF_RESULTS: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    instance: u64,
    generation: u64,
    root: String,
    epoch: String,
    pub query: Query,
}

#[derive(Default)]
pub struct State {
    instance: u64,
    generation: u64,
    search: String,
    revision: Option<String>,
    cursor: Option<String>,
    pending: Option<Request>,
    requested: bool,
    restarted: bool,
    limit: usize,
    pub results: Vec<(Action, Label)>,
    pub error: bool,
}

impl State {
    pub fn new(instance: u64) -> Self {
        Self {
            instance,
            ..Self::default()
        }
    }
    pub fn restart(&mut self, query: &str) {
        self.generation = self.generation.wrapping_add(1);
        self.search = query.to_lowercase();
        self.pending = None;
        self.revision = None;
        self.cursor = None;
        self.requested = !self.search.trim().is_empty();
        self.restarted = false;
        self.limit = PAGE_OF_RESULTS;
        self.results.clear();
        self.error = false;
    }

    pub fn request(&mut self, root: &str, epoch: &str) -> Option<Request> {
        if !self.requested || self.pending.is_some() {
            return None;
        }
        self.requested = false;
        let query = match (&self.revision, &self.cursor) {
            (Some(revision), Some(cursor)) => Query::ListContinue {
                revision: revision.clone(),
                cursor: cursor.clone(),
            },
            _ => Query::ListStart,
        };
        let request = Request {
            instance: self.instance,
            generation: self.generation,
            root: root.into(),
            epoch: epoch.into(),
            query,
        };
        self.pending = Some(request.clone());
        Some(request)
    }

    pub fn complete(
        &mut self,
        request: &Request,
        result: Result<Page, String>,
        host: Option<(&str, &str)>,
    ) -> bool {
        if self.pending.as_ref() != Some(request) {
            return false;
        }
        self.pending = None;
        if request.generation != self.generation {
            return false;
        }
        if host != Some((request.root.as_str(), request.epoch.as_str())) {
            self.results.clear();
            self.requested = false;
            self.error = true;
            return true;
        }
        match result {
            Ok(Page::Page {
                revision,
                sessions,
                next_cursor,
            }) => {
                self.revision = Some(revision);
                self.cursor = next_cursor;
                for session in sessions {
                    if self.matches(&session)
                        && !self.results.iter().any(|(action, _)| {
                            *action == Action::Visit(Route::Session(session.id.clone()))
                        })
                    {
                        self.results.push((
                            Action::Visit(Route::Session(session.id.clone())),
                            Label::Session {
                                name: safe(&session.name),
                                workspace: safe(&session.workspace.host_cwd),
                            },
                        ));
                    }
                }
                self.requested = self.cursor.is_some() && self.results.len() < self.limit;
            }
            Ok(Page::RevisionChanged { .. }) if !self.restarted => {
                self.restarted = true;
                self.revision = None;
                self.cursor = None;
                self.results.clear();
                self.requested = true;
            }
            Err(_) | Ok(Page::RevisionChanged { .. }) => {
                self.error = true;
                self.requested = false;
            }
            Ok(Page::Session { .. }) => unreachable!("list query cannot return one session"),
        }
        true
    }

    fn matches(&self, session: &SessionCatalogProjection) -> bool {
        let mut text = format!(
            "{} {} {}",
            session.name, session.id, session.workspace.host_cwd
        )
        .to_lowercase();
        for label in &session.labels {
            text.push(' ');
            text.push_str(&label.to_lowercase());
        }
        self.search
            .split_whitespace()
            .all(|word| text.contains(word))
    }

    pub fn loading(&self) -> bool {
        self.requested || self.pending.is_some()
    }

    pub fn can_more(&self) -> bool {
        !self.loading() && !self.error && self.cursor.is_some()
    }

    pub fn more(&mut self) {
        if self.can_more() {
            self.limit = self.limit.saturating_add(PAGE_OF_RESULTS);
            self.requested = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::sessions::tests::item;

    fn page(sessions: Vec<SessionCatalogProjection>, cursor: Option<&str>) -> Page {
        Page::Page {
            revision: "revision".into(),
            sessions,
            next_cursor: cursor.map(str::to_owned),
        }
    }

    #[test]
    fn search_reaches_later_host_pages_and_routes_by_session_identity() {
        let mut state = State::default();
        state.restart("deep");
        let first = state.request("root", "epoch").unwrap();
        assert_eq!(first.query, Query::ListStart);
        assert!(state.complete(
            &first,
            Ok(page(vec![item("recent")], Some("next"))),
            Some(("root", "epoch"))
        ));
        assert!(state.results.is_empty());
        let next = state.request("root", "epoch").unwrap();
        assert_eq!(
            next.query,
            Query::ListContinue {
                revision: "revision".into(),
                cursor: "next".into(),
            }
        );
        assert!(state.complete(
            &next,
            Ok(page(vec![item("deep-session")], None)),
            Some(("root", "epoch"))
        ));
        assert_eq!(state.results.len(), 1);
        assert_eq!(
            state.results[0].0,
            Action::Visit(Route::Session("deep-session".into()))
        );
        assert!(!state.loading());
        assert!(!state.can_more());
    }

    #[test]
    fn query_change_and_host_change_discard_in_flight_results() {
        let mut state = State::default();
        state.restart("old");
        let old = state.request("root", "epoch").unwrap();
        state.restart("new");
        let current = state.request("root", "epoch").unwrap();
        assert_eq!(current.query, Query::ListStart);
        assert!(!state.complete(
            &old,
            Ok(page(vec![item("old")], None)),
            Some(("root", "epoch"))
        ));
        assert!(state.results.is_empty());
        assert!(state.complete(
            &current,
            Ok(page(vec![item("new")], None)),
            Some(("other", "epoch"))
        ));
        assert!(state.results.is_empty());
        assert!(state.error);
    }

    #[test]
    fn reopening_the_palette_cannot_accept_its_previous_search_reply() {
        let mut previous = State::new(1);
        previous.restart("same");
        let old = previous.request("root", "epoch").unwrap();
        let mut current = State::new(2);
        current.restart("same");
        let fresh = current.request("root", "epoch").unwrap();
        assert_ne!(old, fresh);
        assert!(!current.complete(
            &old,
            Ok(page(vec![item("same")], None)),
            Some(("root", "epoch")),
        ));
        assert!(current.results.is_empty());
        assert!(current.complete(
            &fresh,
            Ok(page(vec![item("same")], None)),
            Some(("root", "epoch")),
        ));
        assert_eq!(current.results.len(), 1);
    }

    #[test]
    fn revision_change_clears_partial_results_and_restarts_from_the_first_page() {
        let mut state = State::default();
        state.restart("match");
        let first = state.request("root", "epoch").unwrap();
        state.complete(
            &first,
            Ok(page(vec![item("match")], Some("next"))),
            Some(("root", "epoch")),
        );
        let next = state.request("root", "epoch").unwrap();
        state.complete(
            &next,
            Ok(Page::RevisionChanged {
                expected_revision: "revision".into(),
                actual_revision: "changed".into(),
            }),
            Some(("root", "epoch")),
        );
        assert!(state.results.is_empty());
        assert_eq!(
            state.request("root", "epoch").unwrap().query,
            Query::ListStart
        );
    }

    #[test]
    fn more_keeps_the_next_host_cursor_without_dropping_matches_from_a_page() {
        let mut state = State::default();
        state.restart("session");
        let first = state.request("root", "epoch").unwrap();
        state.complete(
            &first,
            Ok(page(
                (0..32).map(|i| item(&format!("session-{i}"))).collect(),
                Some("next"),
            )),
            Some(("root", "epoch")),
        );
        assert_eq!(state.results.len(), 32);
        assert!(state.can_more());
        state.more();
        assert_eq!(
            state.request("root", "epoch").unwrap().query,
            Query::ListContinue {
                revision: "revision".into(),
                cursor: "next".into()
            }
        );
    }
}
