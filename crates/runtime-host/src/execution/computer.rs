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

use maka_js_runtime::repl::Repl;
use maka_runtime::tools::ToolError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(super) struct Computers {
    sessions: Mutex<HashMap<String, Arc<Interaction>>>,
    /// All sessions share the same physical keyboard/pointer/clipboard.
    pub(super) input: Arc<tokio::sync::Mutex<()>>,
    pub(super) driver: Arc<tokio::sync::Mutex<maka_computer_use::Driver>>,
}
pub(super) struct Interaction {
    pub(super) repl: tokio::sync::Mutex<Option<Repl>>,
    pub(super) native: Arc<tokio::sync::Mutex<maka_computer_use::Session>>,
}
impl Computers {
    pub(super) fn get(&self, session: &str) -> Result<Arc<Interaction>, ToolError> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(existing) = sessions.get(session) {
            return Ok(existing.clone());
        }
        if sessions.len() >= 16 {
            return Err(ToolError::Failed(
                "Computer Use session capacity reached; retire an unused Session".into(),
            ));
        }
        let interaction = Arc::new(Interaction {
            repl: Default::default(),
            native: Arc::new(tokio::sync::Mutex::new(maka_computer_use::Session::new(
                self.driver.clone(),
            ))),
        });
        sessions.insert(session.into(), interaction.clone());
        Ok(interaction)
    }
    pub(super) async fn retire(&self, session: &str) -> Result<(), ToolError> {
        let interaction = self.sessions.lock().unwrap().get(session).cloned();
        if let Some(interaction) = interaction {
            interaction.close().await?;
            self.sessions.lock().unwrap().remove(session);
        }
        Ok(())
    }
    pub(super) async fn shutdown(&self) -> Result<(), ToolError> {
        let sessions: Vec<_> = self.sessions.lock().unwrap().keys().cloned().collect();
        let mut failure = None;
        for session in sessions {
            if let Err(error) = self.retire(&session).await {
                failure.get_or_insert(error);
            }
        }
        if let Err(error) = self.driver.lock().await.close().await {
            failure.get_or_insert(error);
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Interaction {
    async fn close(&self) -> Result<(), ToolError> {
        if let Some(repl) = self.repl.lock().await.take() {
            repl.close()
                .await
                .map_err(|e| ToolError::CleanupUnconfirmed(e.to_string()))?;
        }
        self.native.lock().await.close().await
    }
}
