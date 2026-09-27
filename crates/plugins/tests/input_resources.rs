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

use futures_util::future::BoxFuture;
use maka_plugins::{
    Error,
    composition::Scope,
    contributions::{Catalog, Staged},
    fiber::Fiber,
    input::{self, InputPreparation, Outcome, Request, resources},
    terminal_ui::Text,
};
use maka_runtime::input::SelectionSource;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Business {
    calls: Arc<AtomicUsize>,
    unchanged: bool,
}
impl input::Provider for Business {
    fn prepare(
        &self,
        mut request: Request,
        _: maka_plugins::filesystem::ReadDirectory,
    ) -> BoxFuture<'static, Result<Outcome, Error>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let unchanged = self.unchanged;
        Box::pin(async move {
            if unchanged {
                return Ok(Outcome::Unchanged);
            }
            if request
                .selections
                .get("example.context")
                .is_some_and(|selectors| selectors != &["record:7".to_owned()])
            {
                return Ok(Outcome::Blocked {
                    message: "Resource selector is unavailable".into(),
                    receipt: json!({"invalid":true}),
                });
            }
            request.content.text.push_str(" captured context");
            Ok(Outcome::Ready {
                content: request.content,
                receipt: json!({"selector":"record:7"}),
                required_tools: Default::default(),
                basis: None,
            })
        })
    }
}
impl resources::Provider for Business {
    fn query(
        &self,
        _: resources::Query,
        _: resources::Context,
    ) -> BoxFuture<'static, Result<resources::Page, Error>> {
        Box::pin(async {
            Ok(resources::Page {
                items: vec![resources::Item {
                    id: "record:7".into(),
                    title: "Record seven".into(),
                    description: None,
                }],
                next_cursor: None,
            })
        })
    }
    fn resolve(
        &self,
        request: resources::Resolve,
        _: resources::Context,
    ) -> BoxFuture<'static, Result<resources::Value, Error>> {
        Box::pin(async move {
            Ok(resources::Value {
                selector: request.id,
                label: "Record seven".into(),
                quote: None,
            })
        })
    }
}
fn stage(owner: &Fiber, calls: Arc<AtomicUsize>, unchanged: bool) -> Staged {
    let mut staged = Staged::default();
    let business = Arc::new(Business { calls, unchanged });
    resources::stage(
        &mut staged,
        &owner.context(),
        "example.context",
        business.clone(),
        resources::Resources {
            descriptor: resources::Descriptor {
                title: Text::plain("Context"),
            },
            provider: business,
        },
        None,
    )
    .unwrap();
    staged
}
fn request(catalog: &Catalog, owner: &Fiber) -> Request {
    let snapshot = catalog.snapshot::<InputPreparation>(&Scope::Session("session".into()));
    let source = snapshot.entries.get("example.context").unwrap();
    let identity = owner.context().identity().unwrap();
    Request {
        session_id: "session".into(),
        cwd: ".".into(),
        content: "original".into(),
        selections: [("example.context".into(), vec!["record:7".into()])].into(),
        selection_sources: vec![SelectionSource {
            provider: "example.context".into(),
            package_id: identity.package_id,
            entry_id: identity.entry_id,
            activation: identity.activation,
            registration: source.value.resources().unwrap().registration,
            session_id: "session".into(),
        }],
        tools: Default::default(),
        cancellation: Default::default(),
    }
}
#[tokio::test]
async fn resource_selection_pins_both_actual_registrations_and_rejects_foreign_sources() {
    let catalog = Catalog::default();
    let owner = Fiber::new("example", "entry", Scope::Profile).unwrap();
    owner.begin_loading().unwrap();
    owner.ready().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut staged = stage(&owner, calls.clone(), false);
    staged.insert("sibling", 42_u64).unwrap();
    catalog.publish(&owner, staged).unwrap();
    let original = request(&catalog, &owner);
    let workspace = maka_plugins::filesystem::ReadRoot::capture(".").unwrap();
    let scope = Scope::Session("session".into());
    for invalid in 0..5 {
        let mut bad = original.clone();
        match invalid {
            0 => bad.selection_sources.clear(),
            1 => bad.selection_sources.push(bad.selection_sources[0].clone()),
            2 => bad.selection_sources[0].session_id = "other".into(),
            3 => bad.selection_sources[0].registration = uuid::Uuid::new_v4(),
            _ => bad.selection_sources[0].provider = "not-selected".into(),
        }
        assert!(
            input::prepare(&catalog, &scope, bad, &workspace)
                .await
                .is_err()
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "bad sources run no provider callbacks"
    );
    let prepared = input::prepare(&catalog, &scope, original.clone(), &workspace)
        .await
        .unwrap();
    assert_eq!(prepared.content.text, "original captured context");
    assert_eq!(
        prepared.content.preparation[0].source.name,
        "example.context"
    );
    drop(prepared.admit().unwrap().unwrap());
    let mut unsupported = original.clone();
    unsupported.selections.insert(
        "example.context".into(),
        vec!["execute-business-job".into()],
    );
    let rejected = input::prepare(&catalog, &scope, unsupported, &workspace)
        .await
        .unwrap();
    assert!(
        rejected.blocked.is_some(),
        "correct identity never grants execution selectors"
    );
    let key = maka_plugins::remote::key("example", "input.example.context").unwrap();
    catalog
        .withdraw::<maka_plugins::remote::Endpoint>(&owner.context(), &key)
        .unwrap();
    assert!(
        prepared.admit().is_err(),
        "paired endpoint retirement fences prepared admission"
    );
    assert!(
        !catalog
            .snapshot::<InputPreparation>(&scope)
            .entries
            .contains_key("example.context")
    );
    assert!(
        catalog.snapshot::<u64>(&scope).entries["sibling"].is_effective(),
        "pair retirement is not whole-batch retirement"
    );
    let count = calls.load(Ordering::SeqCst);
    assert!(
        input::prepare(&catalog, &scope, original.clone(), &workspace)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), count);
    catalog
        .withdraw::<InputPreparation>(&owner.context(), "example.context")
        .unwrap();
    catalog
        .publish(&owner, stage(&owner, calls.clone(), false))
        .unwrap();
    assert!(
        input::prepare(&catalog, &scope, original, &workspace)
            .await
            .is_err(),
        "same-name replacement never rescopes a saved selector"
    );
    let fresh = request(&catalog, &owner);
    input::prepare(&catalog, &scope, fresh, &workspace)
        .await
        .unwrap();
}
#[tokio::test]
async fn selected_resources_cannot_be_silently_ignored_by_preparation() {
    let catalog = Catalog::default();
    let owner = Fiber::new("example", "entry", Scope::Profile).unwrap();
    owner.begin_loading().unwrap();
    owner.ready().unwrap();
    catalog
        .publish(&owner, stage(&owner, Arc::new(AtomicUsize::new(0)), true))
        .unwrap();
    let request = request(&catalog, &owner);
    let workspace = maka_plugins::filesystem::ReadRoot::capture(".").unwrap();
    let outcome = input::prepare(
        &catalog,
        &Scope::Session("session".into()),
        request,
        &workspace,
    )
    .await;
    assert!(
        matches!(outcome,Err(Error::Invalid(message)) if message=="Selected input resources were not prepared")
    );
}
