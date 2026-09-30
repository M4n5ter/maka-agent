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

use maka_runtime::tools::ToolDefinition;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evaluate {
    pub code: String,
    /// Wall-clock limit including asynchronous waits. Interrupted REPLs require reset.
    #[serde(default = "default_timeout")]
    #[schemars(range(min = 1, max = 120000))]
    pub timeout_ms: u64,
    /// Short, user-facing description of this call, in the user’s language.
    #[schemars(length(max = 256))]
    pub title: Option<String>,
}
fn default_timeout() -> u64 {
    30_000
}

pub fn definitions() -> Vec<ToolDefinition> {
    [
        ("cua_repl", "Execute JavaScript in the persistent Computer Use REPL. Independent of Code Mode. Start with await cua.getState() or const app = await cua.getApp(...). App selection emits initial accessibility state and API documentation. Variables and bound targets survive calls and Turns while this Host runtime is alive; follow the current REPL state in context after a restart. Discovery, selection and observation methods emit their results automatically: call them directly, without nodeRepl.write or nodeRepl.emitImage wrappers. Use emit:false only to process a result yourself; inventory errors remain visible. No Node, filesystem or arbitrary network APIs. After actions, observe with getAXState before deciding the next action. Cancellation/timeout terminates the REPL; use cua_reset then rebind targets. Never automatically replay an uncertain action.", serde_json::to_value(schemars::schema_for!(Evaluate)).unwrap()),
        ("cua_reset", "Reset only this Session's Computer Use REPL, native bindings and observations. Waits for admitted actions to settle. Does not close user applications or change Code Mode.", serde_json::json!({"type":"object","properties":{},"additionalProperties":false})),
    ].into_iter().map(|(name, description, input_schema)| ToolDefinition {
        name:name.into(), description:description.into(), input_schema,
        output_schema:None, provider:None, freeform:None,
    }).collect()
}

pub fn validate(name: &str, input: &Value) -> Result<(), String> {
    match name {
        "cua_repl" => {
            let input: Evaluate =
                serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
            if input.code.len() > 64 * 1024 || !(1..=120_000).contains(&input.timeout_ms) {
                return Err(
                    "Cua code must fit 64 KiB; timeout_ms must be between 1 and 120000".into(),
                );
            }
            Ok(())
        }
        "cua_reset" if input.as_object().is_some_and(|v| v.is_empty()) => Ok(()),
        _ => Err("unknown Computer Use tool or invalid input".into()),
    }
}
