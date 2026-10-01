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

use maka_client::Client;
use maka_plugins::input::resources::{Reply, Request};
use maka_protocol::plugin::{Query, QueryResult, RemoteBinding, RemoteRequest, RemoteResult, View};
use serde_json::json;

async fn directory(
    client: &Client,
    session: &str,
) -> Vec<maka_protocol::plugin::InputResourceProjection> {
    let result = client
        .plugin_query(Query {
            view: View::InputResources,
            root_id: Some(maka_plugins::composition::Scope::Session(session.into())),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    let QueryResult::InputResources(page) = result else {
        panic!("resource directory")
    };
    page.items
}
pub(super) async fn absent(client: &Client, session: &str) {
    assert!(
        directory(client, session)
            .await
            .iter()
            .all(|entry| entry.package_id != "maka.skills")
    );
}
pub(super) async fn exercise(
    client: &Client,
    session: &str,
    document: uuid::Uuid,
) -> RemoteRequest {
    let entry = directory(client, session)
        .await
        .into_iter()
        .find(|entry| entry.package_id == "maka.skills")
        .unwrap();
    let binding = RemoteBinding::Package {
        package_id: entry.package_id,
        method: entry.method,
        session_id: Some(session.into()),
    };
    let call = |input: Request| RemoteRequest::Call {
        binding: binding.clone(),
        target: entry.target.clone(),
        document,
        input: serde_json::to_value(input).unwrap(),
    };
    let mut cursor = None;
    let mut selectors = std::collections::BTreeSet::new();
    let mut continuation = None;
    loop {
        let request = call(Request::Query {
            query: "Review".into(),
            cursor: cursor.clone(),
            limit: 7,
            locale: "en".into(),
        });
        let RemoteResult::Value { value } = client.plugin_remote(request.clone()).await.unwrap()
        else {
            panic!("page")
        };
        let Reply::Page { items, next_cursor } = serde_json::from_value(value).unwrap() else {
            panic!("resource page")
        };
        assert!(items.len() <= 7);
        selectors.extend(items.into_iter().map(|item| item.id));
        if cursor.is_some() {
            continuation = Some(request);
        }
        let Some(next) = next_cursor else { break };
        cursor = Some(next);
    }
    assert_eq!(
        selectors
            .iter()
            .filter(|id| id.starts_with("review-"))
            .count(),
        129
    );
    let RemoteResult::Value { value } = client
        .plugin_remote(call(Request::Resolve {
            id: "review-000".into(),
            locale: "en".into(),
        }))
        .await
        .unwrap()
    else {
        panic!("resolved")
    };
    let Reply::Resolved {
        source,
        selector,
        label,
        ..
    } = serde_json::from_value(value).unwrap()
    else {
        panic!("selection")
    };
    assert_eq!(source.provider, "maka.skills");
    assert_eq!(source.session_id, session);
    assert_eq!(selector, "review-000");
    assert_eq!(label, "Review 0");
    let original = continuation.unwrap();
    let mut wrong = original.clone();
    if let RemoteRequest::Call { input, .. } = &mut wrong {
        input["query"] = json!("different query");
    }
    assert!(client.plugin_remote(wrong).await.is_err());
    original
}
