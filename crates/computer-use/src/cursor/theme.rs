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

use super::Color;
use cursor_overlay::{
    CompiledAnimation, CompiledDrawCommand, CompiledFrame, CompiledGeometry, CompiledStroke,
    CompiledTheme, CompiledTransform, CursorAction,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

/// Artwork is authored here as bounded vector geometry; no runtime theme code.
const TIP: f32 = 29.519;
const BODY: &[[f32; 2]] = &[
    [TIP, TIP],
    [94.0, 54.0],
    [72.0, 66.0],
    [83.0, 92.0],
    [68.0, 99.0],
    [55.0, 75.0],
    [39.0, 91.0],
];
const MARK: &[[f32; 2]] = &[[48.0, 59.0], [56.0, 48.0], [60.0, 64.0], [71.0, 55.0]];
const STAR: &[[f32; 2]] = &[
    [98.0, 20.0],
    [101.0, 28.0],
    [109.0, 31.0],
    [101.0, 34.0],
    [98.0, 42.0],
    [95.0, 34.0],
    [87.0, 31.0],
    [95.0, 28.0],
];
fn path(points: &[[f32; 2]], closed: bool) -> CompiledGeometry {
    CompiledGeometry::Path {
        vertices: points.to_vec(),
        in_tangents: vec![[0.0; 2]; points.len()],
        out_tangents: vec![[0.0; 2]; points.len()],
        closed,
    }
}
fn command(
    geometry: CompiledGeometry,
    fill: Option<[u8; 4]>,
    stroke: Option<([u8; 4], f32)>,
) -> CompiledDrawCommand {
    CompiledDrawCommand {
        geometries: vec![geometry],
        transform: CompiledTransform::default(),
        opacity: 1.0,
        fill,
        stroke: stroke.map(|(color, width)| CompiledStroke {
            color,
            width,
            line_cap: 2,
            line_join: 2,
        }),
    }
}
pub fn compiled(color: Color) -> CompiledTheme {
    let fill = color.rgba();
    let mut actions = BTreeMap::new();
    for action in CursorAction::ALL {
        let frames = if matches!(
            action,
            CursorAction::Click
                | CursorAction::Key
                | CursorAction::Observe
                | CursorAction::Text
                | CursorAction::Scroll
        ) {
            24
        } else {
            1
        };
        let mut animation = Vec::new();
        for i in 0..frames {
            let progress = i as f32 / frames as f32;
            let mut commands = Vec::new();
            if matches!(action, CursorAction::Click | CursorAction::Key) {
                let size = 10.0 + progress * 50.0;
                let mut ring = fill;
                ring[3] = ((1.0 - progress) * 150.0) as u8;
                commands.push(command(
                    CompiledGeometry::Ellipse {
                        center: [TIP, TIP],
                        size: [size, size],
                    },
                    None,
                    Some((ring, 3.0)),
                ));
            }
            let mut halo = fill;
            halo[3] = 34;
            commands.push(command(path(BODY, true), None, Some((halo, 13.0))));
            commands.push(command(
                path(BODY, true),
                Some(fill),
                Some(([255, 255, 255, 255], 4.0)),
            ));
            commands.push(command(
                path(MARK, false),
                None,
                Some(([255, 255, 255, 255], 5.0)),
            ));
            let mut star = fill;
            if frames > 1 {
                star[3] = (165.0 + 90.0 * (progress * std::f32::consts::TAU).cos().abs()) as u8;
            }
            commands.push(command(
                path(STAR, true),
                Some(star),
                Some(([255, 255, 255, 255], 2.0)),
            ));
            animation.push(CompiledFrame { commands });
        }
        actions.insert(
            action.as_str().into(),
            CompiledAnimation {
                still_frame: 0,
                frames: animation,
            },
        );
    }
    CompiledTheme {
        id: format!("org.apache.maka.cursor.{}", color.name()),
        name: format!("Maka {}", color.name()),
        version: "1.0.0".into(),
        author: "Apache Maka".into(),
        license: "Apache-2.0".into(),
        profile: cursor_overlay::THEME_PROFILE.into(),
        source_hash: Sha256::digest(include_bytes!("theme.rs")).into(),
        hotspot: [30, 30],
        actions,
    }
}
pub(crate) fn write_artifacts(directory: &Path) -> Result<(), String> {
    for color in Color::ALL {
        let theme = compiled(color);
        let bytes = cursor_overlay::encode_theme(&theme).map_err(|e| e.to_string())?;
        std::fs::write(directory.join(format!("{}.cua-theme", theme.id)), bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn svg_path(points: &[[f32; 2]], closed: bool) -> String {
    let mut value = String::new();
    for (i, [x, y]) in points.iter().enumerate() {
        use std::fmt::Write;
        let _ = write!(value, "{} {x} {y} ", if i == 0 { "M" } else { "L" });
    }
    if closed {
        value.push('Z');
    }
    value
}
pub(crate) fn svg(color: Color) -> String {
    let [r, g, b, _] = color.rgba();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" fill="none"><g stroke-linecap="round" stroke-linejoin="round"><path d="{body}" stroke="rgb({r},{g},{b})" stroke-width="13" opacity=".15"/><path d="{body}" fill="rgb({r},{g},{b})" stroke="white" stroke-width="4"/><path d="{mark}" stroke="white" stroke-width="5"/><path d="{star}" fill="rgb({r},{g},{b})" stroke="white" stroke-width="2"/></g></svg>"##,
        body = svg_path(BODY, true),
        mark = svg_path(MARK, false),
        star = svg_path(STAR, true)
    )
}
