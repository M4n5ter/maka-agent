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

//! Agent-only access to the Host's interactive desktop.
//!
//! The Host binds interaction state to the Session, owns approval and journals each
//! operation. A call never selects a client desktop, native session, or driver.

use crate::call::Scope;
use futures_util::future::BoxFuture;
use maka_runtime::{capability::CallResult, tools::ToolError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub name: String,
    pub input: Value,
    #[serde(default)]
    pub browsers: Vec<BrowserConnection>,
}

/// An explicitly configured browser provider. The REPL cannot supply endpoints,
/// restart a browser, switch profiles or enable remote debugging by itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserConnection {
    pub id: String,
    pub endpoint: String,
}

pub trait Computer: Send + Sync {
    /// Returns the native MCP-shaped result, including image content and
    /// structured refusals. Returning must include native operation settlement.
    fn call(&self, scope: Scope, input: Call) -> BoxFuture<'_, Result<CallResult, ToolError>>;
}
