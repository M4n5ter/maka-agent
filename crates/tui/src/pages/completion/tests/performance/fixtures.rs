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
use maka_plugins::{
    remote::Target,
    terminal_ui::{Command as Slash, Context, Descriptor, Text},
};
use maka_protocol::{
    plugin::{InputResourceProjection, TerminalViewProjection},
    session::workspace_context as workspace,
};
use maka_runtime::scope::Scope;

pub(super) fn commands(count: usize) -> Vec<TerminalViewProjection> {
    (0..count.div_ceil(io::PAGE))
        .map(|page| {
            let mut descriptor = Descriptor::new(
                Text::plain(format!("Fixture page {page}")),
                Context::Session,
            );
            descriptor.commands = (page * io::PAGE..((page + 1) * io::PAGE).min(count))
                .map(|index| Slash {
                    name: format!("entry-{index:05}"),
                    aliases: vec![],
                    title: Text::plain(format!("Entry {index} 中文 🦀")),
                    description: Text::plain("Open the existing fixture page"),
                    route: json!({"entry":index}),
                })
                .collect();
            descriptor.validate().unwrap();
            TerminalViewProjection {
                package_id: "performance.commands".into(),
                scope_id: Scope::Profile,
                method: format!("page-{page}"),
                target: Target {
                    entry_id: "performance".into(),
                    activation: "one".into(),
                    registration: uuid::Uuid::new_v4(),
                },
                descriptor,
            }
        })
        .collect()
}

pub(super) fn workspace_page(request: &Request, count: usize, offset: usize) -> Output {
    let io::Job::Workspace(query) = &request.job else {
        panic!("expected Workspace request")
    };
    assert_eq!(
        query.cursor.as_ref().map(|cursor| cursor.after.as_str()),
        offset
            .checked_sub(1)
            .map(|previous| format!("file-{previous:05}-中文.rs"))
            .as_deref()
    );
    let basis = workspace::Basis {
        root_id: request.context.root.clone(),
        session_id: request.context.session.clone(),
        boundary_revision: 1,
        workspace: crate::pages::sessions::tests::item("chat").workspace,
        directory_identity: format!("sha256:{}", "a".repeat(64)).try_into().unwrap(),
    };
    let end = (offset + io::PAGE).min(count);
    let entries = (offset..end)
        .map(|index| workspace::Entry {
            path: format!("file-{index:05}-中文.rs"),
            kind: workspace::Kind::File,
        })
        .collect();
    Output::Workspace(workspace::Page {
        basis: basis.clone(),
        directory: query.directory.clone(),
        filter: query.filter.clone(),
        revision: "a".repeat(64),
        entries,
        next_cursor: (end < count).then(|| workspace::Cursor {
            basis,
            revision: "a".repeat(64),
            after: format!("file-{:05}-中文.rs", end - 1),
        }),
    })
}

pub(super) fn provider() -> InputResourceProjection {
    InputResourceProjection {
        provider: "performance.records".into(),
        package_id: "performance.records".into(),
        scope_id: Scope::Profile,
        method: "resources".into(),
        target: Target {
            entry_id: "records".into(),
            activation: "original".into(),
            registration: uuid::Uuid::new_v4(),
        },
        descriptor: maka_plugins::input::resources::Descriptor {
            title: Text::plain("Controlled pending source"),
        },
    }
}
pub(super) fn stale_resources() -> Output {
    Output::Resources {
        items: vec![maka_plugins::input::resources::Item {
            id: "original".into(),
            title: "Retired candidate".into(),
            description: None,
        }],
        next_cursor: None,
    }
}
