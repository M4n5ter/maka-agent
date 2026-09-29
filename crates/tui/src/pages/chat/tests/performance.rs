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

//! Opt-in measurements of page installation and scrolling, including presentation.
use super::*;
use std::time::Instant;

#[test]
#[ignore = "prints transcript page and frame timings"]
fn page_and_scroll_benchmark() {
    let i18n = I18n::new(
        crate::LocalePreference::Explicit(crate::Locale::En),
        crate::Locale::En,
    );
    for media in [false, true] {
        let mut timings = Vec::new();
        let mut scroll_samples = Vec::new();
        let mut retained = 0;
        for _ in 0..20 {
            let mut chat = Chat::default();
            chat.select(&Route::Session("a".into()));
            let request = chat.open_query().unwrap();
            let mut initial = opened();
            initial.batch = batch(1, 80, 80);
            if media {
                initial.batch.rows[39].value = json!({"type":"tool_result", "id":"m40", "turnId":"turn", "toolUseId":"call", "isError":false, "content":{"kind":"json","value":{"content":[{"type":"image","mimeType":"image/png","data":"A".repeat(5 * 1024 * 1024)}],"structuredContent":{"ok":true}}}});
            }
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
            // Production assembly has already verified this digest off the UI thread.
            for row in &mut initial.batch.rows {
                use sha2::{Digest, Sha256};
                row.payload_digest = Some(format!(
                    "sha256:{:x}",
                    Sha256::digest(serde_json::to_vec(&row.value).unwrap())
                ));
            }
            let prepare = Instant::now();
            initial.batch = media::prepare(initial.batch);
            let prepare = prepare.elapsed();
            let start = Instant::now();
            chat.opened(request, Ok(initial));
            let merge = start.elapsed();
            terminal
                .draw(|f| {
                    chat.draw(f, f.area(), &i18n, false);
                })
                .unwrap();
            let frame = start.elapsed() - merge;
            let mut scroll = Vec::new();
            for i in 0..100 {
                let start = Instant::now();
                chat.scroll(i < 50, 3);
                terminal
                    .draw(|f| {
                        chat.draw(f, f.area(), &i18n, false);
                    })
                    .unwrap();
                scroll.push(start.elapsed().as_micros());
            }
            scroll_samples.extend(scroll);
            retained = chat.bytes;
            assert_eq!(chat.rows.len(), 80);
            assert!(retained < 100_000);
            timings.push((merge.as_micros(), frame.as_micros(), prepare.as_micros()));
        }
        let stats = |mut samples: Vec<u128>| {
            samples.sort_unstable();
            json!({"p50":samples[samples.len().div_ceil(2)-1], "p95":samples[(samples.len()*95).div_ceil(100)-1], "max":samples.last()})
        };
        println!(
            "{}",
            json!({
                "benchmark":"transcript_page", "media":media,
                "debug_assertions":cfg!(debug_assertions), "samples":timings.len(),
                "merge_us":stats(timings.iter().map(|sample| sample.0).collect()),
                "first_frame_us":stats(timings.iter().map(|sample| sample.1).collect()),
                "prepare_us":stats(timings.iter().map(|sample| sample.2).collect()),
                "scroll_us":stats(scroll_samples),
                "retained_accounted_bytes":retained,
                "limits":"page installation and TestBackend rendering; excludes transport, worker scheduling, terminal presentation and RSS"
            })
        );
    }
}
