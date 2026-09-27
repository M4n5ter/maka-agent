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

use super::{Context, Page, Provider, Query, Reply, Request, Resolve};
use crate::{
    Error,
    fiber::Context as Owner,
    remote::{self, Caller},
};
use futures_util::future::BoxFuture;
use maka_runtime::input::SelectionSource;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

pub(super) struct Resource {
    pub provider: Arc<dyn Provider>,
    pub owner: Owner,
    pub name: String,
    pub registration: Uuid,
    pub retired: tokio_util::sync::CancellationToken,
}
impl remote::Method for Resource {
    fn call(
        &self,
        input: Value,
        caller: Caller,
    ) -> BoxFuture<'static, Result<Value, remote::Error>> {
        let provider = self.provider.clone();
        let owner = self.owner.clone();
        let name = self.name.clone();
        let registration = self.registration;
        let retired = self.retired.clone();
        Box::pin(async move {
            let request: Request =
                serde_json::from_value(input).map_err(|e| remote::Error::Invalid(e.to_string()))?;
            request.validate().map_err(failure)?;
            let identity = owner.identity().map_err(|_| remote::Error::Retired)?;
            let session_id = caller.session_id.clone().ok_or_else(|| {
                remote::Error::Invalid("Input resources require a Session".into())
            })?;
            let source = SelectionSource {
                provider: name,
                package_id: identity.package_id,
                entry_id: identity.entry_id,
                activation: identity.activation,
                registration,
                session_id: session_id.clone(),
            };
            let cancellation = caller.cancellation.child_token();
            let caller_cancellation = caller.cancellation.clone();
            let _closed = cancellation.clone().drop_guard();
            let stopping = owner.stopping().map_err(|_| remote::Error::Retired)?;
            if cancellation.is_cancelled() {
                return Err(remote::Error::Cancelled);
            }
            if stopping.is_cancelled() || retired.is_cancelled() {
                return Err(remote::Error::Retired);
            }
            let work = async {
                let view = caller.views.session().await?;
                if cancellation.is_cancelled() {
                    return Err(remote::Error::Cancelled);
                }
                let context = Context {
                    session_id,
                    workspace: view.files,
                    cancellation: cancellation.clone(),
                };
                let reply = match request {
                    Request::Query {
                        query,
                        cursor,
                        limit,
                        locale,
                    } => {
                        let page = provider
                            .query(
                                Query {
                                    query,
                                    cursor,
                                    limit,
                                    locale,
                                },
                                context,
                            )
                            .await
                            .map_err(failure)?;
                        page.validate(limit).map_err(failure)?;
                        let Page { items, next_cursor } = page;
                        Reply::Page { items, next_cursor }
                    }
                    Request::Resolve { id, locale } => {
                        let expected = id.clone();
                        let value = provider
                            .resolve(Resolve { id, locale }, context)
                            .await
                            .map_err(failure)?;
                        value.validate().map_err(failure)?;
                        if value.selector != expected {
                            return Err(remote::Error::Invalid(
                                "Resource resolution changed the selected identity".into(),
                            ));
                        }
                        Reply::Resolved {
                            source: Box::new(source),
                            selector: value.selector,
                            label: value.label,
                            quote: value.quote,
                        }
                    }
                };
                super::types::budget(&reply).map_err(failure)?;
                serde_json::to_value(reply).map_err(|e| remote::Error::Invalid(e.to_string()))
            };
            tokio::pin!(work);
            let interrupted = tokio::select! {
                biased;
                _ = cancellation.cancelled() => remote::Error::Cancelled,
                _ = stopping.cancelled() => remote::Error::Retired,
                _ = retired.cancelled() => remote::Error::Retired,
                _ = tokio::time::sleep(Duration::from_secs(10)) => remote::Error::Provider("Input resource query timed out".into()),
                result = &mut work => return result,
            };
            // The existing Remote owner bounds cancellation settlement. Signal
            // it on our own deadline/retirement, then keep polling admitted work
            // so callback cancellation and borrowed-reader cleanup can finish.
            caller_cancellation.cancel();
            match work.await {
                Err(remote::Error::CleanupUnconfirmed) => Err(remote::Error::CleanupUnconfirmed),
                _ => Err(interrupted),
            }
        })
    }
}
fn failure(error: Error) -> remote::Error {
    match error {
        Error::Retired => remote::Error::Retired,
        Error::Cleanup(_) | Error::CleanupPending => remote::Error::CleanupUnconfirmed,
        other => remote::Error::Invalid(other.to_string()),
    }
}
