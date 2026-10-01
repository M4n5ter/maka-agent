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

use super::{State, draft::Input};
use crate::pages::selections::Picked;

impl State {
    pub(crate) fn selections_source(&self) -> Option<&str> {
        let saved = self.saved.as_ref()?;
        Some(if saved.stage == super::saved::Stage::Bindings {
            &saved.copy.target_session_id
        } else {
            &saved.copy.source_session_id
        })
    }
    fn selection_input(&self, session: &str, input: &str) -> Option<&Input> {
        self.saved
            .as_ref()
            .filter(|s| s.copy.target_session_id == session)?
            .inputs
            .iter()
            .find(|i| i.original.message_id == input)
    }
    pub(crate) fn selections(&self, session: &str, input: &str) -> Option<&[Picked]> {
        Some(&self.selection_input(session, input)?.selections)
    }
    pub(crate) fn selections_mut(
        &mut self,
        session: &str,
        input: &str,
    ) -> Option<&mut Vec<Picked>> {
        Some(
            &mut self
                .saved
                .as_mut()
                .filter(|s| s.copy.target_session_id == session)?
                .inputs
                .iter_mut()
                .find(|i| i.original.message_id == input)?
                .selections,
        )
    }
}
