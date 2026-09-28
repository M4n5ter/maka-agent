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

async fn archive(source: &HostFixture) -> std::path::PathBuf {
    let log = source.log().await;
    seed(&log, source).await;
    let expected = log.preview_bundle("source").await.unwrap().subtree_digest;
    log.close().await.unwrap();
    let host = Running::open(source).await;
    let path = source.workspace.join("reviewed.maka-session");
    host.client
        .request(
            Operation::SessionBundleExport,
            json!({"sessionId":"source","destination":path,"expectedSubtreeDigest":expected}),
        )
        .await
        .unwrap();
    host.close().await;
    path
}

async fn empty(host: &Running) {
    assert!(
        matches!(host.client.session_catalog(SessionCatalogQueryInput::ListStart).await.unwrap(),
        SessionCatalogQueryResult::Page { sessions, .. } if sessions.is_empty())
    );
}

#[tokio::test]
async fn import_rejects_a_project_relinked_after_the_reviewed_destination() {
    let source = HostFixture::new("maka-bundle-review-source-");
    let target = HostFixture::new("maka-bundle-review-target-");
    let path = archive(&source).await;
    let destination = Running::open(&target).await;
    configure(&destination.client).await;
    let first = target.workspace.join("first");
    let second = target.workspace.join("second");
    tokio::fs::create_dir(&first).await.unwrap();
    tokio::fs::create_dir(&second).await.unwrap();
    let project = destination
        .client
        .request(
            Operation::ProjectCatalogMutate,
            json!({"kind":"register","path":first}),
        )
        .await
        .unwrap();
    let workspace = json!({"kind":"project","projectId":project["project"]["id"]});
    let preview_input = json!({"source":path,"workspace":workspace});
    let expected = destination
        .client
        .request(Operation::SessionBundleImportPreview, preview_input.clone())
        .await
        .unwrap();
    let canonical = |path: &std::path::Path| {
        maka_fs_tools::workspace::project::host_path(&path.canonicalize().unwrap())
            .unwrap()
            .to_owned()
    };
    assert_eq!(expected["resolvedWorkspace"]["hostCwd"], canonical(&first));
    destination
        .client
        .request(
            Operation::ProjectCatalogMutate,
            json!({"kind":"relink","projectId":project["project"]["id"],"path":second}),
        )
        .await
        .unwrap();
    let mut request = preview_input.clone();
    request["expected"] = expected.clone();
    rejected(
        destination
            .client
            .request(Operation::SessionBundleImport, request.clone())
            .await,
        OperationErrorCode::CandidateSetStale,
    );
    empty(&destination).await;
    let current = destination
        .client
        .request(Operation::SessionBundleImportPreview, preview_input)
        .await
        .unwrap();
    assert_eq!(current["resolvedWorkspace"]["hostCwd"], canonical(&second));
    assert_eq!(
        current["bindingDigest"], expected["bindingDigest"],
        "receipt locator remains stable"
    );
    request["expected"] = current;
    destination
        .client
        .request(Operation::SessionBundleImport, request)
        .await
        .unwrap();
    destination.close().await;
    let log = target.log().await;
    for session in ["source", "child", "managed"] {
        let saved = log
            .get_session::<SessionConfiguration>(session)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.configuration.workspace.host_cwd, canonical(&second));
    }
    log.close().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn import_rejects_a_host_path_alias_retargeted_after_preview() {
    let source = HostFixture::new("maka-bundle-alias-source-");
    let target = HostFixture::new("maka-bundle-alias-target-");
    let path = archive(&source).await;
    let first = target.workspace.join("first");
    let second = target.workspace.join("second");
    let alias = target.workspace.join("selected");
    tokio::fs::create_dir(&first).await.unwrap();
    tokio::fs::create_dir(&second).await.unwrap();
    std::os::unix::fs::symlink(&first, &alias).unwrap();
    let destination = Running::open(&target).await;
    configure(&destination.client).await;
    let mut input = json!({"source":path,"workspace":{"kind":"host_path","path":alias}});
    input["expected"] = destination
        .client
        .request(Operation::SessionBundleImportPreview, input.clone())
        .await
        .unwrap();
    tokio::fs::remove_file(&alias).await.unwrap();
    std::os::unix::fs::symlink(&second, &alias).unwrap();
    rejected(
        destination
            .client
            .request(Operation::SessionBundleImport, input)
            .await,
        OperationErrorCode::CandidateSetStale,
    );
    empty(&destination).await;
    destination.close().await;
}
