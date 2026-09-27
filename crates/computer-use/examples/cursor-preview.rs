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

use maka_computer_use::cursor::{Color, theme};
fn main() {
    let output = std::env::args()
        .nth(1)
        .expect("usage: cursor-preview output.png");
    let mut image = tiny_skia::Pixmap::new(768, 320).unwrap();
    image.fill(tiny_skia::Color::from_rgba8(242, 247, 255, 255));
    let state = cursor_overlay::CursorVisualState::default();
    for (index, theme) in [
        cursor_overlay::embedded_default_theme(),
        std::sync::Arc::new(theme::compiled(Color::Blue)),
        std::sync::Arc::new(theme::compiled(Color::Mint)),
    ]
    .iter()
    .enumerate()
    {
        cursor_overlay::paint_compiled_theme(
            &mut image,
            theme,
            &state,
            128.0 + index as f32 * 256.0,
            150.0,
            std::f32::consts::FRAC_PI_4,
            4.0,
            1.0,
        );
    }
    image.save_png(output).unwrap();
}
