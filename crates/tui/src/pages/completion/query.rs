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
use super::{
    io::{Cancellation, Job},
    model::Cursor,
};
use maka_protocol::session::{SessionCatalogQueryInput, workspace_context as workspace};

impl App {
    pub fn completion_request(&mut self) -> Option<Request> {
        if self.completion.pending.is_some() {
            return None;
        }
        let popup = self.completion.popup.as_ref()?;
        if !self.completion_current(&popup.context, popup.explicit) {
            self.completion.close();
            return None;
        }
        let cursor = popup.scan.as_ref().or(popup.cursor.as_ref());
        let job = if let Some(job) = self.completion.resolve.take() {
            job
        } else if popup.requested {
            match &popup.source {
                Source::Commands | Source::Skills => Job::Skills {
                    page: match cursor {
                        Some(Cursor::Skills { revision, cursor }) => {
                            Some((revision.clone(), cursor.clone()))
                        }
                        _ => None,
                    },
                },
                Source::Workspace { directory } => {
                    let (relative, filter) = popup.query.text().rsplit_once('/').map_or(
                        (directory.clone(), popup.query.text().to_owned()),
                        |(path, name)| {
                            (
                                if directory.is_empty() {
                                    path.into()
                                } else {
                                    format!("{directory}/{path}")
                                },
                                name.into(),
                            )
                        },
                    );
                    Job::Workspace(workspace::Query {
                        session_id: popup.context.session.clone(),
                        directory: relative,
                        filter,
                        cursor: match cursor {
                            Some(Cursor::Workspace(cursor)) => Some(cursor.clone()),
                            _ => None,
                        },
                    })
                }
                Source::Sessions => Job::Sessions(match cursor {
                    Some(Cursor::Catalog { revision, cursor }) => {
                        SessionCatalogQueryInput::ListContinue {
                            revision: revision.clone(),
                            cursor: cursor.clone(),
                        }
                    }
                    _ => SessionCatalogQueryInput::ListStart,
                }),
                Source::Providers => Job::Providers {
                    cursor: match cursor {
                        Some(Cursor::Providers(cursor)) => Some(cursor.clone()),
                        _ => None,
                    },
                },
                Source::Plugin(provider) => Job::Resources {
                    provider: provider.clone(),
                    query: popup.query.text().into(),
                    cursor: match cursor {
                        Some(Cursor::Resource(cursor)) => Some(cursor.clone()),
                        _ => None,
                    },
                },
                Source::Messages { session, name } => Job::Messages {
                    session: session.clone(),
                    name: name.clone(),
                    query: popup.query.text().into(),
                    anchor: match cursor {
                        Some(Cursor::Messages(anchor)) => Some(*anchor),
                        _ => None,
                    },
                },
                Source::Added => return None,
            }
        } else {
            return None;
        };
        self.completion.sequence = self.completion.sequence.wrapping_add(1);
        let popup = self.completion.popup.as_mut()?;
        popup.requested = false;
        let request = Request {
            id: self.completion.sequence,
            generation: popup.generation,
            context: popup.context.clone(),
            locale: self.i18n.locale().id().into(),
            job,
            cancel: Cancellation::default(),
        };
        self.completion.pending = Some(request.clone());
        Some(request)
    }
}
