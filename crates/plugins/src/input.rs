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

//! Pure input preparation; final authority and durable receipts stay with Host.
pub mod resources;
use crate::{
    Error,
    composition::Scope,
    contributions::{Catalog, Contribution},
    fiber::CallGuard,
};
use futures_util::future::BoxFuture;
use maka_runtime::{
    composition::SourceKind,
    input::{InputReceipt, MessageInput},
};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

use crate::revision::Basis;

#[derive(Clone)]
pub struct Request {
    pub session_id: String,
    pub cwd: String,
    pub content: MessageInput,
    pub selections: maka_runtime::input::Selections,
    pub selection_sources: maka_runtime::input::SelectionSources,
    pub tools: BTreeSet<String>,
    pub cancellation: CancellationToken,
}

pub trait Provider: Send + Sync {
    fn prepare(
        &self,
        request: Request,
        workspace: crate::filesystem::ReadDirectory,
    ) -> BoxFuture<'static, Result<Outcome, Error>>;
}
pub struct InputPreparation {
    provider: Arc<dyn Provider>,
    resources: Option<resources::Binding>,
}
impl InputPreparation {
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        Self {
            provider,
            resources: None,
        }
    }
    pub fn resources(&self) -> Option<&resources::Binding> {
        self.resources.as_ref()
    }
}

pub enum Outcome {
    Unchanged,
    Ready {
        content: MessageInput,
        /// Domain-owned, bounded receipt. Host stamps the publishing identity.
        receipt: Value,
        required_tools: BTreeSet<String>,
        basis: Option<Basis>,
    },
    Blocked {
        message: String,
        receipt: Value,
    },
}

pub struct Prepared {
    pub content: MessageInput,
    pub required_tools: BTreeSet<String>,
    pub blocked: Option<String>,
    validity: Vec<(Contribution<InputPreparation>, Option<Basis>)>,
    resources: Vec<Contribution<crate::remote::Endpoint>>,
}
pub struct Admission {
    _owners: Vec<CallGuard>,
    _revisions: Vec<tokio::sync::OwnedRwLockReadGuard<u64>>,
}

impl Prepared {
    pub fn unchanged(content: MessageInput) -> Self {
        Self {
            content,
            required_tools: BTreeSet::new(),
            blocked: None,
            validity: Vec::new(),
            resources: Vec::new(),
        }
    }
    pub fn admit(&self) -> Result<Option<Admission>, Error> {
        let mut owners = Vec::new();
        let mut revisions = Vec::new();
        for source in &self.resources {
            owners.push(source.admit()?);
        }
        for (source, basis) in &self.validity {
            if let Some(basis) = basis {
                let Some(guard) = basis.admit() else {
                    return Ok(None);
                };
                revisions.push(guard);
            }
            owners.push(source.admit()?);
        }
        Ok(Some(Admission {
            _owners: owners,
            _revisions: revisions,
        }))
    }
}

pub async fn prepare(
    catalog: &Catalog,
    scope: &Scope,
    mut request: Request,
    workspace: &crate::filesystem::ReadRoot,
) -> Result<Prepared, Error> {
    maka_runtime::input::validate_selection_sources(
        &request.selections,
        &request.selection_sources,
    )
    .map_err(|error| Error::Invalid(error.into()))?;
    maka_runtime::input::validate_selection_session(
        &request.selection_sources,
        &request.session_id,
    )
    .map_err(|error| Error::Invalid(error.into()))?;
    let captured = catalog.capture(scope);
    let providers = captured.typed::<InputPreparation>().entries;
    let endpoints = captured.typed::<crate::remote::Endpoint>();
    let mut resource_pairs = Vec::new();
    for name in request.selections.keys() {
        let source = providers
            .get(name)
            .ok_or_else(|| Error::Invalid(format!("input preparation {name} is unavailable")))?;
        let supplied = request
            .selection_sources
            .iter()
            .find(|value| &value.provider == name);
        match (source.value.resources(), supplied) {
            (None, None) => {}
            (Some(binding), Some(expected)) => {
                let identity = source.owner.identity()?;
                if expected.package_id != identity.package_id
                    || expected.entry_id != identity.entry_id
                    || expected.activation != identity.activation
                    || expected.registration != binding.registration
                {
                    return Err(Error::Retired);
                }
                let endpoint = resources::endpoint(source, &endpoints)?;
                drop(source.admit()?);
                drop(endpoint.admit()?);
                resource_pairs.push(endpoint);
            }
            _ => {
                return Err(Error::Invalid(
                    "Input resource source does not match its provider".into(),
                ));
            }
        }
    }
    let mut result = Prepared {
        content: request.content.clone(),
        required_tools: BTreeSet::new(),
        blocked: None,
        validity: Vec::new(),
        resources: resource_pairs,
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    for (name, source) in providers {
        let _call = source.admit()?;
        let _resource =
            if source.value.resources().is_some() && request.selections.contains_key(&name) {
                Some(resources::endpoint(&source, &endpoints)?.admit()?)
            } else {
                None
            };
        let stopping = source.owner.stopping()?;
        request.content = result.content.clone();
        let cancellation = request.cancellation.child_token();
        let _closed = cancellation.clone().drop_guard();
        let files = workspace.bind(source.owner.clone(), cancellation);
        let outcome = tokio::select! {
            biased;
            _ = request.cancellation.cancelled() => return Err(Error::Invalid("input preparation cancelled".into())),
            _ = stopping.cancelled() => return Err(Error::Retired),
            outcome = tokio::time::timeout_at(deadline, source.value.provider.prepare(request.clone(), files)) =>
                outcome.map_err(|_| Error::Invalid("input preparation timed out".into()))??,
        };
        let (receipt, basis) = match outcome {
            Outcome::Unchanged => {
                if source.value.resources().is_some() && request.selections.contains_key(&name) {
                    return Err(Error::Invalid(
                        "Selected input resources were not prepared".into(),
                    ));
                }
                continue;
            }
            Outcome::Ready {
                mut content,
                receipt,
                required_tools,
                basis,
            } => {
                if !required_tools.is_subset(&request.tools) {
                    return Err(Error::Invalid(
                        "prepared input requires unavailable tools".into(),
                    ));
                }
                if content.attachments != result.content.attachments {
                    return Err(Error::Invalid(
                        "input preparation cannot replace attachments".into(),
                    ));
                }
                // Providers cannot rewrite another provider's evidence.
                content.preparation = std::mem::take(&mut result.content.preparation);
                result.content = content;
                result.required_tools.extend(required_tools);
                (receipt, basis)
            }
            Outcome::Blocked { message, receipt } => {
                if message.trim().is_empty() || message.len() > 4096 {
                    return Err(Error::Invalid("input rejection exceeds its budget".into()));
                }
                result.blocked = Some(message);
                (receipt, None)
            }
        };
        let revision = crate::prompt::source(&source, SourceKind::Input, &name, &receipt)?;
        result.content.preparation.push(InputReceipt {
            source: revision,
            receipt,
        });
        if result.content.text_bytes() > 64 * 1024
            || serde_json::to_vec(&result.content)
                .map_err(|error| Error::Invalid(error.to_string()))?
                .len()
                > 64 * 1024
        {
            return Err(Error::Invalid("prepared input exceeds 64 KiB".into()));
        }
        result.validity.push((source, basis));
        if result.blocked.is_some() {
            break;
        }
    }
    Ok(result)
}
