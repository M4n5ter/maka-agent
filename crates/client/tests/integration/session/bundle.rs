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

use super::super::connection::pair_with;
use maka_client::{ClientError, RequestFailure};
use maka_protocol::session::{WorkspaceTarget, bundle};
use serde_json::json;

fn preview() -> bundle::ImportPreviewed {
    bundle::ImportPreviewed {
        bundle_digest: format!("sha256:{}", "a".repeat(64)),
        binding_digest: format!("sha256:{}", "b".repeat(64)),
        session_count: 2,
        artifact_files: 7,
        resolved_workspace: maka_protocol::session::WorkspaceProjection {
            target: WorkspaceTarget::HostPath {
                path: "/host/workspace".into(),
            },
            host_cwd: "/host/workspace".into(),
        },
    }
}

#[tokio::test]
async fn bundle_import_freezes_preview_and_queries_receipt_without_reopening_source() {
    let (client, _notices, mut reader, mut writer) = pair_with(maka_client::Operations).await;
    let input = bundle::Import {
        source: "/host/original.maka-session".into(),
        workspace: WorkspaceTarget::HostPath {
            path: "/host/workspace".into(),
        },
        expected: preview(),
    };
    let importing = tokio::spawn({
        let client = client.clone();
        let input = input.clone();
        async move { client.import_session_bundle(input).await }
    });
    let request = reader.read().await.unwrap().unwrap();
    assert_eq!(request["operation"], "session-bundle.import");
    assert_eq!(request["input"], serde_json::to_value(input).unwrap());
    writer.write(&json!({"requestId":request["requestId"],"operation":request["operation"],"ok":true,"result":{
        "sessionCount":2,"artifactFiles":7,"rootSessionId":"root-session","sessionIds":["child","root-session"]
    }})).await.unwrap();
    let receipt = importing.await.unwrap().unwrap();
    assert_eq!(receipt.root_session_id, "root-session");
    let reading = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .query_session_bundle_import(bundle::ImportQuery {
                    bundle_digest: preview().bundle_digest,
                    binding_digest: preview().binding_digest,
                })
                .await
        }
    });
    let request = reader.read().await.unwrap().unwrap();
    assert_eq!(request["operation"], "session-bundle.import.query");
    assert_eq!(
        request["input"],
        json!({"bundleDigest":preview().bundle_digest,"bindingDigest":preview().binding_digest})
    );
    writer.write(&json!({"requestId":request["requestId"],"operation":request["operation"],"ok":true,"result":{"receipt":null}})).await.unwrap();
    assert!(reading.await.unwrap().unwrap().receipt.is_none());
    client.disconnect();
}

#[tokio::test]
async fn bundle_client_rejects_malformed_previews_and_receipts_instead_of_claiming_success() {
    for case in 0..4 {
        let (client, _notices, mut reader, mut writer) = pair_with(maka_client::Operations).await;
        let pending = tokio::spawn({
            let client = client.clone();
            async move {
                if case == 0 {
                    client
                        .preview_session_bundle(bundle::Preview {
                            session_id: "session".into(),
                        })
                        .await
                        .map(|_| ())
                } else {
                    client
                        .import_session_bundle(bundle::Import {
                            source: "/host/source".into(),
                            workspace: WorkspaceTarget::HostPath {
                                path: "/host/workspace".into(),
                            },
                            expected: preview(),
                        })
                        .await
                        .map(|_| ())
                }
            }
        });
        let request = reader.read().await.unwrap().unwrap();
        let result = match case {
            0 => json!({"sessionCount":1,"subtreeDigest":"invalid"}),
            1 => {
                json!({"sessionCount":1,"artifactFiles":7,"rootSessionId":"root","sessionIds":["root"]})
            }
            2 => {
                json!({"sessionCount":2,"artifactFiles":8,"rootSessionId":"root","sessionIds":["child","root"]})
            }
            _ => {
                json!({"sessionCount":2,"artifactFiles":7,"rootSessionId":"root","sessionIds":["root","root"]})
            }
        };
        writer.write(&json!({"requestId":request["requestId"],"operation":request["operation"],"ok":true,"result":result})).await.unwrap();
        assert!(matches!(
            pending.await.unwrap(),
            Err(RequestFailure::Unknown(ClientError::Protocol(_)))
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), client.closed())
            .await
            .unwrap();
    }
}
