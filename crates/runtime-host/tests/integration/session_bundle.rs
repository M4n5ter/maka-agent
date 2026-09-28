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

use super::support::host_fixture::HostFixture;
mod fixtures;
mod host;
mod recovery;
use fixtures::{configure, seed};
use host::Running;
use maka_client::Notification;
use maka_protocol::{Operation, OperationErrorCode, session::*};
use maka_runtime_host::session::SessionConfiguration;
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::test]
async fn native_bundle_transfer_binds_preview_and_recovers_original_receipt_after_restart() {
    let source = HostFixture::new("maka-bundle-source-");
    let destination = HostFixture::new("maka-bundle-destination-");
    let log = source.log().await;
    seed(&log, &source).await;
    fixtures::interrupt(&log).await;
    let inventory = log.preview_bundle("source").await.unwrap();
    log.close().await.unwrap();
    let path = source.workspace.join("history.maka-session");
    let source_host = Running::open(&source).await;
    let preview = source_host
        .client
        .request(
            Operation::SessionBundlePreview,
            json!({"sessionId":"source"}),
        )
        .await
        .unwrap();
    assert_eq!(
        preview,
        json!({"sessionCount":3,"subtreeDigest":inventory.subtree_digest})
    );
    let export = json!({"sessionId":"source","destination":path,"expectedSubtreeDigest":inventory.subtree_digest});
    let result = source_host
        .client
        .request(Operation::SessionBundleExport, export.clone())
        .await
        .unwrap();
    assert_eq!(result["sessionCount"], 3);
    let bytes = tokio::fs::read(&path).await.unwrap();
    assert_eq!(result["compressedBytes"], bytes.len());
    assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
    rejected(
        source_host
            .client
            .request(Operation::SessionBundleExport, export)
            .await,
        OperationErrorCode::OperationConflict,
    );
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    let alternate_path = source.workspace.join("alternate.maka-session");
    source_host
        .client
        .request(
            Operation::SessionBundleExport,
            json!({"sessionId":"child","destination":alternate_path}),
        )
        .await
        .unwrap();
    let alternate_bytes = tokio::fs::read(&alternate_path).await.unwrap();
    source_host.close().await;
    let recovered = source.log().await;
    fixtures::assert_interrupted(&recovered).await;
    recovered.close().await.unwrap();

    let mut target = Running::open(&destination).await;
    configure(&target.client).await;
    let mut request =
        json!({"source":path,"workspace":{"kind":"host_path","path":destination.workspace}});
    let expected = target
        .client
        .request(Operation::SessionBundleImportPreview, request.clone())
        .await
        .unwrap();
    assert_eq!(expected["sessionCount"], 3);
    assert_eq!(expected["artifactFiles"], 0);
    let query =
        json!({"bundleDigest":expected["bundleDigest"],"bindingDigest":expected["bindingDigest"]});
    assert_eq!(
        target
            .client
            .request(Operation::SessionBundleImportQuery, query.clone())
            .await
            .unwrap(),
        json!({"receipt":null})
    );
    request["expected"] = expected.clone();
    let invalid = source.workspace.join("truncated.maka-session");
    tokio::fs::write(&invalid, &bytes[..bytes.len() / 2])
        .await
        .unwrap();
    rejected(
        target
            .client
            .request(
                Operation::SessionBundleImportPreview,
                json!({
                    "source":invalid,"workspace":request["workspace"]
                }),
            )
            .await,
        OperationErrorCode::SourceUnreadable,
    );
    // The same path now contains another valid archive. It must not turn the
    // user's confirmation into permission to import these different bytes.
    tokio::fs::write(&path, &alternate_bytes).await.unwrap();
    rejected(
        target
            .client
            .request(Operation::SessionBundleImport, request.clone())
            .await,
        OperationErrorCode::CandidateSetStale,
    );
    tokio::fs::write(&path, &bytes).await.unwrap();
    for field in [
        "bundleDigest",
        "bindingDigest",
        "sessionCount",
        "artifactFiles",
    ] {
        let mut changed = request.clone();
        changed["expected"][field] = if field.ends_with("Digest") {
            json!(format!("sha256:{}", "f".repeat(64)))
        } else {
            json!(expected[field].as_u64().unwrap() + 1)
        };
        rejected(
            target
                .client
                .request(Operation::SessionBundleImport, changed)
                .await,
            OperationErrorCode::CandidateSetStale,
        );
    }
    let mut changed_workspace = request.clone();
    changed_workspace["workspace"]["path"] = json!(source.workspace);
    rejected(
        target
            .client
            .request(Operation::SessionBundleImport, changed_workspace)
            .await,
        OperationErrorCode::CandidateSetStale,
    );
    let before = target
        .client
        .session_catalog(SessionCatalogQueryInput::ListStart)
        .await
        .unwrap();
    assert!(
        matches!(before, SessionCatalogQueryResult::Page { sessions, .. } if sessions.is_empty())
    );
    let (first, concurrent) = tokio::join!(
        target
            .client
            .request(Operation::SessionBundleImport, request.clone()),
        target
            .client
            .request(Operation::SessionBundleImport, request.clone()),
    );
    let imported = first.unwrap();
    assert_eq!(concurrent.unwrap(), imported, "concurrent exact retry");
    assert_eq!(
        imported,
        json!({"sessionCount":3,"artifactFiles":0,
        "rootSessionId":"source","sessionIds":["child","managed","source"]})
    );
    let receipt =
        json!({"receipt":{"rootSessionId":"source","sessionIds":["child","managed","source"]}});
    assert_eq!(
        target
            .client
            .request(Operation::SessionBundleImportQuery, query.clone())
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(
        target
            .client
            .request(
                Operation::SessionBundlePreview,
                json!({"sessionId":"source"})
            )
            .await
            .unwrap(),
        preview
    );
    loop {
        let notice = tokio::time::timeout(Duration::from_secs(5), target.notices.recv())
            .await
            .unwrap()
            .unwrap();
        if let Notification::Catalog(notice) = notice
            && notice.kind == "session.catalog.changed"
        {
            assert!(
                notice.session_id.is_none(),
                "one whole-catalog invalidation"
            );
            break;
        }
    }
    // An explicit duplicate write remains content-bound, including its original
    // destination binding even if defaults disappear. Recovery itself is read-only.
    target
        .client
        .request(
            Operation::ConnectionCatalogSetDefaultTarget,
            json!({"expectedCatalogRevision":2,"target":null}),
        )
        .await
        .unwrap();
    assert_eq!(
        target
            .client
            .request(Operation::SessionBundleImport, request.clone())
            .await
            .unwrap(),
        imported
    );
    let other_preview = target
        .client
        .request(
            Operation::SessionBundleImportPreview,
            json!({"source":path,"workspace":{"kind":"host_path","path":source.workspace}}),
        )
        .await
        .unwrap();
    rejected(
        target
            .client
            .request(
                Operation::SessionBundleImport,
                json!({
                    "source":path,"workspace":{"kind":"host_path","path":source.workspace},"expected":other_preview
                }),
            )
            .await,
        OperationErrorCode::OperationConflict,
    );
    rejected(target.client.request(Operation::SessionBundleImportQuery,
        json!({"bundleDigest":query["bundleDigest"],"bindingDigest":other_preview["bindingDigest"]})).await,
        OperationErrorCode::OperationConflict);
    tokio::fs::remove_file(&path).await.unwrap();
    assert_eq!(
        target
            .client
            .request(Operation::SessionBundleImportQuery, query.clone())
            .await
            .unwrap(),
        receipt
    );
    target.close().await;

    let log = destination.log().await;
    fixtures::assert_interrupted(&log).await;
    assert_eq!(
        log.preview_bundle("source").await.unwrap().sessions.len(),
        3
    );
    let config = log
        .get_session::<SessionConfiguration>("source")
        .await
        .unwrap()
        .unwrap()
        .configuration;
    assert_eq!(config.name, "Transferred title");
    assert_ne!(config.sandbox_mode, SandboxMode::DangerFullAccess);
    assert_eq!(config.approval_policy, ApprovalPolicy::OnRequest);
    assert!(config.instructions.is_none() && config.bound_tools.is_none());
    assert_eq!(config.target.model().unwrap().model, "fixture-model");
    assert!(log.session_manager("child").await.unwrap().is_none());
    assert!(log.session_manager("managed").await.unwrap().is_none());
    assert!(log.unfinished_invocations(10).await.unwrap().is_empty());
    log.read_model_context("source", None, 100, 65536)
        .await
        .unwrap();
    log.close().await.unwrap();
    let target = Running::open(&destination).await;
    assert_eq!(
        target
            .client
            .request(Operation::SessionBundleImportQuery, query)
            .await
            .unwrap(),
        receipt
    );
    target.close().await;
    let names: Vec<_> = std::fs::read_dir(&source.workspace)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "no abandoned publication temporary files");
}

fn rejected(result: Result<Value, maka_client::RequestFailure>, code: OperationErrorCode) {
    assert!(
        matches!(&result, Err(maka_client::RequestFailure::Rejected(maka_client::ClientError::Rejected(error))) if error.code == code),
        "{result:?}"
    );
}
