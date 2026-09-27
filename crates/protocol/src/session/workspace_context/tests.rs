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

use super::*;
use serde_json::json;
fn basis() -> Value {
    json!({"rootId":"root","sessionId":"session","boundaryRevision":0,
    "workspace":{"target":{"kind":"host_path","path":"/work"},"hostCwd":"/work"},
    "directoryIdentity":format!("sha256:{}", "a".repeat(64))})
}

#[test]
fn workspace_requests_require_relative_paths_and_the_exact_boundary() {
    for path in [
        "../escape",
        "/absolute",
        "a/../escape",
        "a\\escape",
        "a\u{0}/b",
    ] {
        assert!(
            decode_query(
                &json!({"sessionId":"session","directory":path,"filter":"","cursor":null})
            )
            .is_err()
        );
        assert!(decode_capture(&json!({"basis":basis(),"path":path,"kind":"file"})).is_err());
    }
    decode_query(&json!({"sessionId":"session","directory":"src","filter":"中文","cursor":null}))
        .unwrap();
    let mut altered = basis();
    altered["boundaryRevision"] = json!(9_007_199_254_740_992_u64);
    assert!(decode_capture(&json!({"basis":altered,"path":"file","kind":"file"})).is_err());
}

#[test]
fn captured_context_is_content_checked_and_cannot_claim_another_directory() {
    let text = "Workspace directory src:\nsrc/actual.rs\n";
    let mut output = json!({"basis":basis(),"path":"src","kind":"directory",
        "quote":{"text":text,"label":"src"},"contentDigest":maka_runtime::artifact::content_digest(text.as_bytes()),
        "truncated":false,"directoryReference":{"hostId":"root","path":"/work/src"}});
    decode_captured(&output).unwrap();
    output["directoryReference"]["path"] = json!("/outside");
    assert!(decode_captured(&output).is_err());
    output["directoryReference"]["path"] = json!("/work/src");
    output["quote"]["text"] = json!("cosmetic-only label");
    assert!(decode_captured(&output).is_err());
}
