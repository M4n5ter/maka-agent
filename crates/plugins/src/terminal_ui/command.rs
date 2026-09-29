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

//! A slash command opens an existing app route; it grants no execution authority.
use super::Text;
use crate::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Command {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub title: Text,
    pub description: Text,
    pub route: Value,
}
impl Command {
    pub fn validate(&self) -> Result<(), Error> {
        self.title
            .validate()
            .map_err(|reason| Error::Invalid(reason.into()))?;
        self.description
            .validate()
            .map_err(|reason| Error::Invalid(reason.into()))?;
        super::view::route(&self.route)?;
        let mut seen = BTreeSet::new();
        for name in std::iter::once(&self.name).chain(&self.aliases) {
            crate::identifier(name)?;
            if !seen.insert(name) {
                return Err(Error::Invalid("Duplicate terminal command alias".into()));
            }
        }
        if serde_json::to_vec(self)
            .map_err(|e| Error::Invalid(e.to_string()))?
            .len()
            > 64 * 1024
        {
            return Err(Error::Invalid("Terminal command exceeds 64 KiB".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_ui::{Context, Descriptor, Placement};

    #[test]
    fn commands_are_page_routes_with_byte_bounds_not_inventory_limits() {
        let mut descriptor = Descriptor::new(Text::plain("Example"), Context::Application);
        for index in 0..96 {
            descriptor = descriptor.command(Command {
                name: format!("command-{index}"),
                aliases: Vec::new(),
                title: Text::plain(format!("Command {index}")),
                description: Text::plain("Open the existing form"),
                route: serde_json::json!({"form":index}),
            });
        }
        descriptor.validate().unwrap();
        let mut duplicate = descriptor.clone();
        duplicate.commands[1].aliases.push("command-0".into());
        assert!(duplicate.validate().is_err());
        let mut embedded = descriptor.clone();
        embedded.placement = Placement::Slot {
            name: "example.detail".into(),
        };
        assert!(embedded.validate().is_err());
        for invalid in [
            Value::String("x".repeat(8192)),
            Value::String("\u{1b}[2J".into()),
            Value::Array(vec![Value::Null; 128]),
        ] {
            descriptor.commands[0].route = invalid;
            assert!(
                descriptor.validate().is_err(),
                "commands must be usable Read routes"
            );
        }
    }
}
