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

use crate::editor::Editor;
use maka_protocol::model_provider::Descriptor;
use serde_json::{Map, Value};

pub struct Form {
    original: Value,
    pub fields: Vec<Field>,
}
pub struct Field {
    pub key: String,
    pub label: String,
    pub editor: Option<Editor>,
    pub choices: Vec<Value>,
    pub value: Value,
    schema: Value,
    kind: String,
    required: bool,
    original: Value,
}
impl Form {
    pub fn new(configuration: &Value, descriptor: Option<&Descriptor>) -> Self {
        let properties = descriptor
            .and_then(|d| d.configuration_schema.get("properties"))
            .and_then(Value::as_object);
        let mut names = std::collections::BTreeSet::new();
        if let Some(properties) = properties {
            names.extend(properties.keys().cloned());
        }
        if let Some(values) = configuration.as_object() {
            names.extend(values.keys().cloned());
        }
        if let Some(defaults) = descriptor.and_then(|d| d.configuration_defaults.as_object()) {
            names.extend(defaults.keys().cloned());
        }
        let fields = names
            .into_iter()
            .filter_map(|key| {
                let schema = properties
                    .and_then(|p| p.get(&key))
                    .cloned()
                    .unwrap_or(Value::Null);
                let original = configuration.get(&key).cloned().unwrap_or(Value::Null);
                let value = original.clone();
                let defaults = descriptor.and_then(|d| d.configuration_defaults.get(&key));
                let kind = schema["type"]
                    .as_str()
                    .or_else(|| match defaults.unwrap_or(&value) {
                        Value::String(_) => Some("string"),
                        Value::Number(_) => Some("number"),
                        Value::Bool(_) => Some("boolean"),
                        _ => None,
                    });
                let choices = if let Some(values) = schema["enum"].as_array() {
                    if values
                        .iter()
                        .all(|v| v.is_string() || v.is_number() || v.is_boolean() || v.is_null())
                    {
                        values.clone()
                    } else {
                        return None;
                    }
                } else if kind == Some("boolean") {
                    vec![Value::Bool(false), Value::Bool(true)]
                } else {
                    vec![]
                };
                if choices.is_empty() && !matches!(kind, Some("string" | "number" | "integer")) {
                    return None;
                }
                let required = descriptor
                    .and_then(|d| d.configuration_schema["required"].as_array())
                    .is_some_and(|v| v.iter().any(|v| v == &key));
                let editor = choices.is_empty().then(|| {
                    let mut editor =
                        Editor::bounded(64 * 1024, "connection-configuration-too-large");
                    editor.insert(&match &value {
                        Value::String(v) => v.clone(),
                        Value::Null => String::new(),
                        _ => value.to_string(),
                    });
                    editor.clear_history();
                    editor
                });
                let kind = kind.unwrap_or("string").to_owned();
                Some(Field {
                    label: schema["title"].as_str().unwrap_or(&key).to_owned(),
                    key,
                    editor,
                    choices,
                    value,
                    original,
                    kind,
                    schema,
                    required,
                })
            })
            .collect();
        Self {
            original: configuration.clone(),
            fields,
        }
    }
    pub fn value(&self) -> Result<Value, &'static str> {
        let mut result = self.original.as_object().cloned().unwrap_or_else(Map::new);
        for field in &self.fields {
            let value = if let Some(editor) = &field.editor {
                if editor.error.is_some() {
                    return Err("connection-preferences-invalid");
                }
                let raw = editor.text();
                if raw.is_empty() && !field.required && field.original.is_null() {
                    continue;
                }
                match field.kind.as_str() {
                    "number" | "integer" => {
                        let value: Value = serde_json::from_str(raw)
                            .map_err(|_| "connection-preferences-invalid")?;
                        let number = value.as_f64().ok_or("connection-preferences-invalid")?;
                        if field.schema["type"] == "integer" && number.fract() != 0.0
                            || field.schema["minimum"]
                                .as_f64()
                                .is_some_and(|min| number < min)
                            || field.schema["maximum"]
                                .as_f64()
                                .is_some_and(|max| number > max)
                        {
                            return Err("connection-preferences-invalid");
                        }
                        value
                    }
                    _ => {
                        let length = raw.chars().count() as u64;
                        if field.schema["minLength"]
                            .as_u64()
                            .is_some_and(|min| length < min)
                            || field.schema["maxLength"]
                                .as_u64()
                                .is_some_and(|max| length > max)
                        {
                            return Err("connection-preferences-invalid");
                        }
                        Value::String(raw.into())
                    }
                }
            } else {
                field.value.clone()
            };
            if field.required && value.is_null() {
                return Err("connection-preferences-invalid");
            }
            if value.is_null() && self.original.get(&field.key).is_none() {
                result.remove(&field.key);
            } else {
                result.insert(field.key.clone(), value);
            }
        }
        Ok(Value::Object(result))
    }
    pub fn changed(&self) -> bool {
        self.value().is_ok_and(|v| v != self.original)
    }
    pub fn choose(&mut self, index: usize, value: usize) {
        if let Some(field) = self.fields.get_mut(index)
            && let Some(value) = field.choices.get(value)
        {
            field.value = value.clone();
        }
    }
    pub fn invalidate_geometry(&mut self) {
        for field in &mut self.fields {
            if let Some(editor) = &mut field.editor {
                editor.invalidate_geometry();
            }
        }
    }
}
