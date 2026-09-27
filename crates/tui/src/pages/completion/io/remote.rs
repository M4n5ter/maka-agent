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

use super::Request;
use maka_client::{Client, RequestFailure};
use maka_protocol::plugin::{
    self, InputResourceProjection, InputResourceReply, InputResourceRequest, RemoteBinding,
    RemoteRequest, RemoteResult,
};
use serde_json::Value;

pub(super) async fn resource(
    client: &Client,
    request: &Request,
    provider: &InputResourceProjection,
    input: InputResourceRequest,
) -> Result<InputResourceReply, String> {
    let binding = RemoteBinding::Package {
        package_id: provider.package_id.clone(),
        method: provider.method.clone(),
        session_id: Some(request.context.session.clone()),
    };
    let value = remote(
        client,
        request,
        binding,
        provider.target.clone(),
        serde_json::to_value(input).map_err(|error| error.to_string())?,
    )
    .await?;
    plugin::decode_input_resource_reply(&value).map_err(|error| error.to_string())
}

pub(super) async fn remote(
    client: &Client,
    request: &Request,
    binding: RemoteBinding,
    target: maka_plugins::remote::Target,
    input: Value,
) -> Result<Value, String> {
    // Opening must settle even after cancellation, so its allocated ID can be
    // closed. The driver never aborts this job while its connection is live.
    let document = match client.plugin_remote(RemoteRequest::OpenDocument).await {
        Ok(RemoteResult::Document { document }) => document,
        Err(error @ RequestFailure::Unknown(_)) => {
            // A late reply can contain an allocated document we cannot name.
            // Retire this connection so Host settles that ownership as well.
            client.disconnect();
            return Err(format!(
                "Remote document ownership is unknown; connection retired: {error}"
            ));
        }
        Err(error) => return Err(error.to_string()),
        Ok(_) => {
            client.disconnect();
            return Err("Remote document ownership is invalid; connection retired".into());
        }
    };
    let result = request
        .cancel
        .read(client.plugin_remote(RemoteRequest::Call {
            binding,
            target,
            document,
            input,
        }))
        .await;
    if let Err(error) = client
        .plugin_remote(RemoteRequest::CloseDocument { document })
        .await
    {
        client.disconnect();
        return Err(format!(
            "Remote document cleanup was not confirmed; connection retired: {error}"
        ));
    }
    let RemoteResult::Value { value } = result? else {
        return Err("Missing Remote value".into());
    };
    Ok(value)
}

#[cfg(test)]
mod tests;
