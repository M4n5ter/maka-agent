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

//! Local composer CPU samples; transport, Host retrieval and physical presentation are excluded.
use super::*;
use serde_json::{Value, json};
use std::time::Instant;

mod fixtures;
mod scenarios;

const WARMUP: usize = 20;
const MEASURED: usize = 200;

struct Bench {
    app: App,
    terminal: Terminal<TestBackend>,
    inventory: usize,
    size: (u16, u16),
}
impl Bench {
    fn new(inventory: usize, size: (u16, u16)) -> Self {
        let mut app = app(Locale::En);
        app.chrome.motion = false;
        app.apps.directory = fixtures::commands(inventory);
        let mut bench = Self {
            app,
            terminal: Terminal::new(TestBackend::new(size.0, size.1)).unwrap(),
            inventory,
            size,
        };
        bench.draw();
        bench
    }
    fn draw(&mut self) {
        self.terminal
            .draw(|frame| crate::view::draw(frame, &mut self.app))
            .unwrap();
    }
    fn text(&mut self, text: &str) {
        self.app.completion_cancel();
        self.app.input(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        )));
        self.app.input(key(KeyCode::Backspace));
        type_text(&mut self.app, text);
        self.draw();
    }
    fn sample(
        &mut self,
        phase: &str,
        index: usize,
        operation: &str,
        change: impl FnOnce(&mut App),
    ) -> Value {
        let start = Instant::now();
        change(&mut self.app);
        let operation_ns = start.elapsed().as_nanos();
        let frame = Instant::now();
        self.draw();
        let frame_ns = frame.elapsed().as_nanos();
        // Fixture construction, assertions, counters and JSON are outside both regions.
        let popup = self.app.completion.popup.as_ref();
        let rows = popup.map_or(0, |popup| popup.candidates.len());
        let visible = popup.map_or(0, |popup| {
            popup
                .candidates
                .iter()
                .filter(|candidate| {
                    popup
                        .surface
                        .rect(&format!(
                            "completion/candidates/{}/title/name",
                            candidate.id
                        ))
                        .is_some_and(|rect| !rect.is_empty())
                })
                .count()
        });
        let contract_ok = rows <= io::PAGE
            && self.app.sending.is_empty()
            && self.app.drafts["chat"].marks().is_empty();
        assert!(
            contract_ok,
            "local discovery cannot submit or capture resources"
        );
        json!({"type":"sample", "benchmark":"composer_completion_cpu_v1", "phase":phase,
            "sample":index, "warmup":index < WARMUP, "inventory_items":self.inventory,
            "width":self.size.0, "height":self.size.1, "operation":operation,
            "operation_ns":operation_ns, "input_ns":(operation == "input").then_some(operation_ns),
            "callback_ns":(operation == "callback").then_some(operation_ns), "frame_ns":frame_ns,
            "local_total_ns":operation_ns + frame_ns, "host_query_ns":Value::Null,
            "loaded_candidates":rows, "visible_candidates":visible, "pending":self.app.completion.pending.is_some(),
            "draft_bytes":self.app.drafts["chat"].text().len(), "binding_bytes":self.app.completion_retained_bytes(),
            "contract_ok":contract_ok})
    }
}

#[test]
#[ignore = "release composer CPU measurement; run alone with --nocapture"]
fn composer_completion_cpu_samples() {
    if cfg!(debug_assertions) {
        panic!("run this measurement with --release");
    }
    let mut samples = Vec::new();
    for inventory in [32, 512, 4096] {
        for size in [(120, 40), (55, 24)] {
            scenarios::commands(&mut Bench::new(inventory, size), &mut samples);
            scenarios::references(&mut Bench::new(inventory, size), &mut samples);
            scenarios::pending(&mut Bench::new(inventory, size), &mut samples);
            scenarios::retirement(&mut Bench::new(inventory, size), &mut samples);
        }
    }
    println!(
        "{}",
        json!({"type":"metadata", "benchmark":"composer_completion_cpu_v1",
        "executable":std::env::current_exe().unwrap(), "package_version":env!("CARGO_PKG_VERSION"),
        "os":std::env::consts::OS, "arch":std::env::consts::ARCH, "debug_assertions":cfg!(debug_assertions),
        "samples":samples.len(), "warmup_per_repeated_phase":WARMUP, "measured_per_repeated_phase":MEASURED,
        "one_off_phases":["pending_provider_retirement", "retired_provider_reply", "pending_query_close"],
        "renderer":"App::input / App callbacks / view::draw / Terminal<TestBackend>",
        "clock":"std::time::Instant elapsed wall time in nanoseconds",
        "operation_ns":"only the named local input or response/catalog callback",
        "frame_ns":"full App frame, Ratatui buffer diff and TestBackend; no ANSI, PTY or physical display",
        "local_total_ns":"operation_ns + frame_ns; fixture generation and counter collection excluded",
        "host_query_ns":"null: Host retrieval and transport are not measured by this fixture",
        "pending":"a Request callback is held across all edits; after its cleanup settles, a current request is retired by a catalog event; no sleeps or simulated RTT claim",
        "inventory":"32/512/4096 are fixture sizes, not production quotas; @ inventory is delivered through existing 32-item pages",
        "comparison_reference_p95_ns":50_000_000_u64,
        "threshold_scope":"historical local interaction budget reference; these CPU samples do not establish input-to-physical-present latency"})
    );
    for sample in samples {
        println!("{sample}");
    }
}
