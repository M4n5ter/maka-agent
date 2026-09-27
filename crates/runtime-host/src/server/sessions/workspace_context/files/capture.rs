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

use super::{Result, Scope, api, stale, unreadable};
use maka_plugins::filesystem::entries::{self, Operation, ReadFile};
use maka_protocol::turn::{DirectoryReference, QuoteRef};

impl Scope {
    pub fn capture(self, input: api::Capture) -> Result<api::Captured> {
        if input.basis != self.basis {
            return Err(stale());
        }
        let (body, truncated, directory_reference) = match input.kind {
            api::Kind::File => {
                let entries::Output::Read(page) = entries::execute(
                    &self.root,
                    Operation::Read(ReadFile {
                        path: input.path.clone(),
                        offset: 0,
                        limit: api::CAPTURE_BYTES,
                    }),
                    &self.cancellation,
                    Some(entries::Policy {
                        root: &self.cwd,
                        filesystem: &self.policy,
                    }),
                )
                .map_err(unreadable)?
                else {
                    unreachable!()
                };
                let (text, cut) = text(page.bytes, page.next_offset.is_some())?;
                (
                    format!("Workspace file {}:\n{text}", serde_json::json!(input.path)),
                    page.next_offset.is_some() || cut,
                    None,
                )
            }
            api::Kind::Directory => {
                let entries = self.inventory(&input.path)?;
                let mut text = format!("Workspace directory {}:\n", serde_json::json!(input.path));
                let mut truncated = false;
                for entry in entries {
                    let row = format!(
                        "{}{}\n",
                        entry.path,
                        if entry.kind == api::Kind::Directory {
                            "/"
                        } else {
                            ""
                        }
                    );
                    if text.len() + row.len() > api::CAPTURE_BYTES {
                        truncated = true;
                        break;
                    }
                    text.push_str(&row);
                }
                let path = api::directory_path(&self.basis.workspace.host_cwd, &input.path);
                (
                    text,
                    truncated,
                    Some(DirectoryReference {
                        host_id: self.basis.root_id.clone(),
                        path,
                    }),
                )
            }
        };
        let mut body = body;
        if truncated {
            body.push_str("\n[Excerpt truncated]");
        }
        self.current()?;
        let label = input.path[..input.path.floor_char_boundary(200)].to_owned();
        Ok(api::Captured {
            content_digest: maka_runtime::artifact::content_digest(body.as_bytes()),
            basis: self.basis,
            path: input.path,
            kind: input.kind,
            truncated,
            directory_reference,
            quote: QuoteRef {
                text: body,
                label: (!label.is_empty()).then_some(label),
                source: None,
                source_turn_id: None,
            },
        })
    }
}

fn text(mut bytes: Vec<u8>, truncated: bool) -> Result<(String, bool)> {
    let mut cut = false;
    if let Err(error) = std::str::from_utf8(&bytes) {
        if error.error_len().is_some() || !truncated {
            return Err(unreadable("Only UTF-8 text files can be captured"));
        }
        bytes.truncate(error.valid_up_to());
        cut = true;
    }
    let text = String::from_utf8(bytes).map_err(unreadable)?;
    if text.contains('\0') {
        return Err(unreadable("Binary files require an attachment"));
    }
    Ok((text, cut))
}
