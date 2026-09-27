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
fn executable() -> String {
    std::env::temp_dir()
        .join("fixture")
        .to_string_lossy()
        .into_owned()
}
#[test]
fn normal_rename_and_advanced_edits_preserve_exact_launch_values() {
    let previous = Agent {
        id: "fixture".into(),
        display_name: "Fixture".into(),
        executable: executable(),
        args: vec![
            "".into(),
            " leading ".into(),
            "line\nbreak".into(),
            "\u{202e}".into(),
        ],
        env: [(" KEY ".into(), " \nvalue\t ".into())].into(),
    };
    let mut submission = Submission {
        route: json!({"agent":"fixture"}),
        revision: "1".into(),
        action: "save".into(),
        grant: None,
        fields: [
            ("name".into(), json!("Renamed")),
            ("executable".into(), json!(executable())),
        ]
        .into(),
    };
    let changed = normal(&submission, previous.id.clone(), Some(&previous)).unwrap();
    assert_eq!(changed.args, previous.args);
    assert_eq!(changed.env, previous.env);
    for locale in ["en", "zh-CN", "zh-TW"] {
        let configuration = Configuration {
            revision: Some(1),
            agents: vec![previous.clone()],
            activation_error: None,
        };
        advanced(&Words::new(locale), &configuration, &previous, "args")
            .unwrap()
            .validate()
            .unwrap();
    }
    let mut changed = previous.clone();
    submission.route["launch"] = json!("args");
    submission
        .fields
        .insert("launch-json".into(), json!(document(&previous.args)));
    update(&mut changed, &submission).unwrap();
    assert_eq!(changed, previous);
    assert!(!field("args", Some(&previous)).enabled);
}
#[test]
fn ordinary_lines_keep_spaces_empty_values_and_reject_duplicate_variables() {
    let mut submission = Submission {
        route: Value::Null,
        revision: "0".into(),
        action: "save".into(),
        grant: None,
        fields: [
            ("name".into(), json!("Fixture")),
            ("executable".into(), json!(executable())),
            ("args".into(), json!(" leading \ntrailing ")),
            ("env".into(), json!(" KEY = value \nEMPTY=")),
        ]
        .into(),
    };
    let agent = normal(&submission, "fixture".into(), None).unwrap();
    assert_eq!(agent.args, [" leading ", "trailing "]);
    assert_eq!(agent.env[" KEY "], " value ");
    assert_eq!(agent.env["EMPTY"], "");
    let encoded = document(&agent.env);
    assert_eq!(
        serde_json::from_str::<BTreeMap<String, String>>(&encoded).unwrap(),
        agent.env
    );
    submission
        .fields
        .insert("env".into(), json!("A=one\nA=two"));
    assert!(normal(&submission, "fixture".into(), None).is_err());
}

#[test]
fn maximum_valid_launch_values_remain_readable_and_untouched_in_the_bounded_editor() {
    let mut agent = Agent {
        id: "fixture".into(),
        display_name: "Fixture".into(),
        executable: executable(),
        args: vec![],
        env: BTreeMap::new(),
    };
    let empty_bytes = serde_json::to_vec(&(&agent.executable, [""], &agent.env))
        .unwrap()
        .len();
    let remaining = 65_536 - empty_bytes;
    agent.args = vec![format!(
        "{}{}",
        "\"".repeat(remaining / 2),
        if !remaining.is_multiple_of(2) {
            "x"
        } else {
            ""
        }
    )];
    agent.validate().unwrap();
    assert_eq!(
        serde_json::to_vec(&(&agent.executable, &agent.args, &agent.env))
            .unwrap()
            .len(),
        65_536
    );
    let configuration = Configuration {
        revision: Some(1),
        agents: vec![agent.clone()],
        activation_error: None,
    };
    let normal = editor(&Words::new("en"), &configuration, Some(&agent), None);
    normal.validate().unwrap();
    assert!(!normal.field("args").unwrap().enabled);
    let advanced = advanced(&Words::new("en"), &configuration, &agent, "args").unwrap();
    advanced.validate().unwrap();
    assert!(advanced.action("save-launch").is_none());
    let submission = Submission {
        route: json!({"agent":"fixture"}),
        revision: "1".into(),
        action: "save".into(),
        grant: None,
        fields: [
            ("name".into(), json!("Renamed")),
            ("executable".into(), json!(agent.executable)),
            ("env".into(), json!("")),
        ]
        .into(),
    };
    assert_eq!(
        super::normal(&submission, agent.id.clone(), Some(&agent))
            .unwrap()
            .args,
        agent.args
    );
}
