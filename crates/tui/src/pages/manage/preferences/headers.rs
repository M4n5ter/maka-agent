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

use super::SecretAction;
use crate::editor::Editor;
use maka_protocol::configuration::headers::{
    RequestHeaderUpdate, RequestHeadersBasis, RequestHeadersReplace,
};

pub(super) struct Headers {
    pub basis: RequestHeadersBasis,
    pub rows: Vec<Row>,
}
pub(super) struct Row {
    pub name: Editor,
    pub value: Editor,
    pub original: Option<String>,
    pub action: SecretAction,
}
impl Headers {
    pub fn new(basis: RequestHeadersBasis, names: Vec<String>) -> Self {
        Self {
            basis,
            rows: names.into_iter().map(|name| Row::new(Some(name))).collect(),
        }
    }
    pub fn update(&self) -> Result<RequestHeadersReplace, &'static str> {
        let mut headers = Vec::new();
        for row in &self.rows {
            if row.action == SecretAction::Delete {
                continue;
            }
            if row.name.error.is_some() || row.value.error.is_some() {
                return Err("connection-preferences-invalid");
            }
            if row.action == SecretAction::Keep && row.original.as_deref() != Some(row.name.text())
            {
                return Err("headers-value-required");
            }
            headers.push(RequestHeaderUpdate {
                name: row.name.text().into(),
                value: (row.action == SecretAction::Replace).then(|| row.value.text().to_owned()),
            });
        }
        let mut update = RequestHeadersReplace {
            expected: self.basis.clone(),
            headers,
        };
        update
            .normalize()
            .map_err(|_| "connection-preferences-invalid")?;
        Ok(update)
    }
    pub fn changed(&self) -> bool {
        self.update().is_ok() && self.rows.iter().any(|r| r.action != SecretAction::Keep)
    }
    pub fn clear_secrets(&mut self) {
        for row in &mut self.rows {
            row.value = Editor::bounded(16 * 1024, "connection-preferences-limit");
        }
    }
    pub fn invalidate_geometry(&mut self) {
        for row in &mut self.rows {
            row.name.invalidate_geometry();
            row.value.invalidate_geometry();
        }
    }
}
impl Row {
    pub fn new(original: Option<String>) -> Self {
        let mut name = Editor::bounded(128, "connection-preferences-limit");
        name.insert(original.as_deref().unwrap_or(""));
        name.clear_history();
        Self {
            name,
            value: Editor::bounded(16 * 1024, "connection-preferences-limit"),
            action: if original.is_some() {
                SecretAction::Keep
            } else {
                SecretAction::Replace
            },
            original,
        }
    }
}
