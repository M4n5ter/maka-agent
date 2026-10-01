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

use super::{Choice, LoginStart, LoginTarget, State};
use crate::editor::Editor;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use maka_protocol::configuration::{ConnectionCredentialTarget, validation};
use serde_json::Value;

pub(super) struct Identity {
    pub expanded: bool,
    pub fields: [Editor; 4],
    authentication: Authentication,
    configuration_key: Option<(String, bool)>,
}

enum Authentication {
    Empty,
    Text { key: String, required: bool },
    Json,
}
impl Default for Identity {
    fn default() -> Self {
        let mut fields = [
            Editor::bounded(1024, "oauth-name-invalid"),
            Editor::bounded(64, "oauth-slug-invalid"),
            Editor::bounded(64 * 1024, "oauth-field-invalid"),
            Editor::bounded(64 * 1024, "oauth-field-invalid"),
        ];
        fields[2].insert("{}");
        fields[3].insert("{}");
        Self {
            expanded: false,
            fields,
            authentication: Authentication::Json,
            configuration_key: None,
        }
    }
}
impl Identity {
    pub fn clear_authentication(&mut self) {
        self.fields[3] = Editor::bounded(64 * 1024, "oauth-field-invalid");
        if !matches!(self.authentication, Authentication::Text { .. }) {
            self.fields[3].insert("{}");
        }
    }
    pub fn configure(&mut self, choice: &Choice, existing: Option<&ConnectionCredentialTarget>) {
        let schema = &choice.authentication().input_schema;
        self.authentication = match schema["properties"].as_object() {
            Some(properties)
                if properties.is_empty() && schema["additionalProperties"] == false =>
            {
                Authentication::Empty
            }
            Some(properties) if properties.len() == 1 => {
                let (key, property) = properties.iter().next().expect("one field");
                if property["type"] == "string" {
                    Authentication::Text {
                        key: key.clone(),
                        required: schema["required"]
                            .as_array()
                            .is_some_and(|required| required.iter().any(|value| value == key)),
                    }
                } else {
                    Authentication::Json
                }
            }
            _ => Authentication::Json,
        };
        let configuration = existing.map_or(
            &choice.provider.descriptor.configuration_defaults,
            |target| &target.configuration,
        );
        let configuration_schema = &choice.provider.descriptor.configuration_schema;
        self.configuration_key = configuration_schema["properties"]
            .as_object()
            .filter(|properties| {
                properties.len() == 1 && configuration_schema["additionalProperties"] == false
            })
            .and_then(|properties| properties.iter().next())
            .filter(|(key, schema)| {
                schema["type"] == "string"
                    && configuration
                        .as_object()
                        .is_some_and(|values| values.keys().all(|name| name == *key))
            })
            .map(|(key, _)| {
                (
                    key.clone(),
                    configuration_schema["required"]
                        .as_array()
                        .is_some_and(|required| required.iter().any(|v| v == key)),
                )
            });
        self.fields[2] = Editor::bounded(64 * 1024, "oauth-field-invalid");
        self.fields[2].insert(&match &self.configuration_key {
            Some((key, _)) => configuration[key].as_str().unwrap_or_default().to_owned(),
            None => configuration.to_string(),
        });
        // Show a long configuration from its start, not scrolled to its end.
        self.fields[2].key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
        self.clear_authentication();
    }

    pub fn configuration_key(&self) -> Option<&str> {
        self.configuration_key.as_ref().map(|(key, _)| key.as_str())
    }

