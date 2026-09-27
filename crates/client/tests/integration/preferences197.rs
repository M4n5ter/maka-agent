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

use super::connection::pair_with;
use maka_client::{ClientError, RequestFailure};
use maka_protocol::configuration::{
    headers::*,
    policy::{
        RuntimePolicy,
        network_update::{self, CredentialUpdate, Update},
    },
};
use serde_json::json;
use std::time::Duration;
const ID: &str = "5dbbcb6f-8279-4fba-81ea-763a58230cc4";
const CREDENTIAL: &str = "0c1bfba8-0cd8-42bf-adc6-3160eb1e3ae2";

#[tokio::test]
async fn preferences197_proxy_outputs_bind_revision_and_secret_action() {
    let locator = network_update::locator();
    let absent = json!({"locator":locator,"configured":false,"credentialId":null,"revision":null,"updatedAt":null});
    let configured = json!({"locator":locator,"configured":true,"credentialId":CREDENTIAL,"revision":1,"updatedAt":10});
    for (revision, status, valid) in [
        (7, absent.clone(), true),
        (8, absent.clone(), true),
        (9, absent.clone(), false),
        (8, configured, false),
    ] {
        let (client, _notices, mut reader, mut writer) = pair_with(maka_client::Operations).await;
        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .update_network_proxy(Update {
                        expected_policy_revision: 7,
                        expected_credential: None,
                        network_proxy: RuntimePolicy::default().network_proxy,
                        credential: CredentialUpdate::Delete {},
                    })
                    .await
            }
        });
        let frame = tokio::time::timeout(Duration::from_secs(1), reader.read())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(frame["operation"], "runtime.policy.network-proxy.update");
        assert_eq!(frame["input"]["expectedPolicyRevision"], 7);
        writer.write(&json!({"requestId":frame["requestId"],"operation":frame["operation"],"ok":true,"result":{"kind":"committed","revision":revision,"credentialStatus":status}})).await.unwrap();
        let result = request.await.unwrap();
        assert_eq!(result.is_ok(), valid);
        if !valid {
            assert!(matches!(
                result,
                Err(RequestFailure::Unknown(ClientError::Protocol(_)))
            ));
        }
        client.disconnect();
    }
}

#[tokio::test]
async fn preferences197_header_outputs_bind_names_connection_and_credential_cas() {
    let connection = json!({"connectionId":ID,"revision":4});
    let expected = json!({"connection":connection,"credential":null});
    let configured = json!({"locator":{"scope":"connection","connectionId":ID,"kind":"request_headers"},"credentialId":CREDENTIAL,"revision":1});
    for (basis, names, valid) in [
        (
            json!({"connection":connection,"credential":configured}),
            json!(["X-Test"]),
            true,
        ),
        (
            json!({"connection":{"connectionId":ID,"revision":5},"credential":configured}),
            json!(["X-Test"]),
            true,
        ),
        (
            json!({"connection":{"connectionId":ID,"revision":6},"credential":configured}),
            json!(["X-Test"]),
            false,
        ),
        (
            json!({"connection":connection,"credential":configured}),
            json!(["X-Other"]),
            false,
        ),
        (
            json!({"connection":connection,"credential":null}),
            json!(["X-Test"]),
            false,
        ),
    ] {
        let (client, _notices, mut reader, mut writer) = pair_with(maka_client::Operations).await;
        let input: RequestHeadersReplace = serde_json::from_value(
            json!({"expected":expected,"headers":[{"name":"X-Test","value":"private-test-value"}]}),
        )
        .unwrap();
        let request = tokio::spawn({
            let client = client.clone();
            async move { client.replace_request_headers(&input).await }
        });
        let frame = tokio::time::timeout(Duration::from_secs(1), reader.read())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(frame["operation"], "connection.request-headers.replace");
        assert_eq!(frame["input"]["expected"], expected);
        writer.write(&json!({"requestId":frame["requestId"],"operation":frame["operation"],"ok":true,"result":{"kind":"committed","names":names,"basis":basis}})).await.unwrap();
        let result = request.await.unwrap();
        assert_eq!(result.is_ok(), valid);
        if !valid {
            assert!(matches!(
                result,
                Err(RequestFailure::Unknown(ClientError::Protocol(_)))
            ));
        }
        client.disconnect();
    }
}

#[tokio::test]
async fn preferences197_query_and_conflict_echoes_cannot_rebase_another_connection() {
    for (query, result) in [
        (
            true,
            json!({"kind":"found","names":[],"basis":{"connection":{"connectionId":CREDENTIAL,"revision":4},"credential":null}}),
        ),
        (
            false,
            json!({"kind":"connection_stale","expected":{"connectionId":CREDENTIAL,"revision":4},"actual":{"connectionId":CREDENTIAL,"revision":5}}),
        ),
        (
            false,
            json!({"kind":"credential_stale","expected":null,"actual":{"locator":{"scope":"connection","connectionId":CREDENTIAL,"kind":"request_headers"},"credentialId":CREDENTIAL,"revision":1}}),
        ),
    ] {
        let (client, _notices, mut reader, mut writer) = pair_with(maka_client::Operations).await;
        let request = tokio::spawn({
            let client = client.clone();
            async move {
                if query {
                    client.request_headers(ID).await.map(|_| ())
                } else {
                    let input = serde_json::from_value(json!({"expected":{"connection":{"connectionId":ID,"revision":4},"credential":null},"headers":[]})).unwrap();
                    client.replace_request_headers(&input).await.map(|_| ())
                }
            }
        });
        let frame = tokio::time::timeout(Duration::from_secs(1), reader.read())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        writer.write(&json!({"requestId":frame["requestId"],"operation":frame["operation"],"ok":true,"result":result})).await.unwrap();
        assert!(matches!(
            request.await.unwrap(),
            Err(RequestFailure::Unknown(ClientError::Protocol(_)))
        ));
        client.disconnect();
    }
}
