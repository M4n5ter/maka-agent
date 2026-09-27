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
    composition::Scope,
    contributions::{Catalog, Staged},
    executor::{self, Context, Error, Executor, Outcome, Provider, Request, Search, SearchResult},
    fiber::Fiber,
};
use std::{collections::BTreeSet, sync::Arc};

struct Unused;
impl Provider for Unused {
    fn execute(&self, _: Request, _: Context) -> BoxFuture<'static, Result<Outcome, Error>> {
        panic!("search never executes")
    }
}

#[tokio::test]
async fn executor_search_pages_all_identities_and_fences_query_scope_generation_and_revision() {
    let catalog = Catalog::default();
    let fiber = Fiber::new("paging", "paging", Scope::Profile).unwrap();
    fiber.begin_loading().unwrap();
    fiber.ready().unwrap();
    fiber.publish().unwrap();
    let mut staged = Staged::default();
    for index in 0..137 {
        let id = format!("same.agent-{index:03}");
        staged
            .insert(
                &id,
                Executor {
                    id: id.clone().try_into().unwrap(),
                    display_name: "Same agent".into(),
                    capabilities: Default::default(),
                    provider: Arc::new(Unused),
                },
            )
            .unwrap();
    }
    let registration = catalog.register(&fiber.context(), staged).unwrap();
    let scope = Scope::Session("a".into());
    let SearchResult::Page { page: first } = executor::search(
        &catalog,
        &scope,
        Search {
            query: "same".into(),
            cursor: None,
        },
    )
    .unwrap() else {
        panic!("first page")
    };
    assert_eq!(first.executors.len(), 50);
    let cursor = first.next_cursor.unwrap();
    for (query, scope, cursor) in [
        ("different", scope.clone(), cursor.clone()),
        ("same", Scope::Session("b".into()), cursor.clone()),
        (
            "same",
            scope.clone(),
            executor::Cursor {
                generation: uuid::Uuid::new_v4(),
                ..cursor.clone()
            },
        ),
        (
            "same",
            scope.clone(),
            executor::Cursor {
                revision: cursor.revision + 1,
                ..cursor.clone()
            },
        ),
    ] {
        assert!(matches!(
            executor::search(
                &catalog,
                &scope,
                Search {
                    query: query.into(),
                    cursor: Some(cursor)
                }
            )
            .unwrap(),
            SearchResult::Stale
        ));
    }
    let mut query = Search {
        query: "same".into(),
        cursor: None,
    };
    let mut seen = BTreeSet::new();
    loop {
        let result = executor::search(&catalog, &scope, query.clone()).unwrap();
        assert!(serde_json::to_vec(&result).unwrap().len() <= 48 * 1024);
        let SearchResult::Page { page } = result else {
            panic!("stable page")
        };
        for item in page.executors {
            assert!(seen.insert(item.id.as_str().to_owned()));
        }
        let Some(next) = page.next_cursor else {
            assert!(page.complete);
            break;
        };
        assert!(!page.complete);
        assert!(next.offset > query.cursor.as_ref().map_or(0, |cursor| cursor.offset));
        query.cursor = Some(next);
    }
    assert_eq!(seen.len(), 137);
    drop(registration);
    assert!(matches!(
        executor::search(
            &catalog,
            &scope,
            Search {
                query: "same".into(),
                cursor: Some(cursor)
            }
        )
        .unwrap(),
        SearchResult::Stale
    ));
    fiber
        .shutdown(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
        .await
        .unwrap();
}

#[tokio::test]
async fn executor_pages_use_byte_budgets_and_never_return_a_nonadvancing_cursor() {
    for width in [7000, 49 * 1024] {
        let catalog = Catalog::default();
        let fiber = Fiber::new("bytes", "bytes", Scope::Profile).unwrap();
        fiber.begin_loading().unwrap();
        fiber.ready().unwrap();
        fiber.publish().unwrap();
        let mut staged = Staged::default();
        for index in 0..12 {
            let id = format!("bytes-{index}");
            staged
                .insert(
                    &id,
                    Executor {
                        id: id.clone().try_into().unwrap(),
                        display_name: "x".repeat(width),
                        capabilities: Default::default(),
                        provider: Arc::new(Unused),
                    },
                )
                .unwrap();
        }
        let registration = catalog.register(&fiber.context(), staged).unwrap();
        let result = executor::search(&catalog, &Scope::Profile, Search::default());
        if width > 48 * 1024 {
            assert!(result.is_err());
        } else {
            let SearchResult::Page { page } = result.unwrap() else {
                panic!("byte page")
            };
            assert!(page.executors.len() < 12 && !page.executors.is_empty());
            assert_eq!(
                page.next_cursor.unwrap().offset,
                page.executors.len() as u64
            );
        }
        drop(registration);
        fiber
            .shutdown(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
            .await
            .unwrap();
    }
}