    fn configuration(&self) -> Result<Value, &'static str> {
        if let Some((key, required)) = &self.configuration_key {
            let value = self.fields[2].text().trim();
            if *required && value.is_empty() {
                return Err(if key == "baseUrl" {
                    "connection-base-url-required"
                } else {
                    "onboard-configuration-required"
                });
            }
            Ok(if value.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::json!({(key):value})
            })
        } else {
            serde_json::from_str(self.fields[2].text()).map_err(|_| "oauth-field-invalid")
        }
    }

    pub fn authentication_field(&self) -> Option<&str> {
        match &self.authentication {
            Authentication::Empty => None,
            Authentication::Text { key, .. } => Some(key),
            Authentication::Json => Some("JSON"),
        }
    }

    fn authentication_input(&self) -> Result<Value, &'static str> {
        let text = self.fields[3].text();
        match &self.authentication {
            Authentication::Empty => Ok(serde_json::json!({})),
            Authentication::Text { key, required } => {
                if text.is_empty() {
                    if *required {
                        Err("connection-authentication-required")
                    } else {
                        Ok(serde_json::json!({}))
                    }
                } else {
                    Ok(serde_json::json!({(key): text}))
                }
            }
            Authentication::Json => serde_json::from_str(text).map_err(|_| "oauth-field-invalid"),
        }
    }

    pub fn start(
        &self,
        choice: &Choice,
        existing: Option<&ConnectionCredentialTarget>,
    ) -> Result<LoginStart, &'static str> {
        if let Some(error) = self.error() {
            return Err(error);
        }
        let configuration = self.configuration()?;
        let target = if let Some(expected) = existing {
            LoginTarget::Existing {
                expected: expected.clone(),
                configuration,
            }
        } else {
            let name = self.fields[0].text().trim();
            let slug = self.fields[1].text().trim();
            LoginTarget::Create {
                provider: choice.provider.identity.clone(),
                configuration,
                name: if name.is_empty() {
                    choice.provider.descriptor.label.clone()
                } else {
                    name.into()
                },
                slug: if slug.is_empty() {
                    format!("connection-{}", uuid::Uuid::new_v4().simple())
                } else {
                    slug.into()
                },
            }
        };
        let input = LoginStart {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            target,
            authentication: maka_protocol::oauth::AuthenticationInput {
                method: choice.authentication().id.clone(),
                input: self.authentication_input()?,
            },
        };
        input.validate().map_err(|_| "oauth-field-invalid")?;
        Ok(input)
    }

    pub fn error(&self) -> Option<&'static str> {
        if let Some(error) = self.fields.iter().find_map(|field| field.error) {
            return Some(error);
        }
        let name = self.fields[0].text().trim();
        if !name.is_empty() && validation::text(name, 256, false).is_err() {
            return Some("oauth-name-invalid");
        }
        let slug = self.fields[1].text().trim();
        if !slug.is_empty() && validation::slug(slug).is_err() {
            return Some("oauth-slug-invalid");
        }
        let configuration = match self.configuration() {
            Ok(value) => value,
            Err(error) => return Some(error),
        };
        if validation::provider_configuration(&configuration).is_err() {
            return Some("oauth-field-invalid");
        }
        self.authentication_input().err()
    }

    pub fn invalidate_geometry(&mut self) {
        for field in &mut self.fields {
            field.invalidate_geometry();
        }
    }
}
impl State {
    pub(super) fn customizable(&self) -> bool {
        self.attempt.is_none() && self.provider.is_some() && !self.picker.open
    }
    pub fn invalidate_identity_geometry(&mut self) {
        self.identity.invalidate_geometry();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fixtures::entry;

    #[test]
    fn optional_string_configuration_keeps_an_omitted_value_omitted() {
        let mut choice = Choice {
            provider: entry("optional-url", false),
            method: 0,
        };
        choice.provider.descriptor.configuration_schema = serde_json::json!({
            "type":"object", "properties":{"baseUrl":{"type":"string","minLength":1}},
            "additionalProperties":false
        });
        choice.provider.descriptor.configuration_defaults = serde_json::json!({});
        let mut form = Identity::default();
        form.configure(&choice, None);
        assert_eq!(form.configuration().unwrap(), serde_json::json!({}));
        form.fields[2].insert("https://relay.example/v1");
        assert_eq!(
            form.configuration().unwrap(),
            serde_json::json!({"baseUrl":"https://relay.example/v1"})
        );
    }

    #[test]
    fn provider_schema_drives_authentication_and_recovery_excludes_secrets() {
        let mut choice = Choice {
            provider: entry("external-example", false),
            method: 0,
        };
        let mut form = Identity::default();
        form.configure(&choice, None);
        assert_eq!(form.authentication_field(), Some("apiKey"));
        assert!(form.error().is_some());
        form.fields[0].insert("Personal account");
        form.fields[1].insert("personal");
        form.fields[3].insert("private-key");
        let start = form.start(&choice, None).unwrap();
        assert_eq!(
            start.authentication.input,
            serde_json::json!({"apiKey":"private-key"})
        );
        let LoginTarget::Create {
            provider,
            name,
            slug,
            ..
        } = &start.target
        else {
            panic!()
        };
        assert_eq!(*provider, choice.provider.identity);
        assert_eq!(name, "Personal account");
        assert_eq!(slug, "personal");
        let state = State {
            attempt: Some(start.recovery()),
            prepared: Some(start.clone()),
            ..Default::default()
        };
        let checkpoint = serde_json::to_string(&state.checkpoint()).unwrap();
        assert!(!checkpoint.contains("private-key"));
        assert!(!checkpoint.contains("authentication"));
        form.clear_authentication();
        assert!(form.fields[3].text().is_empty());
        assert_eq!(start.authentication.input["apiKey"], "private-key");

        choice.provider.descriptor.authentication[0].input_schema =
            serde_json::json!({"type":"object","properties":{"token":{"type":"object"}}});
        form.configure(&choice, None);
        assert_eq!(form.authentication_field(), Some("JSON"));
        form.fields[3] = Editor::bounded(64 * 1024, "oauth-field-invalid");
        form.fields[3].insert(r#"{"token":{"value":"nested"}}"#);
        assert_eq!(
            form.start(&choice, None).unwrap().authentication.input["token"]["value"],
            "nested"
        );

        let interactive = Choice {
            provider: entry("external-browser", true),
            method: 0,
        };
        form.configure(&interactive, None);
        assert!(form.authentication_field().is_none());
        assert_eq!(
            form.start(&interactive, None).unwrap().authentication.input,
            serde_json::json!({})
        );
    }
}
