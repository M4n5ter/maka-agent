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

fn preview() -> Value {
    json!({
        "bundleDigest":format!("sha256:{}", "a".repeat(64)),
        "bindingDigest":format!("sha256:{}", "b".repeat(64)),
        "sessionCount":2,"artifactFiles":0,
        "resolvedWorkspace":{"target":{"kind":"host_path","path":"/workspace"},"hostCwd":"/workspace"}
    })
}

#[test]
fn import_requires_the_complete_content_and_workspace_preview() {
    let mut input = json!({"source":"/tmp/history.maka-session",
        "workspace":{"kind":"host_path","path":"/workspace"}});
    decode_import_preview(&input).unwrap();
    assert!(decode_import(&input).is_err());
    input["expected"] = preview();
    decode_import(&input).unwrap();
    for field in [
        "bundleDigest",
        "bindingDigest",
        "sessionCount",
        "artifactFiles",
        "resolvedWorkspace",
    ] {
        let mut missing = input.clone();
        missing["expected"].as_object_mut().unwrap().remove(field);
        assert!(decode_import(&missing).is_err(), "{field}");
    }
    for invalid in [
        json!("a".repeat(64)),
        json!(format!("sha256:{}", "A".repeat(64))),
        Value::Null,
    ] {
        let mut changed = input.clone();
        changed["expected"]["bundleDigest"] = invalid;
        assert!(decode_import(&changed).is_err());
    }
    input["expected"]["sessionCount"] = json!(0);
    assert!(decode_import(&input).is_err());
}

#[test]
fn receipt_query_has_no_path_or_mutation_input_and_preserves_exact_ids() {
    let expected = preview();
    let mut query =
        json!({"bundleDigest":expected["bundleDigest"],"bindingDigest":expected["bindingDigest"]});
    decode_import_query(&query).unwrap();
    query["source"] = json!("/tmp/current.maka-session");
    assert!(decode_import_query(&query).is_err());
    assert_eq!(
        decode_import_queried(&json!({"receipt":null}))
            .unwrap()
            .receipt,
        None
    );
    assert!(decode_import_queried(&json!({})).is_err());
    let receipt = json!({"rootSessionId":"root","sessionIds":["child","root"]});
    let result = decode_import_queried(&json!({"receipt":receipt}))
        .unwrap()
        .receipt
        .unwrap();
    assert_eq!(result.root_session_id, "root");
    assert_eq!(result.session_ids, ["child", "root"]);
    for ids in [
        json!([]),
        json!(["child"]),
        json!(["root", "root"]),
        json!(["root", "bad/id"]),
    ] {
        assert!(
            decode_import_queried(&json!({"receipt":{"rootSessionId":"root","sessionIds":ids}}))
                .is_err()
        );
    }
    let mut imported = receipt;
    imported["sessionCount"] = json!(2);
    imported["artifactFiles"] = json!(0);
    decode_imported(&imported).unwrap();
    imported["sessionCount"] = json!(3);
    assert!(decode_imported(&imported).is_err());
}
