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

fn checked() -> sign_in::Checked {
    let event = json!({"kind":"initialized","agentInfo":{"name":"fixture","version":"1"},
        "authMethods":(0..31).map(|index| json!({
            "id":format!("{index:04}{}","m".repeat(1020)),
            "name":"\"".repeat(256),"type":"agent"
        })).collect::<Vec<_>>()});
    sign_in::Checked {
        id: uuid::Uuid::new_v4(),
        agent: "fixture".into(),
        revision: 1,
        methods: sign_in::offered(&event),
        summary: summary(&event),
        start: 0,
    }
}

fn near_limit(words: &Words, checked: &sign_in::Checked) -> (Agent, Configuration, View) {
    let mut original = agent();
    original.args = vec!["\"".into()];
    original.env = [("ENV".into(), "\"".into())].into();
    let mut configuration = Configuration {
        revision: Some(1),
        agents: vec![original.clone()],
        activation_error: None,
    };
    // Each extra quote contributes two wire bytes to each of two ordinary fields.
    // Select a legal command whose form fits immediately before sign-in is added.
    let base = editor(
        words,
        &configuration,
        Some(&original),
        Some(&checked.summary),
    );
    let base_bytes = serde_json::to_vec(&base).unwrap().len();
    let quotes = (1 + (view::MAX_BYTES - 64 - base_bytes) / 4).min(view::MAX_TEXT - 4);
    original.args = vec!["\"".repeat(quotes)];
    original.env.insert("ENV".into(), "\"".repeat(quotes));
    original.validate().unwrap();
    let command_bytes = serde_json::to_vec(&(&original.executable, &original.args, &original.env))
        .unwrap()
        .len();
    assert!(command_bytes > 55 * 1024 && command_bytes <= 64 * 1024);
    configuration.agents = vec![original.clone()];
    let before_auth = editor(
        words,
        &configuration,
        Some(&original),
        Some(&checked.summary),
    );
    before_auth.validate().unwrap();
    let bytes = serde_json::to_vec(&before_auth).unwrap().len();
    assert!(bytes > view::MAX_BYTES - 256 && bytes <= view::MAX_BYTES - 64);
    assert!(
        before_auth.field("args").unwrap().enabled && before_auth.field("env").unwrap().enabled
    );
    (original, configuration, before_auth)
}

#[tokio::test]
async fn public_app_fits_the_complete_auth_form_without_truncating_launch_configuration() {
    for locale in ["en", "zh-CN", "zh-TW"] {
        for phase in [
            None,
            Some(auth::Phase::Pending),
            Some(auth::Phase::Cancelling),
        ] {
            let checked = checked();
            let words = Words::new(locale);
            let (original, configuration, mut before_auth) = near_limit(&words, &checked);
            let fixture = Fixture::new(original.clone()).with_auth(checked.clone(), phase);
            // A methods-only projection was the missed composition boundary.
            // This reproduces the rejected size before exercising public Read.
            if phase.is_none() {
                sign_in::append(
                    &mut before_auth,
                    &json!({"agent":"fixture"}),
                    &configuration,
                    Some(checked.clone()),
                    None,
                    &words,
                );
                assert!(serde_json::to_vec(&before_auth).unwrap().len() > view::MAX_BYTES);
            }
            let route = json!({"agent":"fixture"});
            let complete = fixture.read_locale(route.clone(), locale).await;
            assert!(!complete.field("env").unwrap().enabled);
            assert_eq!(fixture.stored(), original);
            if phase.is_some() {
                assert!(
                    complete
                        .actions
                        .iter()
                        .filter(|action| action.enabled)
                        .all(|action| action.id.starts_with("cancel-authentication-"))
                );
                assert!(complete.actions.iter().any(
                    |action| action.enabled && action.id.starts_with("cancel-authentication-")
                ));
                assert!(complete.fields.iter().all(|field| !field.enabled));
                assert_eq!(fixture.backend.writes.load(Ordering::SeqCst), 0);
            } else {
                assert!(
                    matches!(&complete.field("auth-method").unwrap().control, view::Control::Choice { options, .. } if options.len() == 32)
                );
                assert!(
                    complete
                        .action(&format!("authenticate-{}", checked.id))
                        .unwrap()
                        .enabled
                );
                let mut fields = complete
                    .action("save")
                    .unwrap()
                    .fields
                    .iter()
                    .map(|id| (id.clone(), json!(text_field(&complete, id))))
                    .collect::<BTreeMap<_, _>>();
                fields.insert("name".into(), json!("Renamed"));
                fixture
                    .submit(&complete, route, "save", fields)
                    .await
                    .unwrap();
                assert_eq!(fixture.stored().args, original.args);
                assert_eq!(fixture.stored().env, original.env);
                assert_eq!(fixture.stored().display_name, "Renamed");
                assert_eq!(fixture.backend.writes.load(Ordering::SeqCst), 1);
            }
        }
    }
}
