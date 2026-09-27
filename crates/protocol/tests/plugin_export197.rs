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

use maka_protocol::{Operation, plugin};
use serde_json::json;

#[test]
fn package_export_requires_exact_installed_precondition_and_absolute_destination() {
    let valid = json!({"extensionId":"example.board","targetPath":"/host/package.maka-extension",
        "expected":{"baseGeneration":7,"contentDigest":format!("sha256-{}", "a".repeat(64))}});
    assert!(plugin::decode_input(Operation::PluginPackageExport, &valid).is_ok());
    for (field, value) in [
        ("expected", json!(null)),
        ("expected", json!({"baseGeneration":7,"contentDigest":null})),
        (
            "expected",
            json!({"baseGeneration":7,"contentDigest":"invalid"}),
        ),
        ("targetPath", json!("relative/path")),
        ("targetPath", json!("/host/path\nother")),
    ] {
        let mut bad = valid.clone();
        bad[field] = value;
        assert!(plugin::decode_input(Operation::PluginPackageExport, &bad).is_err());
    }
    let mut missing = valid;
    missing.as_object_mut().unwrap().remove("expected");
    assert!(plugin::decode_input(Operation::PluginPackageExport, &missing).is_err());
}
