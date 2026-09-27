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

use crate::{
    app::Action,
    editor::{Editor, completion::Token},
};
use maka_protocol::{
    plugin::{CommandProjection, InputResourceProjection},
    session::workspace_context as workspace,
};
use maka_runtime::input::{DirectoryReference, QuoteRef, SelectionSource};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftKey {
    pub session: String,
    pub input: Option<String>,
    #[serde(default)]
    pub display: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Context {
    pub root: String,
    pub epoch: String,
    pub draft: DraftKey,
    pub session: String,
    pub token: Token,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub label: String,
    pub origin: Origin,
    pub inline: Option<maka_runtime::input::InlineReferenceKind>,
    pub payload: Payload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Payload {
    Context {
        quote: QuoteRef,
        directory: Option<DirectoryReference>,
    },
    Selection {
        provider: String,
        selector: String,
        source: Option<SelectionSource>,
        quote: Option<QuoteRef>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Origin {
    Workspace,
    Session { name: String },
    Skill,
    Plugin { title: String, package: String },
}

pub type Bindings = BTreeMap<DraftKey, BTreeMap<String, Binding>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Commands,
    Skills,
    Workspace,
    Sessions,
    Plugins,
    Added,
}
impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Commands => "completion-commands",
            Self::Skills => "skills-title",
            Self::Workspace => "completion-workspace",
            Self::Sessions => "completion-sessions",
            Self::Plugins => "completion-plugins",
            Self::Added => "completion-added",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Source {
    Commands,
    Skills,
    Workspace { directory: String },
    Sessions,
    Messages { session: String, name: String },
    Providers,
    Added,
    Plugin(InputResourceProjection),
}
impl Source {
    pub fn category(&self) -> Category {
        match self {
            Self::Commands => Category::Commands,
            Self::Skills => Category::Skills,
            Self::Workspace { .. } => Category::Workspace,
            Self::Sessions | Self::Messages { .. } => Category::Sessions,
            Self::Providers | Self::Plugin(_) => Category::Plugins,
            Self::Added => Category::Added,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Pick {
    Native(Action),
    Command(CommandProjection),
    Skill {
        id: String,
        name: String,
    },
    Workspace(workspace::Capture),
    Session {
        id: String,
        name: String,
    },
    Provider(InputResourceProjection),
    Resource {
        provider: InputResourceProjection,
        id: String,
    },
    Captured(Binding),
    Bound(String),
}

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub source: String,
    pub enabled: bool,
    pub pick: Pick,
}

#[derive(Clone, Debug)]
pub(super) enum Cursor {
    Workspace(workspace::Cursor),
    Catalog { revision: String, cursor: String },
    Skills { revision: String, cursor: String },
    Providers(String),
    Resource(String),
    Messages(u64),
    Local(usize),
}

pub(super) struct Popup {
    pub context: Context,
    pub generation: u64,
    pub source: Source,
    pub explicit: bool,
    pub controls: bool,
    pub query: Editor,
    pub candidates: Vec<Candidate>,
    pub selected: Option<String>,
    pub next: Option<Cursor>,
    pub previous: Vec<Option<Cursor>>,
    pub cursor: Option<Cursor>,
    pub scan: Option<Cursor>,
    pub error: Option<String>,
    pub preview: Option<Binding>,
    pub surface: crate::ui::Surface<super::Command>,
    pub query_area: Option<ratatui::layout::Rect>,
    pub area: Option<ratatui::layout::Rect>,
    pub requested: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReferenceKey {
    Mark(String),
    Original { provider: String, selector: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionTarget {
    pub draft: DraftKey,
    pub key: ReferenceKey,
}
#[derive(Clone)]
pub(super) struct Reselection {
    pub target: SelectionTarget,
    pub provider: String,
    pub selector: String,
}
