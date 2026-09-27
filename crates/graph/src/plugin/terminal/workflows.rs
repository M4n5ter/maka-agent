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
use maka_runtime::execution::ThinkingLevel;

#[test]
fn offline_presets_keep_explicit_thinking_and_can_return_to_model_default() {
    for locale in ["en", "zh-CN", "zh-TW"] {
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
            let preset = Preset {
                id: "offline".into(),
                name: "Offline model".into(),
                description: String::new(),
                profile: "local_read".into(),
                connection_slug: "offline".into(),
                model: "later".into(),
                thinking_level: level,
                enabled: false,
            };
            let view = editor(&Words::new(locale), "3".into(), Some(&preset));
            view.validate().unwrap();
            let view::Control::Choice { value, options } = &view.field("thinking").unwrap().control
            else {
                panic!("thinking choice")
            };
            assert_eq!(thinking_input(value).unwrap(), level);
            assert!(options.iter().any(|option| option.value == "default"));
            assert!(
                view.action("save")
                    .unwrap()
                    .fields
                    .contains(&"thinking".into())
            );
        }
    }
    assert_eq!(thinking_input("default").unwrap(), None);
    assert!(thinking_input("invented").is_err());
}
