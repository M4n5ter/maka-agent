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

use super::Target;
use maka_protocol::{configuration::policy::*, session::*};
use serde::{Deserialize, Serialize};

/// Deserialization-only schema for uncertain writes saved by the old native UI.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Purpose {
    Preferences,
    Executor,
    NewExecutor,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyTarget {
    root: String,
    epoch: String,
    session: String,
    name: String,
    revision: u64,
    read_message: Option<String>,
    run: Option<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<maka_runtime::execution::WorkspaceTarget>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "input",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Mutation {
    Policy(RuntimePolicyMutationInput),
    Executor(SessionConfigurationUpdateInput),
    Create(SessionCreateInput),
}
impl Mutation {
    pub(super) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Policy(input) => {
                if !matches!(
                    input.operation,
                    RuntimePolicyMutation::SetPersonalization { .. }
                        | RuntimePolicyMutation::SetWorkspaceInstructions { .. }
                ) {
                    return Err("Unsupported preference mutation".into());
                }
                maka_protocol::runtime_policy::decode_mutation_input(&serde_json::json!(input))
                    .map(|_| ())
            }

            Self::Executor(input) => {
                decode_session_configuration_update_input(&serde_json::json!(input)).map(|_| ())
            }
            Self::Create(input) => {
                decode_session_create_input(&serde_json::json!(input)).map(|_| ())
            }
        }
        .map_err(|e| e.to_string())
    }
    fn session(&self) -> Option<&str> {
        match self {
            Self::Policy(_) => None,
            Self::Executor(i) => Some(&i.session_id),
            Self::Create(i) => Some(&i.session_id),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    target: LegacyTarget,
    page: Purpose,
    mutation: Mutation,
}
impl Checkpoint {
    pub(super) fn target(&self) -> Target {
        let t = &self.target;
        Target {
            root: t.root.clone(),
            epoch: t.epoch.clone(),
            session: t.session.clone(),
            name: t.name.clone(),
            revision: t.revision,
            read_message: t.read_message.clone(),
            run: t.run.clone(),
        }
    }
    pub fn validate(&self, root: &str) -> Result<(), String> {
        let t = &self.target;
        if t.root != root
            || t.epoch.is_empty()
            || t.epoch.len() > 128
            || t.name.len() > 4096
            || self.mutation.session().is_some_and(|s| s != t.session)
        {
            return Err("Saved session operation changed its destination".into());
        }
        let valid = match (&self.mutation, self.page) {
            (Mutation::Policy(_), Purpose::Preferences) => t.session.is_empty(),
            (Mutation::Executor(i), Purpose::Executor) => {
                i.expected_revision == t.revision && i.patch.executor_target.is_some()
            }
            (Mutation::Create(i), Purpose::NewExecutor) => {
                t.workspace.as_ref() == Some(&i.workspace)
                    && t.revision == 0
                    && matches!(i.target, SessionCreateTarget::Executor { .. })
            }
            _ => false,
        };
        if !valid {
            return Err("Saved session operation changed its purpose".into());
        }
        self.mutation.validate()
    }
}
