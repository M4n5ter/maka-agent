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

//! Bounded depth projection into ordinary terminal Braille cells; no image protocol.
use super::mark::FORMATIONS;
use crate::theme::Palette;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
};

#[derive(Default)]
pub(super) struct Renderer {
    dots: Vec<(f32, [f32; 3])>,
}

impl Renderer {
    pub(super) fn draw(
        &mut self,
        area: Rect,
        buffer: &mut Buffer,
        colors: Palette,
        phase: Option<f32>,
    ) {
        let width = usize::from(area.width) * 2;
        let height = usize::from(area.height) * 4;
        self.dots
            .resize(width * height, (f32::NEG_INFINITY, [0.0; 3]));
        self.dots.fill((f32::NEG_INFINITY, [0.0; 3]));
        let phase_value = phase.unwrap_or(0.0);
        let smooth = |t: f32| {
            let t = t.clamp(0.0, 1.0);
            t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
        };
        let blend =
            smooth((phase_value - 0.12) / 0.24) * (1.0 - smooth((phase_value - 0.62) / 0.24));
        // Keep both poses legible. The formation rocks gently while the dots
        // travel; it never turns the word edge-on or backwards.
        let angle = (std::f32::consts::TAU * phase_value).sin() * 0.18;
        let (sy, cy) = angle.sin_cos();
        let (sx, cx) = (angle.sin() * 0.16).sin_cos();
        let (sz, cz) = (angle.sin() * -0.055).sin_cos();
        let rotate = |[x, y, z]: [f32; 3]| {
            let [x, z] = [x * cy + z * sy, z * cy - x * sy];
            let [y, z] = [y * cx - z * sx, z * cx + y * sx];
            [x * cz - y * sz, y * cz + x * sz, z]
        };
        // Brand blue from assets/logo.png / scripts/generate-logo.py.
        const BLUE: [u8; 3] = [0x47, 0xa3, 0xe2];
        let rgb = match (colors.background, colors.foreground) {
            (Color::Rgb(r, g, b), Color::Rgb(fr, fg, fb)) => Some(([r, g, b], [fr, fg, fb])),
            _ => None,
        };
        let opacity = if phase.is_some() { 0.44 } else { 0.24 };
        for drone in FORMATIONS.iter() {
            let interpolate =
                |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * blend);
            let [x, y, z] = rotate(interpolate(drone.logo.position, drone.word.position));
            let perspective = 4.5 / (4.5 - z);
            let scale = height as f32 / 2.5;
            let column = (width as f32 / 2.0 + x * perspective * scale) as isize;
            let row = (height as f32 / 2.0 + y * perspective * scale) as isize;
            if column < 0 || row < 0 || column >= width as isize || row >= height as isize {
                continue;
            }
            let dot = &mut self.dots[row as usize * width + column as usize];
            if z <= dot.0 {
                continue;
            }
            let [nx, ny, nz] = rotate(interpolate(drone.logo.normal, drone.word.normal));
            let length = nx.hypot(ny).hypot(nz).max(f32::EPSILON);
            let [nx, ny, nz] = [nx / length, ny / length, nz / length];
            let diffuse = (-nx * 0.35 - ny * 0.45 + nz * 0.82).max(0.0);
            let gloss = (-nx * 0.18 - ny * 0.25 + nz * 0.95).max(0.0).powi(14) * 0.25;
            let shade = if let Some((bg, fg)) = rgb {
                std::array::from_fn(|i| {
                    let lit = f32::from(BLUE[i]) * (0.45 + 0.55 * diffuse);
                    let lit = lit + (f32::from(fg[i]) - lit) * gloss;
                    f32::from(bg[i]) + (lit - f32::from(bg[i])) * opacity
                })
            } else {
                [0.0; 3]
            };
            *dot = (z, shade);
        }
        const BITS: [u32; 8] = [1, 8, 2, 16, 4, 32, 64, 128];
        for y in 0..usize::from(area.height) {
            for x in 0..usize::from(area.width) {
                let (mut bits, mut count, mut color) = (0, 0, [0.0; 3]);
                for (point, bit) in BITS.iter().enumerate() {
                    let dot = &self.dots[(y * 4 + point / 2) * width + x * 2 + point % 2];
                    if dot.0.is_finite() {
                        bits |= bit;
                        count += 1;
                        for (sum, channel) in color.iter_mut().zip(dot.1) {
                            *sum += channel;
                        }
                    }
                }
                if count == 0 {
                    continue;
                }
                let cell = &mut buffer[(area.x + x as u16, area.y + y as u16)];
                cell.set_char(char::from_u32(0x2800 + bits).unwrap());
                if rgb.is_some() {
                    let [r, g, b] = color.map(|v| (v / count as f32).round() as u8);
                    cell.set_fg(Color::Rgb(r, g, b));
                } else {
                    cell.set_fg(Color::Rgb(BLUE[0], BLUE[1], BLUE[2]))
                        .set_style(ratatui::style::Style::default().add_modifier(Modifier::DIM));
                }
            }
        }
    }
}
