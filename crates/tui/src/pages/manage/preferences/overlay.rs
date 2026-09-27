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

use crate::editor::Editor;
use maka_protocol::configuration::{Patch, validation};
use serde_json::Value;

pub(super) struct Overlay {
    pub original: Option<Value>,
    pub editor: Editor,
}
impl Overlay {
    pub fn new(original: Option<Value>) -> Self {
        let mut editor = Editor::bounded(32 * 1024, "connection-preferences-limit");
        if let Some(value) = &original {
            editor.insert(&serde_json::to_string_pretty(value).expect("overlay value"));
        }
        editor.clear_history();
        Self { original, editor }
    }
    pub fn value(&self) -> Result<Option<Value>, &'static str> {
        if self.editor.error.is_some() {
            return Err("request-overlay-invalid");
        }
        if self.editor.text().trim().is_empty() {
            return Ok(None);
        }
        let value: Value =
            serde_json::from_str(self.editor.text()).map_err(|_| "request-overlay-invalid")?;
        if value.is_null() {
            return Ok(None);
        }
        validation::overlay(&value).map_err(|_| "request-overlay-invalid")?;
        Ok((!value.as_object().is_some_and(|object| object.is_empty())).then_some(value))
    }
    pub fn patch(&self) -> Result<Patch<Value>, &'static str> {
        Ok(match self.value()? {
            Some(value) => Patch::Set(value),
            None => Patch::Clear,
        })
    }
    pub fn changed(&self) -> bool {
        self.value().is_ok_and(|value| value != self.original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences197_overlay_edits_and_clears_only_valid_bounded_objects() {
        let mut overlay = Overlay::new(Some(
            serde_json::json!({"temperature":0.2,"extra":{"enabled":true}}),
        ));
        assert!(!overlay.changed());
        overlay.editor = Editor::bounded(32 * 1024, "connection-preferences-limit");
        overlay.editor.insert("{\"temperature\":0.5}");
        assert!(overlay.changed());
        assert!(
            matches!(overlay.patch().unwrap(), Patch::Set(value) if value["temperature"] == 0.5)
        );
        overlay.editor = Editor::bounded(32 * 1024, "connection-preferences-limit");
        assert!(matches!(overlay.patch().unwrap(), Patch::Clear));
        overlay.editor.insert("{\"__proto__\":{}}");
        assert!(overlay.patch().is_err());
    }
}
