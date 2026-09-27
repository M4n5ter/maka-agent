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

use super::{Action, App, Command, Editor, Form, view};
use crate::{
    pages::manage::preferences::provider,
    ui::{self, Node, On, Size, Tone},
    view::{form, safe},
};
use maka_protocol::model_provider::Descriptor;
use ratatui::Frame;
use serde_json::Value;

pub(super) enum Mode {
    Fields(provider::Form),
    Advanced,
}
impl Mode {
    pub fn new(value: &Value, descriptor: Option<&Descriptor>) -> Self {
        Self::Fields(provider::Form::new(value, descriptor))
    }
    pub fn advanced(&self) -> bool {
        matches!(self, Self::Advanced)
    }
    pub fn fields(&self) -> Option<&provider::Form> {
        if let Self::Fields(fields) = self {
            Some(fields)
        } else {
            None
        }
    }
    pub fn invalidate_geometry(&mut self) {
        if let Self::Fields(fields) = self {
            fields.invalidate_geometry();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Field {
    Base(usize),
    Provider(usize),
}

impl Form {
    pub(super) fn configuration_value(&self) -> Result<Value, &'static str> {
        let value = match &self.configuration {
            Mode::Fields(fields) => fields.value()?,
            Mode::Advanced => {
                if self.fields[1].error.is_some() {
                    return Err("connection-configuration-invalid");
                }
                serde_json::from_str(self.fields[1].text())
                    .map_err(|_| "connection-configuration-invalid")?
            }
        };
        maka_protocol::configuration::validation::provider_configuration(&value)
            .map_err(|_| "connection-configuration-invalid")?;
        Ok(value)
    }
    pub(super) fn reset_configuration(&mut self) {
        let descriptor = &self.providers[self.provider].descriptor;
        let defaults = &descriptor.configuration_defaults;
        self.fields[1] = Editor::bounded(64 * 1024, "onboard-field-invalid");
        self.fields[1].insert(&defaults.to_string());
        self.fields[1].clear_history();
        self.configuration = Mode::new(defaults, Some(descriptor));
    }
    pub(super) fn switch_configuration(&mut self, advanced: bool) -> Result<(), &'static str> {
        let value = self.configuration_value()?;
        if advanced {
            // Keep the user's JSON spelling and history when the ordinary form
            // has not changed its value; only one mode supplies request input.
            if serde_json::from_str::<Value>(self.fields[1].text())
                .ok()
                .as_ref()
                != Some(&value)
            {
                self.fields[1] = Editor::bounded(64 * 1024, "onboard-field-invalid");
                self.fields[1]
                    .insert(&serde_json::to_string_pretty(&value).expect("configuration"));
                self.fields[1].clear_history();
            }
            self.configuration = Mode::Advanced;
        } else {
            self.configuration = Mode::new(&value, Some(&self.providers[self.provider].descriptor));
        }
        self.fields[1].invalidate_geometry();
        self.configuration.invalidate_geometry();
        Ok(())
    }
    pub(super) fn editor(&self, field: Field) -> Option<&Editor> {
        match field {
            Field::Base(index) if index != 1 || self.configuration.advanced() => {
                self.fields.get(index)
            }
            Field::Provider(index) => self
                .configuration
                .fields()?
                .fields
                .get(index)?
                .editor
                .as_ref(),
            _ => None,
        }
    }
    pub(super) fn editor_mut(&mut self, field: Field) -> Option<&mut Editor> {
        match field {
            Field::Base(index) if index != 1 || self.configuration.advanced() => {
                self.fields.get_mut(index)
            }
            Field::Provider(index) => {
                if let Mode::Fields(fields) = &mut self.configuration {
                    fields.fields.get_mut(index)?.editor.as_mut()
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    pub(super) fn fields_in_order(&self) -> Vec<Field> {
        let mut fields = vec![Field::Base(0)];
        if let Some(configuration) = self.configuration.fields() {
            fields.extend((0..configuration.fields.len()).map(Field::Provider));
        } else {
            fields.push(Field::Base(1));
        }
        fields.push(Field::Base(2));
        fields
    }
}

pub(super) fn row_path(index: usize) -> String {
    format!("{}/config/{index}", view::FORM)
}
pub(super) fn path(field: Field) -> String {
    match field {
        Field::Base(index) => view::row_path(index),
        Field::Provider(index) => row_path(index),
    }
}
pub(super) fn focused(path: &str) -> Option<Field> {
    view::row(path).map(Field::Base).or_else(|| {
        path.strip_prefix(&format!("{}/config/", view::FORM))?
            .parse()
            .ok()
            .map(Field::Provider)
    })
}
pub(super) fn label(app: &App, field: &provider::Field) -> String {
    if field.key == "baseUrl" {
        app.i18n.text("provider-base-url")
    } else {
        safe(&field.label)
    }
}
pub(super) fn rows(app: &App, width: u16) -> Option<Node<Action>> {
    let fields = app.onboarding.dialog.as_ref()?.configuration.fields()?;
    let rows = fields
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            if field.editor.is_some() {
                return Node::slot(index.to_string(), 1)
                    .on(On::Activate(Action::Onboard(Command::ConfigurationField(
                        index,
                    ))))
                    .enabled(app.onboarding_offered(&Command::ConfigurationField(index)));
            }
            let display = |value: &Value| match value {
                Value::String(value) => safe(value),
                Value::Bool(value) => app.i18n.text(if *value {
                    "model-profile-on"
                } else {
                    "model-profile-off"
                }),
                Value::Null => app.i18n.text("thinking-default"),
                value => value.to_string(),
            };
            Node::row(
                index.to_string(),
                vec![
                    Node::text("label", vec![(label(app, field), Tone::Muted)])
                        .clip()
                        .size(Size::Fixed(width)),
                    Node::text("value", vec![(display(&field.value), Tone::Accent)])
                        .clip()
                        .size(Size::Fill),
                ],
            )
            .on(On::Choose {
                choices: field
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(choice, value)| ui::Choice {
                        label: display(value),
                        action: Action::Onboard(Command::ConfigurationChoice(index, choice)),
                    })
                    .collect(),
                current: field.choices.iter().position(|value| value == &field.value),
            })
            .enabled(app.onboarding_offered(&Command::ConfigurationChoice(index, 0)))
        })
        .collect();
    Some(Node::column("config", rows))
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut App, width: u16) {
    let Some(fields) = app
        .onboarding
        .dialog
        .as_ref()
        .and_then(|form| form.configuration.fields())
    else {
        return;
    };
    let labels: Vec<_> = fields
        .fields
        .iter()
        .map(|field| label(app, field))
        .collect();
    let rects: Vec<_> = (0..fields.fields.len())
        .map(|index| {
            app.layer
                .rect(&row_path(index))
                .filter(|rect| !rect.is_empty())
        })
        .collect();
    let focused = app.layer.focused_path().and_then(focused);
    let editable = app.onboarding_enabled(&Command::Field(0));
    let colors = app.theme.colors();
    let Some(form) = &mut app.onboarding.dialog else {
        return;
    };
    let editing = form.models.is_none();
    let Mode::Fields(configuration) = &mut form.configuration else {
        return;
    };
    for (index, field) in configuration.fields.iter_mut().enumerate() {
        let Some(editor) = &mut field.editor else {
            continue;
        };
        if let Some(rect) = rects[index].filter(|_| editing) {
            form::draw(
                frame,
                rect,
                width,
                form::Row {
                    label: &labels[index],
                    focused: editable && focused == Some(Field::Provider(index)),
                    masked: false,
                    placeholder: None,
                },
                editor,
                colors,
            );
        } else {
            editor.invalidate_geometry();
        }
    }
}

#[cfg(test)]
mod tests;
