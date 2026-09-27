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

#[tokio::test]
async fn public_app_preserves_and_edits_a_long_valid_executable_path() {
    let mut original = agent();
    original.executable = std::env::temp_dir()
        .join("x".repeat(32_000))
        .to_string_lossy()
        .into_owned();
    let fixture = Fixture::new(original.clone());
    let route = json!({"agent":"fixture"});
    let shown = fixture.read(route.clone()).await;
    assert!(!shown.field("executable").unwrap().enabled);
    let fields = [
        ("name".into(), json!("Renamed")),
        ("args".into(), json!("")),
        ("env".into(), json!("")),
    ]
    .into();
    fixture
        .submit(&shown, route.clone(), "save", fields)
        .await
        .unwrap();
    assert_eq!(fixture.stored().executable, original.executable);
    let shown = fixture.read(route).await;
    let path_route = destination(&shown, "executable-fragments");
    let path = fixture.read(path_route.clone()).await;
    assert!(path.action("launch-item-remove").is_none());
    let replacement = std::env::temp_dir()
        .join("changed")
        .to_string_lossy()
        .into_owned();
    fixture
        .submit(
            &path,
            path_route,
            "launch-item-save",
            [("fragment".into(), json!(document(&replacement)))].into(),
        )
        .await
        .unwrap();
    assert_eq!(
        fixture.stored().executable,
        format!("{replacement}{}", &original.executable[CHUNK..])
    );
    assert!(fixture.stored().args.is_empty() && fixture.stored().env.is_empty());
}
