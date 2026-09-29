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
use crate::{
    app::ConnectionState,
    i18n::{I18n, Locale, LocalePreference},
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn app() -> App {
    let mut app = App::new(
        "/unused".into(),
        I18n::new(LocalePreference::Explicit(Locale::En), Locale::En),
    );
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "epoch".into(),
    };
    app.providers = crate::providers::fixtures::catalog();
    app.apply(Action::Onboard(Command::Open));
    let form = app.onboarding.dialog.as_mut().unwrap();
    let descriptor = &mut form.providers[0].descriptor;
    descriptor.configuration_schema = json!({"type":"object","properties":{
        "baseUrl":{"type":"string","minLength":1},"limit":{"type":"integer","minimum":1,"maximum":20},
        "enabled":{"type":"boolean"},"mode":{"enum":["a","b"]},"nullable":{"enum":["a",null]}
    },"required":["baseUrl"]});
    descriptor.configuration_defaults = json!({"baseUrl":"https://before.example","limit":3,"enabled":true,"mode":"a","nullable":null,"opaque":{"nested":[1,2]}});
    form.reset_configuration();
    paint(&mut app, 80, 28);
    app
}

fn paint(app: &mut App, width: u16, height: u16) {
    Terminal::new(TestBackend::new(width, height))
        .unwrap()
        .draw(|frame| crate::view::draw(frame, app))
        .unwrap();
}
fn index(app: &App, key: &str) -> usize {
    app.onboarding
        .dialog
        .as_ref()
        .unwrap()
        .configuration
        .fields()
        .unwrap()
        .fields
        .iter()
        .position(|field| field.key == key)
        .unwrap()
}
fn edit(app: &mut App, command: Command, value: &str) {
    app.apply(Action::Onboard(command));
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    )));
    app.input(Event::Paste(value.into()));
}

#[test]
fn onboarding197_common_controls_supply_the_same_reviewed_configuration_to_verify_and_save() {
    let mut app = app();
    let base = index(&app, "baseUrl");
    edit(
        &mut app,
        Command::ConfigurationField(base),
        "https://after.example",
    );
    let limit = index(&app, "limit");
    edit(&mut app, Command::ConfigurationField(limit), "21");
    assert!(!app.onboarding_enabled(&Command::Verify));
    edit(&mut app, Command::ConfigurationField(limit), "9");
    app.apply(Action::Onboard(Command::ConfigurationChoice(
        index(&app, "enabled"),
        0,
    )));
    app.apply(Action::Onboard(Command::ConfigurationChoice(
        index(&app, "mode"),
        1,
    )));
    assert!(
        !app.onboarding
            .dialog
            .as_ref()
            .unwrap()
            .configuration
            .advanced()
    );
    assert!(!app.onboarding_enabled(&Command::Field(1)));
    for locale in Locale::ALL {
        app.i18n = I18n::new(LocalePreference::Explicit(locale), locale);
        paint(&mut app, 44, 22);
        assert!(app.onboarding_enabled(&Command::Verify));
        assert!(app.i18n.diagnostics().is_empty());
    }
    let request = app.onboarding_request(false).unwrap();
    let reviewed = request.input.target.clone();
    let maka_protocol::oauth::Target::Create {
        configuration: value,
        ..
    } = &reviewed
    else {
        panic!("anonymous onboarding creates a connection")
    };
    assert_eq!(value["baseUrl"], "https://after.example");
    assert_eq!(value["limit"], 9);
    assert_eq!(value["enabled"], false);
    assert_eq!(value["mode"], "b");
    assert_eq!(value["opaque"], json!({"nested":[1,2]}));
    assert!(value.as_object().unwrap().contains_key("nullable"));
    app.onboarding_completed(
        request.ticket,
        Ok(super::super::ResultValue::Verified(
            maka_protocol::configuration::onboarding::OnboardingVerifyResult::Verified {
                models: vec![serde_json::from_value(json!({"id":"model"})).unwrap()],
            },
        )),
    );
    paint(&mut app, 80, 28);
    assert!(!app.onboarding_enabled(&Command::Advanced(true)));
    assert!(!app.onboarding_enabled(&Command::ConfigurationField(base)));
    app.apply(Action::Onboard(Command::Toggle("model".into())));
    let save = app.onboarding_request(true).unwrap();
    assert!(
        save.input.target == reviewed,
        "save keeps the exact verified target"
    );
}

