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

//! A quiet, continuous mark in space left over after the composer is laid out.
use crate::{motion::Motion, theme::Palette};
use ratatui::{buffer::Buffer, layout::Rect};

mod mark;
mod render;

#[derive(Default)]
pub(crate) struct Welcome {
    renderer: render::Renderer,
}

impl Welcome {
    pub(crate) fn draw(
        &mut self,
        area: Rect,
        buffer: &mut Buffer,
        colors: Palette,
        motion: &mut Motion,
    ) {
        // Terminal cells are roughly twice as tall as they are wide. Keep a
        // roomy stage for both formations; never request space from the composer.
        let rows = area
            .height
            .saturating_sub(4)
            .min(22)
            .min(area.width.saturating_sub(4) / 3);
        if rows < 12 {
            return;
        }
        let stage = Rect::new(
            area.x + (area.width - rows * 3) / 2,
            area.y + (area.height - rows) / 2,
            rows * 3,
            rows,
        );
        self.renderer
            .draw(stage, buffer, colors, motion.cycle(20_000, 50));
    }
}

#[cfg(test)]
mod tests;
