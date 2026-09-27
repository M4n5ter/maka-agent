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

//! Optional discovery is an opt-in to queueable context selectors owned by
//! this same input provider. Domain preparation must validate every selector.
mod remote;
mod types;
use super::InputPreparation;
use crate::{
    Error,
    contributions::{Contribution, Publisher, Snapshot, Staged},
    fiber::Context as Owner,
};
use futures_util::future::BoxFuture;
use std::{any::TypeId, sync::Arc};
use tokio_util::sync::CancellationToken;
pub use types::*;
use uuid::Uuid;

#[derive(Clone)]
pub struct Context {
    pub session_id: String,
    pub workspace: crate::filesystem::ReadDirectory,
    pub cancellation: CancellationToken,
}
pub trait Provider: Send + Sync {
    fn query(&self, request: Query, context: Context) -> BoxFuture<'static, Result<Page, Error>>;
    fn resolve(
        &self,
        request: Resolve,
        context: Context,
    ) -> BoxFuture<'static, Result<Value, Error>>;
}
pub struct Resources {
    pub descriptor: Descriptor,
    pub provider: Arc<dyn Provider>,
}
pub struct Binding {
    pub descriptor: Descriptor,
    pub method: String,
    pub registration: Uuid,
}
/// Stage both halves together. A registration's drop withdraws both; JS close
/// uses withdraw() to remove the same typed pair in one original-owner lock.
pub fn stage(
    staged: &mut Staged,
    owner: &Owner,
    name: &str,
    prepare: Arc<dyn super::Provider>,
    resources: Resources,
    content_digest: Option<String>,
) -> Result<(), Error> {
    resources.descriptor.validate()?;
    let identity = owner.identity()?;
    let method = method(name)?;
    let registration = Uuid::new_v4();
    let retired = CancellationToken::new();
    let handler = crate::remote::Handler::Method(Arc::new(remote::Resource {
        provider: resources.provider,
        owner: owner.clone(),
        name: name.into(),
        registration,
        retired: retired.clone(),
    }));
    let endpoint = match content_digest {
        Some(digest) => crate::remote::Endpoint::new(digest, handler),
        None => crate::remote::Endpoint::standalone(handler),
    }
    .with_registration(registration);
    staged.insert_pair(
        crate::remote::key(&identity.package_id, &method)?,
        endpoint,
        name,
        InputPreparation {
            provider: prepare,
            resources: Some(Binding {
                descriptor: resources.descriptor,
                method,
                registration,
            }),
        },
        retired,
    )
}

pub fn withdraw(publisher: &Publisher, owner: &Owner, names: &[String]) -> Result<(), Error> {
    let package = owner.identity()?.package_id;
    let mut keys = Vec::new();
    for name in names {
        keys.push((TypeId::of::<InputPreparation>(), name.clone()));
        keys.push((
            TypeId::of::<crate::remote::Endpoint>(),
            crate::remote::key(&package, &method(name)?)?,
        ));
    }
    publisher.withdraw_group(&keys)
}
fn method(name: &str) -> Result<String, Error> {
    crate::identifier(name)?;
    let method = format!("input.{name}");
    crate::identifier(&method)?;
    Ok(method)
}
/// Both contributions must still be the original same-owner publication.
pub fn endpoint(
    source: &Contribution<InputPreparation>,
    endpoints: &Snapshot<crate::remote::Endpoint>,
) -> Result<Contribution<crate::remote::Endpoint>, Error> {
    let binding = source
        .value
        .resources
        .as_ref()
        .ok_or_else(|| Error::Invalid("Input provider declares no resources".into()))?;
    let identity = source.owner.identity()?;
    let key = crate::remote::key(&identity.package_id, &binding.method)?;
    let endpoint = endpoints.entries.get(&key).ok_or(Error::Retired)?;
    let endpoint_owner = endpoint.owner.identity()?;
    if !source.is_effective()
        || !endpoint.is_effective()
        || source.registration_id() != endpoint.registration_id()
        || identity.package_id != endpoint_owner.package_id
        || identity.entry_id != endpoint_owner.entry_id
        || identity.activation != endpoint_owner.activation
        || endpoint.value.target(&endpoint_owner).registration != binding.registration
    {
        return Err(Error::Retired);
    }
    Ok(endpoint.clone())
}
