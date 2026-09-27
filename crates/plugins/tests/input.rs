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

use futures_util::{future::BoxFuture, poll};
use maka_plugins::{
    composition::Scope,
    contributions::{Catalog, Staged},
    fiber::Fiber,
    input::*,
    revision::Revision,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

struct Example(Revision);
impl Provider for Example {
    fn prepare(
        &self,
        mut request: Request,
        _workspace: maka_plugins::filesystem::ReadDirectory,
    ) -> BoxFuture<'static, Result<Outcome, maka_plugins::Error>> {
        let revision = self.0.clone();
        Box::pin(async move {
            let basis = revision.capture().await;
            if request.content.text == "replace attachment" {
                request.content.attachments = Some(Vec::new());
            }
            request.content.text.push_str("\nprepared business input");
            Ok(Outcome::Ready {
                content: request.content,
                receipt: json!({"ticket":42}),
                required_tools: Default::default(),
                basis: Some(basis),
            })
        })
    }
}

#[tokio::test]
async fn admission_orders_domain_invalidation_and_retirement_without_executing_callbacks() {
    let catalog = Catalog::default();
    let owner = Fiber::new("example", "example", Scope::Profile).unwrap();
    owner.begin_loading().unwrap();
    owner.ready().unwrap();
    let revision = Revision::default();
    let mut staged = Staged::default();
    staged
        .insert(
            "example.prepare",
            InputPreparation::new(Arc::new(Example(revision.clone()))),
        )
        .unwrap();
    catalog.publish(&owner, staged).unwrap();
    let request = Request {
        session_id: "session".into(),
        cwd: ".".into(),
        content: "original".into(),
        selections: Default::default(),
        selection_sources: Default::default(),
        tools: Default::default(),
        cancellation: Default::default(),
    };
    let workspace = maka_plugins::filesystem::ReadRoot::capture(".").unwrap();
    assert!(
        prepare(
            &catalog,
            &Scope::Session("session".into()),
            Request {
                content: "replace attachment".into(),
                ..request.clone()
            },
            &workspace,
        )
        .await
        .is_err(),
        "native preparation cannot rewrite user attachments"
    );
    let first = prepare(
        &catalog,
        &Scope::Session("session".into()),
        request.clone(),
        &workspace,
    )
    .await
    .unwrap();
    assert_eq!(first.content.preparation[0].source.package_id, "example");
    let accepted = serde_json::to_vec(&first.content).unwrap();
    let admission = first.admit().unwrap().unwrap();
    let invalidation = revision.invalidate();
    tokio::pin!(invalidation);
    assert!(
        poll!(&mut invalidation).is_pending(),
        "a domain writer waits for durable input admission"
    );
    drop(admission);
    let writer = invalidation.await;
    assert!(
        first.admit().unwrap().is_none(),
        "commit never waits for an in-progress domain writer"
    );
    drop(writer);
    assert!(
        first.admit().unwrap().is_none(),
        "stale preparation must be repeated"
    );
    let fresh = prepare(
        &catalog,
        &Scope::Session("session".into()),
        request,
        &workspace,
    )
    .await
    .unwrap();
    let admitted = fresh.admit().unwrap().unwrap();
    let stopping = owner.shutdown(tokio::time::Instant::now() + Duration::from_secs(1));
    tokio::pin!(stopping);
    assert!(poll!(&mut stopping).is_pending());
    assert!(fresh.admit().is_err(), "retirement fences a new start");
    assert_eq!(
        accepted,
        serde_json::to_vec(&first.content).unwrap(),
        "accepted evidence survives lifecycle changes"
    );
    drop(admitted);
    stopping.await.unwrap();
}

#[tokio::test]
async fn many_input_providers_preserve_preparation_receipts_and_real_resource_bounds() {
    let catalog = Catalog::default();
    let owner = Fiber::new("example", "example", Scope::Profile).unwrap();
    owner.begin_loading().unwrap();
    owner.ready().unwrap();
    let mut staged = Staged::default();
    for index in 0..96 {
        staged
            .insert(
                format!("example.prepare-{index:03}"),
                InputPreparation::new(Arc::new(Example(Revision::default()))),
            )
            .unwrap();
    }
    catalog.publish(&owner, staged).unwrap();
    let request = Request {
        session_id: "session".into(),
        cwd: ".".into(),
        content: "original".into(),
        selections: Default::default(),
        selection_sources: Default::default(),
        tools: Default::default(),
        cancellation: Default::default(),
    };
    let workspace = maka_plugins::filesystem::ReadRoot::capture(".").unwrap();
    let scope = Scope::Session("session".into());
    let prepared = prepare(&catalog, &scope, request.clone(), &workspace)
        .await
        .unwrap();
    assert_eq!(prepared.content.preparation.len(), 96);
    assert_eq!(
        prepared.content.text,
        format!("original{}", "\nprepared business input".repeat(96))
    );
    for (index, receipt) in prepared.content.preparation.iter().enumerate() {
        assert_eq!(receipt.source.name, format!("example.prepare-{index:03}"));
        assert_eq!(receipt.source.package_id, "example");
        assert_eq!(receipt.receipt, json!({"ticket":42}));
    }
    maka_runtime::input::validate_receipts(&prepared.content.preparation).unwrap();
    drop(prepared.admit().unwrap().unwrap());
    let oversized = prepare(
        &catalog,
        &scope,
        Request {
            content: "x".repeat(64 * 1024).into(),
            ..request.clone()
        },
        &workspace,
    )
    .await;
    assert!(
        matches!(oversized, Err(maka_plugins::Error::Invalid(message)) if message == "prepared input exceeds 64 KiB")
    );
    request.cancellation.cancel();
    let cancelled = prepare(&catalog, &scope, request, &workspace).await;
    assert!(
        matches!(cancelled, Err(maka_plugins::Error::Invalid(message)) if message == "input preparation cancelled")
    );
    owner
        .shutdown(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert!(prepared.admit().is_err(), "all captured sources retire");
}
