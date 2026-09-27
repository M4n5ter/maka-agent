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

use super::remote::remote;
use super::{Output, Request};
use maka_client::Client;
use maka_protocol::plugin::{RemoteBinding, RemoteKind, RemoteRequest, RemoteResult};
use serde_json::json;

pub(super) async fn skills(
    client: &Client,
    request: &Request,
    page: &Option<(String, String)>,
) -> Result<Output, String> {
    let binding = RemoteBinding::Package {
        package_id: crate::pages::skills::PROVIDER.into(),
        method: "request".into(),
        session_id: Some(request.context.session.clone()),
    };
    let RemoteResult::Bound {
        target,
        handler: RemoteKind::Method,
    } = request
        .cancel
        .read(client.plugin_remote(RemoteRequest::Bind {
            binding: binding.clone(),
        }))
        .await?
    else {
        return Err("Skills did not publish a method".into());
    };
    let value = remote(client, request, binding, target, json!({"kind":"invocable","page":page.as_ref().map(|(revision,cursor)| json!({"revision":revision,"cursor":cursor}))})).await?;
    let result: maka_skills::api::InvocableResult =
        serde_json::from_value(value).map_err(|error| error.to_string())?;
    if let maka_skills::api::InvocableResult::Page {
        revision,
        items,
        next_cursor,
    } = &result
    {
        let mut seen = std::collections::HashSet::new();
        if revision.is_empty()
            || page.as_ref().is_some_and(|(old, _)| old != revision)
            || next_cursor
                .as_ref()
                .is_some_and(|cursor| page.as_ref().is_some_and(|(_, old)| old == cursor))
            || items.iter().any(|item| {
                !seen.insert(&item.id)
                    || crate::pages::skills::validate(&[crate::pages::skills::Picked {
                        id: item.id.clone(),
                        name: item.name.clone(),
                    }])
                    .is_err()
            })
        {
            return Err("Invalid Skills candidate page".into());
        }
    }
    Ok(Output::Skills(result))
}
