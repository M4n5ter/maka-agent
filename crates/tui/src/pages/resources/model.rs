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
impl State {
    pub fn complete(&mut self, request: Request, result: Result<Output, Failure>) {
        let same = self.target.as_ref().is_some_and(|target| {
            target.root == request.target.root
                && target.epoch == request.target.epoch
                && target.session == request.target.session
        });
        let check = matches!(request.work, Work::Check(_))
            && self.saved.as_ref().is_some_and(|saved| {
                saved.root == request.target.root && saved.session == request.target.session
            })
            && self.target.as_ref().is_some_and(|target| {
                target.root == request.target.root && target.epoch == request.target.epoch
            });
        let current = (same || check)
            && self.generation == request.generation
            && self.serial == request.serial;
        let mutation = request.saved().is_some() && self.saved == request.saved();
        if !current && !mutation {
            return;
        }
        if current {
            self.busy = false;
        }
        match result {
            Ok(Output::Started(result)) if mutation => {
                let result = *result;
                self.saved = None;
                self.mutation = Mutation::Idle;
                if same {
                    let Work::Start(input) = request.work else {
                        return;
                    };
                    self.selected = Some(result.resource_ref.clone());
                    self.generation += 1;
                    self.terminal.clear();
                    self.items.insert(
                        result.resource_ref.clone(),
                        ResourceUpdate {
                            session_id: request.target.session,
                            ownership: Ownership::Local,
                            source_turn_id: input.launch_id.clone(),
                            source_tool_call_id: input.launch_id,
                            result,
                        },
                    );
                    self.command_form = false;
                    self.command = Editor::bounded(32 * 1024, "resources-command-limit");
                    self.refresh();
                }
            }
            Ok(Output::Stopped) if mutation => {
                self.saved = None;
                self.mutation = Mutation::Idle;
                self.terminal.clear();
                if same {
                    self.refresh();
                }
            }
            Ok(Output::Query(output)) => self.query(request, *output),
            Err(failure) => {
                if mutation {
                    if failure.unknown {
                        self.mutation = Mutation::Unknown;
                    } else {
                        self.saved = None;
                        self.mutation = Mutation::Idle;
                    }
                }
                self.error = Some(if failure.unknown {
                    "resources-unknown"
                } else {
                    "resources-failed"
                });
            }
            _ => self.error = Some("resources-failed"),
        }
        if current && self.stale && self.pending.is_none() {
            self.stale = false;
            self.refresh_selected();
        }
    }
    fn query(&mut self, request: Request, output: ResourceQueryResult) {
        let checking = matches!(request.work, Work::Check(_));
        match output {
            ResourceQueryResult::RevisionChanged { .. } => {
                let input = ResourceQueryInput::ListStart {
                    session_id: request.target.session,
                };
                self.pending = Some(if checking {
                    self.checking = None;
                    self.ambiguous = false;
                    Work::Check(input)
                } else {
                    Work::Query(input)
                });
            }
            ResourceQueryResult::Page {
                session_id,
                revision,
                resources,
                next_cursor,
            } => {
                if !checking {
                    self.items.clear();
                    self.next_page = None;
                }
                for row in resources {
                    if checking {
                        if let Some(Checkpoint {
                            operation: Pending::Launch { id },
                            ..
                        }) = &self.saved
                            && matches!(row.ownership, Ownership::Local)
                            && row.session_id == session_id
                            && row.source_turn_id == *id
                            && row.source_tool_call_id == *id
                        {
                            self.ambiguous |= self.checking.as_ref().is_some_and(|previous| {
                                previous.resource_ref != row.result.resource_ref
                            });
                            self.checking = Some(row.result.clone());
                        }
                    } else {
                        self.items.insert(row.result.resource_ref.clone(), row);
                    }
                }
                if let Some(cursor) = next_cursor {
                    if checking {
                        self.pending = Some(Work::Check(ResourceQueryInput::ListContinue {
                            session_id,
                            revision,
                            cursor,
                        }));
                    } else {
                        self.next_page = Some((revision, cursor));
                    }
                } else if checking {
                    if !self.ambiguous
                        && let Some(found) = self.checking.take()
                    {
                        self.saved = None;
                        self.mutation = Mutation::Idle;
                        self.error = Some("resources-found");
                        if self
                            .target
                            .as_ref()
                            .is_some_and(|target| target.session == session_id)
                        {
                            self.selected = Some(found.resource_ref);
                            self.refresh();
                        }
                    } else {
                        self.error = Some("resources-unknown");
                    }
                } else if self
                    .selected
                    .as_ref()
                    .is_some_and(|selected| !self.items.contains_key(selected))
                {
                    self.selected = None;
                    self.terminal.clear();
                }
            }
            ResourceQueryResult::Resource { resource, .. } if checking => {
                if resource.as_ref().is_some_and(|row| {
                    matches!(
                        row.result.status,
                        ShellStatus::Completed
                            | ShellStatus::Failed
                            | ShellStatus::TimedOut
                            | ShellStatus::Cancelled
                    )
                }) {
                    self.saved = None;
                    self.mutation = Mutation::Idle;
                    self.error = Some("resources-ended");
                    self.refresh();
                } else {
                    self.error = Some("resources-unknown");
                }
            }
            ResourceQueryResult::Resource { resource, .. } => {
                if let Some(row) = resource {
                    self.items.insert(row.result.resource_ref.clone(), *row);
                } else {
                    self.selected = None;
                    self.terminal.clear();
                }
            }
        }
    }
}
