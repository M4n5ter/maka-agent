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

use maka_event_log::{
    EventLog,
    root::{ROOT_DATABASE, RootNamespaces, RootOwner},
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// Only lifecycle is shared; each scenario asserts its own canonical evidence.
pub(crate) struct HostFixture {
    _directory: tempfile::TempDir,
    namespaces: RootNamespaces,
    root: PathBuf,
    pub(crate) workspace: PathBuf,
}

impl HostFixture {
    pub(crate) fn owner(&self) -> RootOwner {
        RootOwner::open(&self.root, &self.namespaces).unwrap()
    }

    pub(crate) fn new(prefix: &str) -> Self {
        #[cfg(unix)]
        let directory = tempfile::Builder::new()
            .prefix(prefix)
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        #[cfg(windows)]
        let directory = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
        let namespaces = RootNamespaces {
            ownership: directory.path().join("owners"),
            control: directory.path().join("control"),
        };
        let root = directory.path().join("root");
        let workspace = directory.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let owner = RootOwner::create(&root, &namespaces).unwrap();
        drop(owner);
        Self {
            _directory: directory,
            namespaces,
            root,
            workspace,
        }
    }

    pub(crate) async fn log(&self) -> EventLog {
        EventLog::open(&self.root.join(ROOT_DATABASE))
            .await
            .unwrap()
    }
}
