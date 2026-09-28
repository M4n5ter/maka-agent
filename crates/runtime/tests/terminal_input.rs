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

use maka_runtime::terminal::{
    MouseEncoding, MouseTracking, TerminalSize,
    input::{InputAction, encode_actions},
};
use serde_json::json;

#[test]
fn typed_paste_is_strict_and_encodes_at_the_live_bracketed_paste_cut() {
    use maka_runtime::terminal::{TerminalScreen, input::encode_actions_with_paste};
    let size = TerminalSize::new(80, 24).unwrap();
    let modes = TerminalScreen::new(size).input;
    let action = InputAction::parse(json!({"type":"paste","text":"中\r\nline\t2"})).unwrap();
    assert_eq!(
        serde_json::from_value::<InputAction>(serde_json::to_value(&action).unwrap()).unwrap(),
        action
    );
    assert_eq!(
        encode_actions_with_paste(std::slice::from_ref(&action), modes, size, true).unwrap(),
        "\x1b[200~中\nline\t2\x1b[201~"
    );
    assert_eq!(
        encode_actions_with_paste(&[action], modes, size, false).unwrap(),
        "中\rline\t2"
    );
    for invalid in [
        json!({"type":"paste","text":""}),
        json!({"type":"paste","text":"\x1b[201~"}),
        json!({"type":"paste","text":"\u{0085}"}),
        json!({"type":"paste","text":"x","modifiers":[]}),
    ] {
        assert!(InputAction::parse(invalid).is_err());
    }
}

#[test]
fn human_mouse_mode_races_are_ignored_but_tools_and_other_bad_input_stay_strict() {
    use maka_runtime::terminal::{TerminalScreen, input::encode_controller_actions};
    let size = TerminalSize::new(80, 24).unwrap();
    let mut modes = TerminalScreen::new(size).input;
    let mouse =
        InputAction::parse(json!({"type":"mouse","event":"press","button":"left","x":1,"y":1}))
            .unwrap();
    assert!(encode_actions(std::slice::from_ref(&mouse), modes, size).is_err());
    assert_eq!(
        encode_controller_actions(std::slice::from_ref(&mouse), modes, size, false).unwrap(),
        ""
    );
    modes.mouse_tracking_mode = MouseTracking::Vt200;
    modes.mouse_encoding = MouseEncoding::Sgr;
    assert_eq!(
        encode_controller_actions(std::slice::from_ref(&mouse), modes, size, false).unwrap(),
        "\x1b[<0;2;2M"
    );
    let movement = InputAction::parse(json!({"type":"mouse","event":"move","x":1,"y":1})).unwrap();
    assert_eq!(
        encode_controller_actions(&[movement], modes, size, false).unwrap(),
        ""
    );
    let outside =
        InputAction::parse(json!({"type":"mouse","event":"press","button":"left","x":80,"y":1}))
            .unwrap();
    assert_eq!(
        encode_controller_actions(&[outside], modes, size, false).unwrap(),
        ""
    );
    assert!(
        encode_controller_actions(
            &[mouse, InputAction::Paste("\x03".into())],
            modes,
            size,
            false
        )
        .is_err()
    );
}
