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
use maka_protocol::session::workspace_context as api;
use serde_json::json;

#[tokio::test]
async fn workspace_client_rejects_cross_session_scope_and_nonadvancing_pages() {
    for case in 0..3 {
        let (client, _notices, mut reader, mut writer) = pair_with(maka_client::Operations).await;
        let basis: api::Basis = serde_json::from_value(
            json!({"rootId":client.identity.root_id,"sessionId":"session","boundaryRevision":1,
            "workspace":{"target":{"kind":"host_path","path":"/work"},"hostCwd":"/work"},
            "directoryIdentity":format!("sha256:{}", "a".repeat(64))}),
        )
        .unwrap();
        let revision = format!("sha256:{}", "b".repeat(64));
        let input = api::Query {
            session_id: "session".into(),
            directory: "src".into(),
            filter: String::new(),
            cursor: Some(api::Cursor {
                basis: basis.clone(),
                revision: revision.clone(),
                after: "src/a".into(),
            }),
        };
        let reading = tokio::spawn({
            let client = client.clone();
            async move { client.query_session_workspace(input).await }
        });
        let request = reader.read().await.unwrap().unwrap();
        assert_eq!(request["operation"], "session.workspace.query");
        let mut output = json!({"basis":basis,"directory":"src","filter":"","revision":revision,
            "entries":[{"path":"src/b","kind":"file"}],"nextCursor":null});
        match case {
            0 => output["basis"]["sessionId"] = json!("another"),
            1 => output["entries"][0]["path"] = json!("src/a"),
            _ => output["directory"] = json!("elsewhere"),
        }
        writer.write(&json!({"requestId":request["requestId"],"operation":request["operation"],"ok":true,"result":output})).await.unwrap();
        assert!(matches!(
            reading.await.unwrap(),
            Err(RequestFailure::Unknown(ClientError::Protocol(_)))
        ));
        client.disconnect();
    }
}
