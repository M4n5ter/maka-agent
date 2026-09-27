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

use super::{SecretAction, basis};
use crate::editor::Editor;
use maka_protocol::configuration::{
    CredentialStatus,
    policy::{
        NetworkProxy, RuntimePolicySnapshot, network_test,
        network_update::{self, CredentialTarget, CredentialUpdate},
    },
};

pub(super) struct Proxy {
    pub snapshot: RuntimePolicySnapshot,
    pub status: CredentialStatus,
    pub value: NetworkProxy,
    pub secret_action: SecretAction,
    pub fields: Vec<Editor>,
    pub test: Option<network_test::Output>,
}
impl Proxy {
    pub fn new(snapshot: RuntimePolicySnapshot, status: CredentialStatus) -> Self {
        let value = snapshot.policy.network_proxy.clone();
        let values = [
            &value.host,
            &value.port.to_string(),
            &value.username,
            "",
            &value.bypass_list.join(", "),
            &value.auto_bypass_domains.join(", "),
            "https://www.google.com/generate_204",
        ];
        let fields = values
            .into_iter()
            .map(|v| {
                let mut editor = Editor::bounded(16 * 1024, "connection-preferences-limit");
                editor.insert(v);
                editor.clear_history();
                editor
            })
            .collect();
        Self {
            snapshot,
            status,
            value,
            secret_action: SecretAction::Keep,
            fields,
            test: None,
        }
    }
    pub fn update(&self) -> Result<network_update::Update, &'static str> {
        if self.fields.iter().any(|e| e.error.is_some()) {
            return Err("connection-preferences-invalid");
        }
        let mut value = self.value.clone();
        value.host = self.fields[0].text().into();
        value.port = self.fields[1]
            .text()
            .parse()
            .map_err(|_| "connection-preferences-invalid")?;
        value.username = self.fields[2].text().into();
        let patterns = |raw: &str| {
            raw.split([',', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        };
        value.bypass_list = patterns(self.fields[4].text());
        value.auto_bypass_domains = patterns(self.fields[5].text());
        if value.auth_enabled
            && self.secret_action == SecretAction::Keep
            && basis(&self.status).is_some()
            && CredentialTarget::from_proxy(&value)
                != CredentialTarget::from_proxy(&self.snapshot.policy.network_proxy)
        {
            return Err("proxy-target-changed");
        }
        let credential = if !value.auth_enabled {
            CredentialUpdate::Delete {}
        } else {
            match self.secret_action {
                SecretAction::Keep => CredentialUpdate::Keep {},
                SecretAction::Delete => CredentialUpdate::Delete {},
                SecretAction::Replace => CredentialUpdate::Replace {
                    secret: self.fields[3].text().into(),
                    expected_target: Some(CredentialTarget::from_proxy(
                        &self.snapshot.policy.network_proxy,
                    )),
                },
            }
        };
        let mut update = network_update::Update {
            expected_policy_revision: self.snapshot.revision,
            expected_credential: basis(&self.status),
            network_proxy: value,
            credential,
        };
        update
            .normalize()
            .map_err(|_| "connection-preferences-invalid")?;
        Ok(update)
    }
    pub fn changed(&self) -> bool {
        self.update().is_ok_and(|u| {
            u.network_proxy != self.snapshot.policy.network_proxy
                || !matches!(u.credential, CredentialUpdate::Keep {})
                    && (self.secret_action == SecretAction::Replace
                        || basis(&self.status).is_some())
        })
    }
    pub fn test_input(&self) -> Result<network_test::Input, &'static str> {
        if self.changed() || self.update().is_err() {
            return Err("proxy-save-before-test");
        }
        let input = network_test::Input {
            network_proxy: Some(self.snapshot.policy.network_proxy.clone()),
            url: Some(self.fields[6].text().into()),
            timeout_ms: Some(10_000),
        };
        maka_protocol::network_proxy::decode_input(
            &serde_json::to_value(&input).expect("wire input"),
        )
        .map_err(|_| "connection-preferences-invalid")
    }
    pub fn committed(&mut self, revision: u64, status: CredentialStatus) {
        if let Ok(update) = self.update() {
            self.snapshot.policy.network_proxy = update.network_proxy.clone();
            self.value = update.network_proxy;
        }
        self.snapshot.revision = revision;
        self.status = status;
        self.secret_action = SecretAction::Keep;
        self.fields[3] = Editor::bounded(10_240, "connection-preferences-limit");
        self.test = None;
    }
}
