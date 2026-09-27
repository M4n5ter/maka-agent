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
use maka_protocol::plugin::{PackageExport, PackagePrecondition};
use serde_json::json;

#[tokio::test]
async fn package_export_sends_reviewed_precondition_and_rejects_foreign_destination() {
    let (client, _notices, mut reader, mut writer) = pair_with(maka_client::Operations).await;
    let input = PackageExport {
        extension_id: "example.board".into(),
        target_path: "/host/board.maka-extension".into(),
        expected: PackagePrecondition {
            base_generation: 7,
            content_digest: Some(format!("sha256-{}", "a".repeat(64))),
        },
    };
    let request = tokio::spawn({
        let client = client.clone();
        let input = input.clone();
        async move { client.plugin_package_export(input).await }
    });
    let frame = reader.read().await.unwrap().unwrap();
    assert_eq!(frame["input"], serde_json::to_value(&input).unwrap());
    writer
        .write(
            &json!({"requestId":frame["requestId"],"operation":frame["operation"],"ok":true,
        "result":{"targetPath":"/host/other"}}),
        )
        .await
        .unwrap();
    assert!(matches!(
        request.await.unwrap(),
        Err(RequestFailure::Unknown(ClientError::Protocol(_)))
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), client.closed())
        .await
        .unwrap();
}
