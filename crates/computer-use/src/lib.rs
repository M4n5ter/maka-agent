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

//! Independent Computer Use REPL and Session-owned Cua targets.
//!
//! Host supplies authorization and journal ownership; this crate supplies the
//! pinned native implementation. Perception and driver administration are absent.

mod browser;
mod catalog;
pub mod cursor;
pub mod desktop;
mod driver;
mod identity;
#[cfg(target_os = "macos")]
mod macos;
pub mod plugin;
pub mod protocol;
mod session;

pub use catalog::{Evaluate, definitions, validate};
pub use desktop::Session;
pub use driver::Driver;
pub fn facade() -> String {
    facade_for(desktop::local_platform())
}

pub fn facade_for(platform: &str) -> String {
    format!(
        "const nativePlatform = {platform:?};\n{}",
        include_str!("facade.js")
    )
}

pub const ID: &str = "maka.computer-use";
pub const REVISION: &str = "cua-repl-v1";
