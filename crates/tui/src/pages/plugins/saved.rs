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

/// Safe identity only. There is no durable Host operation receipt for these
/// operations, and intentionally no persisted payload or retry-original action.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pending {
    binding: Binding,
    token: Uuid,
    change: Change,
    place: Place,
    base: u64,
    package_digest: Option<String>,
    source_digest: Option<String>,
    payload_withheld: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    export_target: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pending: Vec<Pending>,
    withheld: Vec<Place>,
    #[serde(default)]
    exports: Vec<export::Draft>,
    #[serde(default)]
    exported: Option<export::Exported>,
}
impl Checkpoint {
    pub fn validate(&self, root: &str) -> Result<(), String> {
        if self.pending.len() > LIMIT
            || self.withheld.len() > LIMIT
            || self.exports.len() > LIMIT
            || self.pending.iter().any(|p| {
                p.binding.root != root
                    || p.binding.epoch.is_empty()
                    || p.binding.epoch.len() > 256
                    || !p.place.valid()
                    || !p.payload_withheld
                    || match (&p.change, &p.place, &p.export_target) {
                        (Change::Export, Place::Export(_), Some(path)) => {
                            !export::valid_path(path)
                                || p.package_digest
                                    .as_ref()
                                    .is_none_or(|digest| !export::valid_digest(digest))
                        }
                        (Change::Export, _, _) => true,
                        (_, _, Some(_)) => true,
                        _ => false,
                    }
                    || p.package_digest
                        .iter()
                        .chain(p.source_digest.iter())
                        .any(|digest| digest.len() != 71 || !digest.starts_with("sha256-"))
            })
            || self.withheld.iter().any(|p| !p.valid())
        {
            return Err("Invalid plugin management checkpoint".into());
        }
        let mut seen = std::collections::HashSet::new();
        for draft in &self.exports {
            draft.validate()?;
            if !seen.insert(&draft.package) {
                return Err("Duplicate plugin export draft".into());
            }
        }
        if let Some(exported) = &self.exported {
            exported.validate()?;
        }
        Ok(())
    }
}
impl Pending {
    fn of(request: &Request) -> Option<Self> {
        let (change, place, base) = request.intent()?;
        let (package_digest, source_digest) = match request.mutation()? {
            io::Mutation::Install(input) => (
                input
                    .expected
                    .as_ref()
                    .and_then(|expected| expected.content_digest.clone()),
                input.source_digest.clone(),
            ),
            io::Mutation::Restart(input) | io::Mutation::Uninstall(input) => (
                input
                    .expected
                    .as_ref()
                    .and_then(|expected| expected.content_digest.clone()),
                None,
            ),
            io::Mutation::Apply(_) => (None, None),
            io::Mutation::Export(input) => (input.expected.content_digest.clone(), None),
        };
        Some(Self {
            package_digest,
            source_digest,
            payload_withheld: true,
            export_target: match request.mutation()? {
                io::Mutation::Export(input) => Some(input.target_path.clone()),
                _ => None,
            },
            binding: request.binding.clone(),
            token: request.token,
            change,
            place: place.clone(),
            base,
        })
    }
    pub(super) fn export_target(&self, package: &str) -> Option<&str> {
        if matches!(&self.place, Place::Export(id) if id == package) {
            self.export_target.as_deref()
        } else {
            None
        }
    }
    pub(super) fn export_location(&self) -> Option<&str> {
        self.export_target.as_deref()
    }
    pub(super) fn description(&self, app: &App) -> String {
        let mut description = format!(
            "{} · {} · {} · {} · {}",
            self.token,
            self.binding.epoch,
            self.base,
            app.i18n.text(self.change.label()),
            match &self.place {
                Place::Package(id) | Place::New(id) | Place::Export(id) => id.clone(),
                place => place
                    .entry()
                    .map(|key| format!("{} / {}", String::from(key.scope.clone()), key.id))
                    .unwrap_or_default(),
            }
        );
        if let Some(path) = &self.export_target {
            description.push_str(&format!(
                "\n{path}\n{}",
                self.package_digest.as_deref().unwrap_or_default()
            ));
        }
        description
    }
}
impl State {
    pub fn checkpoint(&self) -> Checkpoint {
        let mut pending = self.unknown.clone();
        if let Some(request) = self.pending.as_ref().or(self.queued.as_ref())
            && let Some(item) = Pending::of(request)
        {
            pending.push(item);
        }
        let mut withheld = self.withheld.clone();
        for (place, draft) in &self.drafts {
            if draft.dirty.iter().any(|dirty| *dirty)
                && !matches!(place, Place::Export(_))
                && !withheld.contains(place)
                && withheld.len() < LIMIT
            {
                withheld.push(place.clone());
            }
        }
        let exports = self
            .drafts
            .iter()
            .filter_map(|(place, draft)| {
                let Place::Export(package) = place else {
                    return None;
                };
                Some(export::Draft {
                    package: package.clone(),
                    path: draft.fields[0].text().into(),
                    cursor: draft.fields[0].cursor(),
                })
            })
            .collect();
        Checkpoint {
            pending,
            withheld,
            exports,
            exported: self.exported.clone(),
        }
    }
    pub fn restore(&mut self, saved: Checkpoint) {
        self.unknown = saved.pending;
        self.withheld = saved.withheld;
        self.drafts
            .extend(saved.exports.into_iter().map(export::Draft::restore));
        self.exported = saved.exported;
    }
    pub(super) fn remember_unknown(&mut self, request: &Request) {
        if self.unknown.len() < LIMIT
            && !self.unknown.iter().any(|p| p.token == request.token)
            && let Some(pending) = Pending::of(request)
        {
            self.unknown.push(pending);
        }
    }
}
