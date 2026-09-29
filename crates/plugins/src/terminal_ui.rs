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

//! Native terminal views share the Remote endpoint's exact registration.
//! The descriptor says where the shell presents a view and how it looks in
//! navigation; the view itself arrives as a component tree ([`view`]).

pub mod app;
mod command;
pub use command::Command;
pub mod page;
pub mod presenter;
pub mod transcript;
pub mod view;

use crate::Error;
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 9;

pub use maka_runtime::display::Text;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Context {
    Application,
    Session,
}

/// Where the shell presents a view. The shell decides order and look.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Placement {
    /// A page of its own: an application's in the sidebar, a session's
    /// opened from that session.
    #[default]
    Page,
    /// A panel of the session inspector.
    Panel,
    /// One quiet line above the session's composer.
    Status,
    /// A category of Settings.
    Settings,
    /// Inside other views that declare a slot of this name.
    Slot { name: String },
}

/// A navigation glyph, with the ASCII the shell uses when told to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Icon {
    pub glyph: String,
    pub ascii: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub version: u32,
    pub title: Text,
    pub context: Context,
    #[serde(default)]
    pub placement: Placement,
    #[serde(default)]
    pub icon: Option<Icon>,
    /// A stream of the same package whose items say the view is stale.
    #[serde(default)]
    pub changes: Option<String>,
    /// Lower first among views of the same placement.
    #[serde(default)]
    pub order: u16,
    /// Inert slash metadata; only Page placements can be opened independently.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<Command>,
}
impl Descriptor {
    pub fn new(title: Text, context: Context) -> Self {
        Self {
            version: VERSION,
            title,
            context,
            placement: Placement::Page,
            icon: None,
            changes: None,
            order: 0,
            commands: Vec::new(),
        }
    }
    pub fn placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }
    pub fn icon(mut self, glyph: &str, ascii: &str) -> Self {
        self.icon = Some(Icon {
            glyph: glyph.into(),
            ascii: ascii.into(),
        });
        self
    }
    pub fn changes(mut self, stream: &str) -> Self {
        self.changes = Some(stream.into());
        self
    }
    pub fn order(mut self, order: u16) -> Self {
        self.order = order;
        self
    }
    pub fn command(mut self, command: Command) -> Self {
        self.commands.push(command);
        self
    }
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = || Error::Invalid("Invalid terminal view descriptor".into());
        if self.version != VERSION {
            return Err(Error::Invalid("Unsupported terminal view version".into()));
        }
        self.title
            .validate()
            .map_err(|reason| Error::Invalid(reason.into()))?;
        if !self.commands.is_empty() && self.placement != Placement::Page {
            return Err(invalid());
        }
        let mut names = std::collections::BTreeSet::new();
        for command in &self.commands {
            command.validate()?;
            for name in std::iter::once(&command.name).chain(&command.aliases) {
                if !names.insert(name) {
                    return Err(Error::Invalid("Duplicate terminal command alias".into()));
                }
            }
        }
        if serde_json::to_vec(self)
            .map_err(|e| Error::Invalid(e.to_string()))?
            .len()
            > 64 * 1024
        {
            return Err(Error::Invalid("Terminal descriptor exceeds 64 KiB".into()));
        }
        match (&self.placement, self.context) {
            // A panel and a status belong to the session they sit in; a
            // settings category to the application.
            (Placement::Panel | Placement::Status, Context::Application)
            | (Placement::Settings, Context::Session) => return Err(invalid()),
            (Placement::Slot { name }, _) => view::identifier(name)?,
            _ => {}
        }
        if let Some(icon) = &self.icon {
            use unicode_width::UnicodeWidthStr;
            if icon.glyph.chars().count() > 2
                || !(1..=2).contains(&icon.glyph.width())
                || !view::safe(&icon.glyph, false)
                || icon.ascii.is_empty()
                || icon.ascii.len() > 2
                || !icon.ascii.bytes().all(|c| c.is_ascii_graphic())
            {
                return Err(invalid());
            }
        }
        if let Some(stream) = &self.changes {
            view::identifier(stream)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{Caller, Endpoint, Handler, Method, Stream, StreamProvider};
    use futures_util::future::BoxFuture;
    use serde_json::json;
    use std::sync::Arc;

    struct Uncalled;
    impl Method for Uncalled {
        fn call(
            &self,
            _: serde_json::Value,
            _: Caller,
        ) -> BoxFuture<'static, Result<serde_json::Value, crate::remote::Error>> {
            unreachable!("publishing metadata does not invoke the handler")
        }
    }
    impl StreamProvider for Uncalled {
        fn open(
            &self,
            _: serde_json::Value,
            _: Caller,
        ) -> BoxFuture<'static, Result<Box<dyn Stream>, crate::remote::Error>> {
            unreachable!("a stream cannot publish a terminal view")
        }
    }

    #[test]
    fn navigation_metadata_has_bounded_text_and_explicit_version_and_language_fallback() {
        let value = json!({"version":VERSION,"title":{
            "fallback":"Skills", "translations":{"en":"Skills","zh":"技能","zh-TW":"技能管理"}
        },"context":"session"});
        let descriptor: Descriptor = serde_json::from_value(value.clone()).unwrap();
        descriptor.validate().unwrap();
        assert_eq!(descriptor.title.resolve("zh-TW"), "技能管理");
        assert_eq!(descriptor.title.resolve("zh-CN"), "技能");
        assert_eq!(descriptor.title.resolve("ja-JP"), "Skills");
        let endpoint = Endpoint::standalone(Handler::Method(Arc::new(Uncalled)))
            .with_terminal_view(descriptor.clone())
            .unwrap();
        assert_eq!(endpoint.terminal_view(), Some(&descriptor));
        assert!(
            Endpoint::standalone(Handler::Stream(Arc::new(Uncalled)))
                .with_terminal_view(descriptor.clone())
                .is_err()
        );
        let mut invalid = descriptor.clone();
        invalid.title.translations = (0..9).map(|n| (format!("x-{n}"), "Title".into())).collect();
        assert!(invalid.validate().is_err());
        for (field, replacement) in [
            ("version", json!(VERSION + 1)),
            (
                "title",
                json!({"fallback":"\u{001b}[31m","translations":{}}),
            ),
            (
                "title",
                json!({"fallback":"\u{202e}spoof","translations":{}}),
            ),
            (
                "title",
                json!({"fallback":"x".repeat(257),"translations":{}}),
            ),
            (
                "title",
                json!({"fallback":"Skills","translations":{"zh--CN":"技能"}}),
            ),
        ] {
            let mut invalid = value.clone();
            invalid[field] = replacement;
            assert!(
                serde_json::from_value::<Descriptor>(invalid)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut invalid = value;
        invalid["execute"] = json!("not a navigation property");
        assert!(serde_json::from_value::<Descriptor>(invalid).is_err());
    }

    #[test]
    fn placements_bind_to_their_context_and_icons_are_one_narrow_glyph() {
        let session = Descriptor::new(Text::plain("Goal"), Context::Session);
        let application = Descriptor::new(Text::plain("Insights"), Context::Application);
        for descriptor in [
            session.clone().placement(Placement::Panel),
            session.clone().placement(Placement::Status),
            application.clone().placement(Placement::Settings),
            application
                .clone()
                .icon("◎", "o")
                .changes("changes")
                .order(2),
            session.clone().placement(Placement::Slot {
                name: "maka.workhub.assignment".into(),
            }),
        ] {
            descriptor.validate().unwrap();
        }
        for descriptor in [
            application.clone().placement(Placement::Panel),
            application.clone().placement(Placement::Status),
            session.clone().placement(Placement::Settings),
            application.clone().icon("abc", "a"),
            application.clone().icon("◎", ""),
            application.clone().icon("\u{1b}", "x"),
            application.clone().changes("bad\nname"),
        ] {
            assert!(descriptor.validate().is_err(), "{descriptor:?}");
        }
    }
}
