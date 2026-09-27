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

use super::*;

impl Pages {
    /// Seed the real page state for read-only presentation tests. No worker,
    /// authorization, process or installer is started by this test fixture.
    pub(in crate::plugin::terminal) fn observed_for_test(
        caller: &Caller,
        checked: Checked,
        phase: Option<Phase>,
    ) -> Self {
        let attempt = phase.map(|phase| Attempt {
            id: Uuid::new_v4(),
            agent: checked.agent.clone(),
            method: checked.methods[0]["id"].as_str().unwrap().to_owned(),
            revision: checked.revision,
            phase,
            url: (phase == Phase::Pending)
                .then(|| "https://example.test/sign-in?challenge=fixture".into()),
            url_revision: 1,
            stop: CancellationToken::new(),
        });
        let (state, _) = watch::channel(attempt);
        let (commands, _) = mpsc::channel(1);
        let (installation, _) = watch::channel(None);
        let page = Arc::new(Page {
            state,
            commands,
            installation,
            checked: Mutex::new(Some(checked)),
        });
        Self(Arc::new(Mutex::new(BTreeMap::from([(
            (caller.connection_id, caller.document_id),
            page,
        )]))))
    }
}
