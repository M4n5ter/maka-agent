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

mod capture;

use super::{Code, Result, SessionConfiguration, api, failure, stale, unreadable};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use maka_runtime::execution::SandboxMode;
use maka_sandbox::{Sandbox, filesystem::Compiled};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const INVENTORY_BYTES: usize = 8 * 1024 * 1024;
pub(super) struct Scope {
    root: Dir,
    basis: api::Basis,
    policy: Compiled,
    cwd: PathBuf,
    cancellation: CancellationToken,
    deadline: Instant,
}
impl Scope {
    pub fn open(
        root_id: String,
        session_id: String,
        configuration: SessionConfiguration,
        state: PathBuf,
        control: PathBuf,
        cancellation: CancellationToken,
    ) -> Result<Self> {
        let (cwd, directory_identity) =
            maka_fs_tools::directory::capture(Path::new(&configuration.workspace.host_cwd))
                .map_err(unreadable)?;
        let spelling = maka_fs_tools::workspace::project::host_path(&cwd).map_err(unreadable)?;
        if spelling != configuration.workspace.host_cwd {
            return Err(stale());
        }
        // A discovery callback cannot expose Host state even for an unrestricted
        // execution. Allocated workspace exceptions stay with the existing policy.
        if cwd.starts_with(&control) {
            return Err(failure(
                Code::OperationUnavailable,
                "Host control state is not workspace context",
            ));
        }
        let (Sandbox::Managed { mut filesystem, .. }, _) = crate::execution::permissions::resolve(
            SandboxMode::ReadOnly,
            Path::new(spelling),
            &state,
            configuration.workspace_origin,
        )
        .map_err(unreadable)?
        else {
            return Err(failure(
                Code::InternalFailure,
                "Workspace read policy is missing",
            ));
        };
        let control = maka_fs_tools::workspace::project::host_path(&control).map_err(unreadable)?;
        filesystem
            .rules
            .push(maka_sandbox::filesystem::Rule::subtree(
                control,
                maka_sandbox::filesystem::Access::Deny,
            ));
        let root = maka_fs_tools::directory::open(&cwd, &directory_identity).map_err(unreadable)?;
        // Compiled policies use Host spelling, not Windows' verbatim prefix.
        let cwd = PathBuf::from(spelling);
        Ok(Self {
            root,
            cwd,
            policy: filesystem.compile().map_err(unreadable)?,
            basis: api::Basis {
                root_id,
                session_id,
                boundary_revision: configuration.boundary_revision,
                workspace: configuration.workspace,
                directory_identity,
            },
            cancellation,
            deadline: Instant::now() + Duration::from_secs(5),
        })
    }
    fn check(&self) -> Result<()> {
        if self.cancellation.is_cancelled() || Instant::now() >= self.deadline {
            return Err(failure(
                Code::OperationUnavailable,
                "Workspace read cancelled or timed out",
            ));
        }
        Ok(())
    }
    fn current(&self) -> Result<()> {
        self.check()?;
        maka_fs_tools::directory::open(&self.cwd, &self.basis.directory_identity)
            .map_err(|_| stale())?;
        Ok(())
    }
    fn directory(&self, path: &str) -> Result<Dir> {
        self.check()?;
        api::relative(path, true).map_err(unreadable)?;
        let mut directory = self.root.try_clone().map_err(unreadable)?;
        if !self.policy.access(&self.cwd.join(path)).can_read() {
            return Err(failure(
                Code::OperationUnavailable,
                "Workspace policy denies this directory",
            ));
        }
        for segment in path.split('/').filter(|part| !part.is_empty()) {
            directory = directory.open_dir_nofollow(segment).map_err(unreadable)?;
        }
        Ok(directory)
    }
    fn inventory(&self, directory: &str) -> Result<Vec<api::Entry>> {
        let handle = self.directory(directory)?;
        let mut entries = Vec::new();
        let mut bytes = 0usize;
        for entry in handle.entries().map_err(unreadable)? {
            self.check()?;
            let entry = entry.map_err(unreadable)?;
            let Ok(name) = entry.file_name().into_string() else {
                // Wire paths must retain exact identity; skip unaddressable names.
                continue;
            };
            let path = if directory.is_empty() {
                name
            } else {
                format!("{directory}/{name}")
            };
            if api::relative(&path, false).is_err()
                || !self.policy.access(&self.cwd.join(&path)).can_read()
            {
                continue;
            }
            let kind = entry.file_type().map_err(unreadable)?;
            let kind = if kind.is_file() {
                api::Kind::File
            } else if kind.is_dir() {
                api::Kind::Directory
            } else {
                continue;
            };
            bytes = bytes.saturating_add(path.len().saturating_mul(6) + 128);
            if bytes > INVENTORY_BYTES {
                return Err(failure(
                    Code::OperationUnavailable,
                    "Directory inventory exceeds the read byte budget",
                ));
            }
            entries.push(api::Entry { path, kind });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        self.current()?;
        Ok(entries)
    }
    pub fn query(self, input: api::Query) -> Result<api::Page> {
        if input
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.basis != self.basis)
        {
            return Err(stale());
        }
        let all = self.inventory(&input.directory)?;
        let revision = maka_runtime::artifact::content_digest(
            &serde_json::to_vec(&(&self.basis, &input.directory, &input.filter, &all))
                .map_err(unreadable)?,
        );
        if input
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.revision != revision)
        {
            return Err(stale());
        }
        if input
            .cursor
            .as_ref()
            .is_some_and(|cursor| !all.iter().any(|entry| entry.path == cursor.after))
        {
            return Err(failure(
                Code::InvalidRequest,
                "Workspace continuation is outside its inventory",
            ));
        }
        let filter = input.filter.to_lowercase();
        let mut remaining = all.into_iter().filter(|entry| {
            input
                .cursor
                .as_ref()
                .is_none_or(|cursor| entry.path > cursor.after)
                && entry
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&filter)
        });
        let mut entries: Vec<_> = remaining.by_ref().take(128).collect();
        let mut more = remaining.next().is_some();
        loop {
            let next_cursor = more.then(|| api::Cursor {
                basis: self.basis.clone(),
                revision: revision.clone(),
                after: entries.last().expect("nonempty page").path.clone(),
            });
            let page = api::Page {
                basis: self.basis.clone(),
                directory: input.directory.clone(),
                filter: input.filter.clone(),
                revision: revision.clone(),
                entries: entries.clone(),
                next_cursor,
            };
            if serde_json::to_vec(&page).map_err(unreadable)?.len() <= api::PAGE_BYTES {
                self.current()?;
                return Ok(page);
            }
            if entries.len() <= 1 {
                return Err(failure(
                    Code::OperationUnavailable,
                    "Workspace candidate exceeds page byte budget",
                ));
            }
            entries.pop();
            more = true;
        }
    }
}

#[cfg(test)]
mod tests;
