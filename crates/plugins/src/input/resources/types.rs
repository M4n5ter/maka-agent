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

use crate::{Error, terminal_ui::Text};
use maka_runtime::input::{QuoteRef, SelectionSource};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub title: Text,
}
impl Descriptor {
    pub fn validate(&self) -> Result<(), Error> {
        self.title
            .validate()
            .map_err(|reason| Error::Invalid(reason.into()))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub query: String,
    pub cursor: Option<String>,
    pub limit: usize,
    pub locale: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolve {
    pub id: String,
    pub locale: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Query {
        query: String,
        #[serde(default)]
        cursor: Option<String>,
        limit: usize,
        locale: String,
    },
    Resolve {
        id: String,
        locale: String,
    },
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Query {
                query,
                cursor,
                limit,
                locale,
            } => {
                text(query, 1024, true)?;
                if !(1..=64).contains(limit) {
                    return Err(invalid("Invalid resource page size"));
                }
                if let Some(cursor) = cursor {
                    text(cursor, 4096, false)?;
                }
                text(locale, 64, false)
            }
            Self::Resolve { id, locale } => {
                text(id, 512, false)?;
                text(locale, 64, false)
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    pub items: Vec<Item>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
impl Page {
    pub fn validate(&self, limit: usize) -> Result<(), Error> {
        if self.items.len() > limit {
            return Err(invalid("Resource page exceeds requested size"));
        }
        let mut seen = BTreeSet::new();
        for item in &self.items {
            text(&item.id, 512, false)?;
            text(&item.title, 256, false)?;
            if !seen.insert(&item.id) {
                return Err(invalid("Duplicate resource item"));
            }
            if let Some(detail) = &item.description {
                text(detail, 2048, true)?;
            }
        }
        if let Some(cursor) = &self.next_cursor {
            text(cursor, 4096, false)?;
        }
        budget(self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Value {
    pub selector: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<QuoteRef>,
}
impl Value {
    pub fn validate(&self) -> Result<(), Error> {
        text(&self.selector, 512, false)?;
        text(&self.label, 256, false)?;
        if let Some(quote) = &self.quote {
            quote_text(&quote.text, 32000, true)?;
            if let Some(label) = &quote.label {
                quote_text(label, 200, false)?;
            }
            if let Some(turn) = &quote.source_turn_id {
                entity(turn)?;
            }
            if let Some(source) = &quote.source {
                entity(&source.session_id)?;
                quote_text(&source.session_name, 200, false)?;
            }
            let content = serde_json::json!({"text":"","quotes":[quote]});
            if serde_json::to_vec(&content)
                .map_err(|e| invalid(&e.to_string()))?
                .len()
                > 52 * 1024
            {
                return Err(invalid("Captured resource quote exceeds input byte limit"));
            }
        }
        budget(self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Reply {
    Page {
        items: Vec<Item>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_cursor: Option<String>,
    },
    Resolved {
        source: Box<SelectionSource>,
        selector: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quote: Option<QuoteRef>,
    },
}
pub(super) fn budget(value: &impl Serialize) -> Result<(), Error> {
    if serde_json::to_vec(value)
        .map_err(|e| invalid(&e.to_string()))?
        .len()
        > 64 * 1024
    {
        return Err(invalid("Input resource reply exceeds 64 KiB"));
    }
    Ok(())
}
fn text(value: &str, max: usize, empty: bool) -> Result<(), Error> {
    if (!empty && value.is_empty())
        || value.len() > max
        || !crate::terminal_ui::view::safe(value, false)
    {
        return Err(invalid("Invalid input resource text"));
    }
    Ok(())
}
fn quote_text(value: &str, max: usize, multiline: bool) -> Result<(), Error> {
    if value.is_empty()
        || value.encode_utf16().count() > max
        || !crate::terminal_ui::view::safe(value, multiline)
    {
        return Err(invalid("Invalid captured resource quote"));
    }
    Ok(())
}
fn entity(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(invalid("Invalid captured resource source ID"));
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn captured_quotes_fit_the_input_utf16_identity_and_encoded_limits() {
        let original = json!({
            "selector":"record:1", "label":"Record",
            "quote":{
                "text":"界".repeat(16000), "label":"😀".repeat(100),
                "sourceTurnId":"turn-1",
                "source":{"sessionId":"session_1","sessionName":"🧭".repeat(100),
                    "capturedAt":0,"truncated":false}
            }
        });
        let value: Value = serde_json::from_value(original.clone()).unwrap();
        value.validate().unwrap();
        for (pointer, replacement) in [
            ("/quote/text", json!("x".repeat(32001))),
            ("/quote/label", json!("😀".repeat(101))),
            ("/quote/label", json!("")),
            ("/quote/source/sessionName", json!("🧭".repeat(101))),
            ("/quote/source/sessionName", json!("")),
            ("/quote/sourceTurnId", json!("x".repeat(129))),
            ("/quote/source/sessionId", json!("session.invalid")),
        ] {
            let mut bad = original.clone();
            *bad.pointer_mut(pointer).unwrap() = replacement;
            let value: Value = serde_json::from_value(bad).unwrap();
            assert!(value.validate().is_err(), "{pointer}");
        }
        for text in ["界".repeat(18000), "\n".repeat(27000)] {
            let value: Value = serde_json::from_value(json!({
                "selector":"record:1", "label":"Record", "quote":{"text":text}
            }))
            .unwrap();
            budget(&value).unwrap();
            assert!(
                value.validate().is_err(),
                "a valid Remote reply must still fit the smaller input envelope"
            );
        }
    }
}
