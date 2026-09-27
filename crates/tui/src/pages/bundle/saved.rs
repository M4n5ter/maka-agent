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

use super::{State, WorkspaceTarget, api};
use crate::editor::{Editor, saved::Cursor};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Mode {
    Export { session: String, name: String },
    Import,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub root: String,
    pub mode: Mode,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Workspace {
    Project { id: String, name: String },
    HostPath { path: String },
}
impl Workspace {
    pub fn target(&self) -> WorkspaceTarget {
        match self {
            Self::Project { id, .. } => WorkspaceTarget::Project {
                project_id: id.clone(),
            },
            Self::HostPath { path } => WorkspaceTarget::HostPath { path: path.clone() },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Frozen {
    Export {
        session: String,
        destination: String,
        digest: String,
        count: u64,
    },
    Import {
        source: String,
        workspace: Workspace,
        expected: api::ImportPreviewed,
    },
}
impl Frozen {
    fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::Export {
                session,
                destination,
                digest,
                count,
            } => {
                api::decode_export(&serde_json::json!({
                    "sessionId":session,"destination":destination,"expectedSubtreeDigest":digest,
                }))
                .map_err(|e| e.to_string())?;
                api::decode_previewed(&serde_json::json!({
                    "sessionCount":count,"subtreeDigest":digest,
                }))
                .map_err(|e| e.to_string())?;
                valid_path(destination)
            }
            Self::Import {
                source,
                workspace,
                expected,
            } => {
                api::decode_import(&serde_json::json!({
                    "source":source,"workspace":workspace.target(),"expected":expected,
                }))
                .map_err(|e| e.to_string())?;
                (match workspace {
                    Workspace::Project { name, .. } => valid_text(name, 4096),
                    Workspace::HostPath { path } => valid_path(path),
                }) && valid_path(source)
            }
        };
        valid
            .then_some(())
            .ok_or_else(|| "Invalid bundle request".into())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Receipt {
    Exported {
        count: u64,
        bytes: u64,
    },
    Imported {
        receipt: api::ImportReceipt,
        artifacts: Option<u64>,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Outcome {
    #[default]
    Draft,
    Unknown {
        write: Frozen,
    },
    Complete {
        write: Frozen,
        receipt: Receipt,
    },
}
/// Drafts contain paths and stable identities only. A persisted write is unknown
/// until its actual reply arrives; reopening never resubmits it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    intent: Intent,
    path: String,
    path_cursor: Cursor,
    workspace: String,
    workspace_cursor: Cursor,
    project: Option<(String, String)>,
    outcome: Outcome,
}
impl Checkpoint {
    pub fn validate(&self, root: &str) -> Result<(), String> {
        if self.intent.root != root
            || !valid_text(&self.path, 4096)
            || !valid_text(&self.workspace, 4096)
        {
            return Err("Invalid bundle checkpoint".into());
        }
        self.path_cursor.validate(&self.path)?;
        self.workspace_cursor.validate(&self.workspace)?;
        if let Mode::Export { session, name } = &self.intent.mode {
            api::decode_preview(&serde_json::json!({"sessionId":session}))
                .map_err(|e| e.to_string())?;
            if !valid_text(name, 4096) || self.project.is_some() {
                return Err("Invalid bundle target".into());
            }
        }
        if let Some((id, name)) = &self.project {
            api::decode_import_preview(&serde_json::json!({"source":"/bundle","workspace":{"kind":"project","projectId":id}}))
                .map_err(|e| e.to_string())?;
            if !valid_text(name, 4096) {
                return Err("Invalid bundle project".into());
            }
        }
        if let Outcome::Unknown { write } | Outcome::Complete { write, .. } = &self.outcome {
            write.validate()?;
            let matches = match (&self.intent.mode, write) {
                (
                    Mode::Export { session, .. },
                    Frozen::Export {
                        session: actual,
                        destination,
                        ..
                    },
                ) => session == actual && self.path == *destination,
                (
                    Mode::Import,
                    Frozen::Import {
                        source, workspace, ..
                    },
                ) => {
                    self.path == *source
                        && match (workspace, &self.project) {
                            (Workspace::Project { id, name }, Some((actual, label))) => {
                                id == actual && name == label
                            }
                            (Workspace::HostPath { path }, None) => self.workspace == *path,
                            _ => false,
                        }
                }
                _ => false,
            };
            if !matches {
                return Err("Bundle checkpoint changed its request".into());
            }
        }
        if let Outcome::Complete { write, receipt } = &self.outcome {
            match (write, receipt) {
                (
                    Frozen::Export {
                        count: expected, ..
                    },
                    Receipt::Exported { count, bytes },
                ) if expected == count => {
                    api::decode_exported(
                        &serde_json::json!({"sessionCount":count,"compressedBytes":bytes}),
                    )
                    .map_err(|e| e.to_string())?;
                }
                (Frozen::Import { expected, .. }, Receipt::Imported { receipt, artifacts }) => {
                    api::decode_import_queried(&serde_json::json!({"receipt":receipt}))
                        .map_err(|e| e.to_string())?;
                    if expected.session_count != receipt.session_ids.len() as u64
                        || artifacts.is_some_and(|count| count != expected.artifact_files)
                    {
                        return Err("Bundle import receipt changed its preview".into());
                    }
                }
                _ => return Err("Invalid bundle receipt".into()),
            }
        }
        Ok(())
    }
}
impl State {
    pub fn checkpoint(&self) -> Option<Checkpoint> {
        Some(Checkpoint {
            intent: self.intent.clone()?,
            path: self.path.text().to_owned(),
            path_cursor: self.path.cursor(),
            workspace: self.workspace.text().to_owned(),
            workspace_cursor: self.workspace.cursor(),
            project: self.project.clone(),
            outcome: self.outcome.clone(),
        })
    }
    pub fn restore(&mut self, checkpoint: Checkpoint) {
        let sequence = self.sequence;
        *self = State::default();
        self.sequence = sequence;
        self.intent = Some(checkpoint.intent);
        self.path = restore_editor(&checkpoint.path, checkpoint.path_cursor);
        self.workspace = restore_editor(&checkpoint.workspace, checkpoint.workspace_cursor);
        self.project = checkpoint.project;
        self.outcome = checkpoint.outcome;
    }
}
fn restore_editor(text: &str, cursor: Cursor) -> Editor {
    let mut editor = Editor::bounded(4096, "bundle-path-invalid");
    editor.insert(text);
    let _ = editor.restore_cursor(cursor);
    editor.clear_history();
    editor
}
fn valid_text(text: &str, limit: usize) -> bool {
    text.len() <= limit && !text.chars().any(char::is_control)
}
pub(super) fn valid_path(path: &str) -> bool {
    valid_text(path, 4096)
        && api::decode_export(&serde_json::json!({
            "sessionId":"validate-path","destination":path,
        }))
        .is_ok()
}
