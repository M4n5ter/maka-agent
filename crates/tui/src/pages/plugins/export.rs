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

//! Installed-package export stays on the existing plugin mutation driver.
use super::view::{button_row, review, text};
use super::*;
use crate::{
    editor::saved::Cursor,
    ui::{Node, Role, Tone},
    view::safe,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Exported {
    pub package: String,
    pub target: String,
    pub digest: String,
}
impl Exported {
    pub fn validate(&self) -> Result<(), String> {
        if maka_plugins::identifier(&self.package).is_err()
            || !valid_path(&self.target)
            || !valid_digest(&self.digest)
        {
            return Err("Invalid plugin export receipt".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Draft {
    pub package: String,
    pub path: String,
    pub cursor: Cursor,
}
impl Draft {
    pub fn validate(&self) -> Result<(), String> {
        if maka_plugins::identifier(&self.package).is_err()
            || self.path.len() > 4096
            || self.path.chars().any(char::is_control)
        {
            return Err("Invalid plugin export draft".into());
        }
        self.cursor.validate(&self.path)?;
        Ok(())
    }
    pub fn restore(self) -> (Place, drafts::Draft) {
        let mut draft = drafts::Draft::exporting(0);
        draft.fields[0].insert(&self.path);
        let _ = draft.fields[0].restore_cursor(self.cursor);
        draft.fields[0].clear_history();
        (Place::Export(self.package), draft)
    }
}
pub(super) fn valid_path(path: &str) -> bool {
    maka_protocol::plugin::decode_input(
        maka_protocol::Operation::PluginPackageExport,
        &serde_json::json!({
            "extensionId":"validate-path", "targetPath":path,
            "expected":{"baseGeneration":0,"contentDigest":format!("sha256-{}", "0".repeat(64))},
        }),
    )
    .is_ok()
}
pub(super) fn valid_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256-").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
impl State {
    pub(super) fn export_uncertain(&self, id: &str, path: &str) -> bool {
        self.unknown
            .iter()
            .any(|pending| pending.export_target(id) == Some(path))
    }
}
pub(super) fn rows(app: &App, snapshot: &Snapshot, id: &str, rows: &mut Vec<Node<Command>>) {
    let state = &app.plugins;
    if let Some(exported) = &state.exported
        && exported.package == id
    {
        rows.push(text(
            "exported",
            app.i18n
                .format("plugins-exported", &[("path", &safe(&exported.target))]),
            Tone::Success,
        ));
    }
    let Some(package) = snapshot.package(id) else {
        rows.push(text(
            "missing",
            app.i18n.text("plugins-missing"),
            Tone::Warning,
        ));
        return;
    };
    rows.push(text(
        "name",
        format!("{} · {}", safe(&package.display_name), safe(id)),
        Tone::Strong,
    ));
    rows.push(text(
        "source",
        app.i18n.text("plugins-export-installed"),
        Tone::Subtle,
    ));
    rows.push(text(
        "version",
        format!(
            "{}: {}",
            app.i18n.text("plugins-export-version"),
            package.content_digest
        ),
        Tone::Subtle,
    ));
    rows.push(text(
        "note",
        app.i18n.text("plugins-export-path-note"),
        Tone::Muted,
    ));
    if let Some(error) = state.draft().and_then(|draft| draft.fields[0].error) {
        rows.push(text("field-error", app.i18n.text(error), Tone::Warning));
    }
    super::forms::field(app, rows, 0, "plugins-export-target", 3);
    let path = state
        .draft()
        .map(|draft| draft.fields[0].text())
        .unwrap_or_default();
    rows.push(Node::row(
        "export-actions",
        vec![review(app, "export", Change::Export).enabled(
            !state.export_uncertain(id, path)
                && app.plugins_enabled(&Command::Review(state.token, Change::Export)),
        )],
    ));
    rows.push(button_row(
        app,
        "back",
        "plugins-export-back",
        Command::Visit(Place::Package(id.into())),
        Role::Normal,
    ));
}