#[test]
fn onboarding197_advanced_roundtrip_preserves_opaque_values_null_and_invalid_drafts() {
    let mut app = app();
    app.apply(Action::Onboard(Command::Advanced(true)));
    paint(&mut app, 80, 28);
    let json = "{\n  \"baseUrl\": \"https://advanced.example\", \"nullable\": null, \"opaque\": {\"nested\": [null, \"中文\"]}\n}";
    edit(&mut app, Command::Field(1), json);
    app.apply(Action::Onboard(Command::Advanced(false)));
    paint(&mut app, 80, 28);
    assert_eq!(
        app.onboarding
            .dialog
            .as_ref()
            .unwrap()
            .configuration_value()
            .unwrap(),
        serde_json::from_str::<Value>(json).unwrap()
    );
    app.apply(Action::Onboard(Command::Advanced(true)));
    paint(&mut app, 80, 28);
    assert_eq!(
        app.onboarding.dialog.as_ref().unwrap().fields[1].text(),
        json,
        "unchanged fields retain the exact advanced draft"
    );
    edit(&mut app, Command::Field(1), "{invalid-json");
    app.apply(Action::Onboard(Command::Advanced(false)));
    let form = app.onboarding.dialog.as_ref().unwrap();
    assert!(form.configuration.advanced());
    assert_eq!(form.fields[1].text(), "{invalid-json");
    assert!(!app.onboarding_enabled(&Command::Verify));
    edit(&mut app, Command::Field(1), json);
    app.apply(Action::Onboard(Command::Advanced(false)));
    paint(&mut app, 80, 28);
    let base = index(&app, "baseUrl");
    edit(
        &mut app,
        Command::ConfigurationField(base),
        "https://ordinary.example",
    );
    let form = app.onboarding.dialog.as_ref().unwrap();
    assert_eq!(
        form.configuration_value().unwrap()["baseUrl"],
        "https://ordinary.example"
    );
    assert_eq!(
        serde_json::from_str::<Value>(form.fields[1].text()).unwrap()["baseUrl"],
        "https://advanced.example",
        "only the active mode supplies input"
    );
    app.apply(Action::Onboard(Command::Advanced(true)));
    let value = app
        .onboarding
        .dialog
        .as_ref()
        .unwrap()
        .configuration_value()
        .unwrap();
    assert_eq!(value["opaque"], json!({"nested":[null,"中文"]}));
    assert!(value.as_object().unwrap().contains_key("nullable"));
}

#[test]
fn onboarding197_switching_provider_replaces_both_modes_and_their_edit_history() {
    let mut app = app();
    app.apply(Action::Onboard(Command::Advanced(true)));
    paint(&mut app, 80, 28);
    edit(
        &mut app,
        Command::Field(1),
        r#"{"oldPrivateDraft":"only-in-memory"}"#,
    );
    let mut other = crate::providers::fixtures::entry("other", false);
    other.descriptor.configuration_defaults = json!({"baseUrl":"https://other.example"});
    app.onboarding
        .dialog
        .as_mut()
        .unwrap()
        .providers
        .push(other);
    app.apply(Action::Onboard(Command::PickProvider));
    paint(&mut app, 80, 28);
    app.apply(Action::Onboard(Command::Provider(1)));
    paint(&mut app, 80, 28);
    let form = app.onboarding.dialog.as_ref().unwrap();
    assert!(!form.configuration.advanced());
    assert_eq!(
        form.configuration_value().unwrap(),
        json!({"baseUrl":"https://other.example"})
    );
    assert!(!form.fields[1].text().contains("oldPrivateDraft"));
}

#[test]
fn onboarding197_configuration_rows_scroll_to_real_editors() {
    let mut app = app();
    let form = app.onboarding.dialog.as_mut().unwrap();
    form.providers[0].descriptor.configuration_schema = json!({"type":"object"});
    form.providers[0].descriptor.configuration_defaults = Value::Object(
        (0..50)
            .map(|index| (format!("field-{index:02}"), json!(format!("value-{index}"))))
            .collect(),
    );
    form.reset_configuration();
    paint(&mut app, 44, 22);
    let last = index(&app, "field-49");
    app.apply(Action::Onboard(Command::ConfigurationField(last)));
    paint(&mut app, 44, 22);
    assert!(
        app.layer
            .rect(&row_path(last))
            .is_some_and(|rect| !rect.is_empty())
    );
    edit(&mut app, Command::ConfigurationField(last), "last edited");
    assert_eq!(
        app.onboarding
            .dialog
            .as_ref()
            .unwrap()
            .configuration_value()
            .unwrap()["field-49"],
        "last edited"
    );
}
