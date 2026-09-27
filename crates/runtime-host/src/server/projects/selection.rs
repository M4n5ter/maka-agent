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

use crate::server::Host;
use maka_plugins::{
    fiber::CallGuard,
    remote::{
        Error,
        projects::{Output, Project, Query},
    },
};
use tokio_util::sync::CancellationToken;

pub(in crate::server) async fn query(
    host: &Host,
    query: Query,
    cancellation: CancellationToken,
    lease: CallGuard,
) -> Result<Output, Error> {
    let records = host
        .log
        .list_projects()
        .await
        .map_err(|error| Error::Provider(error.to_string()))?;
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        let mut projects = Vec::new();
        for record in records {
            if cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let available =
                record.archived_at.is_none() && !super::projection::available(&record).is_empty();
            projects.push(Project {
                id: record.id,
                name: record.name,
                available,
            });
        }
        page(projects, query)
    })
    .await
    .map_err(|error| Error::Provider(error.to_string()))?
}

fn page(mut projects: Vec<Project>, query: Query) -> Result<Output, Error> {
    query.validate()?;
    projects.sort_by(|a, b| a.id.cmp(&b.id));
    let revision = maka_runtime::artifact::content_digest(
        &serde_json::to_vec(&projects).map_err(|error| Error::Provider(error.to_string()))?,
    );
    let start = match query {
        Query::Start => 0,
        Query::Continue {
            revision: expected,
            cursor,
        } => {
            if revision != expected {
                return Ok(Output::Changed { revision });
            }
            projects
                .binary_search_by(|project| project.id.cmp(&cursor))
                .map_err(|_| Error::Invalid("Project continuation was not in its catalog".into()))?
                + 1
        }
    };
    let end = (start + 32).min(projects.len());
    let next_cursor = (end < projects.len()).then(|| projects[end - 1].id.clone());
    let output = Output::Page {
        revision,
        projects: projects.drain(start..end).collect(),
        next_cursor,
    };
    maka_plugins::remote::validate_payload(
        &serde_json::to_value(&output).map_err(|error| Error::Provider(error.to_string()))?,
    )?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_pages_exhaust_inventory_and_reject_changed_metadata() {
        let projects: Vec<_> = (0..97)
            .map(|i| Project {
                id: format!("project-{i:03}"),
                name: format!("Project {i} 中文"),
                available: i % 2 == 0,
            })
            .collect();
        let mut query = Query::Start;
        let mut ids = Vec::new();
        loop {
            let Output::Page {
                revision,
                projects: rows,
                next_cursor,
            } = page(projects.clone(), query).unwrap()
            else {
                panic!()
            };
            ids.extend(rows.iter().map(|row| row.id.clone()));
            let Some(cursor) = next_cursor else { break };
            query = Query::Continue { revision, cursor };
            let mut changed = projects.clone();
            changed[0].available = false;
            assert!(matches!(
                page(changed, query.clone()).unwrap(),
                Output::Changed { .. }
            ));
        }
        assert_eq!(
            ids,
            projects.iter().map(|p| p.id.clone()).collect::<Vec<_>>()
        );
    }
}
