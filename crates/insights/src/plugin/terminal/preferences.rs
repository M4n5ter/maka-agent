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
use crate::preferences::Preferences;

#[derive(Deserialize)]
pub(super) struct Snapshot {
    pub revision: Option<u64>,
    pub preferences: Preferences,
}

pub(super) async fn read(method: &dyn Method, caller: &Caller) -> Result<Snapshot, Error> {
    #[derive(Deserialize)]
    struct Response {
        snapshot: Snapshot,
    }
    let response: Response = decode(
        method
            .call(json!({"kind":"preferences"}), caller.clone())
            .await?,
    )?;
    Ok(response.snapshot)
}

fn initial(route: &Value) -> bool {
    route.is_null() || route.as_object().is_some_and(serde_json::Map::is_empty)
}

pub(super) fn place(route: Value, saved: &Preferences) -> Result<Place, Error> {
    let route = if initial(&route) {
        json!({"tab":saved.tab,"range":saved.range,"selection":saved.selection})
    } else {
        route
    };
    serde_json::from_value(route).map_err(|error| Error::Invalid(error.to_string()))
}

fn expected(revision: &str) -> Result<Option<u64>, Error> {
    let value = revision
        .strip_prefix("preferences:")
        .and_then(|value| value.split_once(':'))
        .map(|(value, _)| value)
        .ok_or_else(|| Error::Invalid("Missing saved view revision".into()))?;
    if value == "none" {
        Ok(None)
    } else {
        value
            .parse()
            .map(Some)
            .map_err(|_| Error::Invalid("Invalid saved view revision".into()))
    }
}

pub(super) fn decorate(
    mut view: View,
    revision: Option<u64>,
    words: &Words,
) -> Result<View, Error> {
    view.revision = format!(
        "preferences:{}:{}",
        revision.map_or_else(|| "none".into(), |revision| revision.to_string()),
        view.revision
    );
    view.actions.push(Action {
        fields: view.fields.iter().map(|field| field.id.clone()).collect(),
        ..action(
            "save-view",
            words.t("Save this view", "保存此视图", "儲存此檢視"),
        )
    });
    let controls = row(
        "saved-view-controls",
        vec![
            button("save-view", "save-view", Role::Normal),
            link(
                "load-view",
                words.t("Load saved view", "加载已保存视图", "載入已儲存檢視"),
                Value::Null,
            )
            .into(),
        ],
    );
    let root = match &mut view.root {
        Node::Scroll { child, .. } => child.as_mut(),
        root => root,
    };
    let Node::Column { children, .. } = root else {
        return Err(Error::Provider("Missing usage page controls".into()));
    };
    children.push(controls);
    view.validate()
        .map_err(|error| Error::Provider(error.to_string()))?;
    Ok(view)
}

pub(super) async fn submit(
    method: &dyn Method,
    mut submission: Submission,
    caller: Caller,
) -> Result<Reply, Error> {
    let revision = expected(&submission.revision)?;
    let original = if initial(&submission.route) {
        let saved = read(method, &caller).await?;
        if saved.revision != revision {
            return Ok(Reply::Conflict);
        }
        place(submission.route.clone(), &saved.preferences)?
    } else {
        place(submission.route.clone(), &Preferences::default())?
    };
    submission.route = original.route();
    if submission.action == "filter" {
        return activity::filter(submission);
    }
    // Activity's visible fields are the reviewed values, including un-applied edits.
    // Its session scope stays local; the existing shared preference only owns filters.
    let place: Place = if original.tab.as_deref() == Some("activity") {
        let Reply::Applied { route } = activity::filter(submission)? else {
            return Err(Error::Invalid("Invalid usage filters".into()));
        };
        serde_json::from_value(route).map_err(|error| Error::Invalid(error.to_string()))?
    } else {
        original
    };
    let preferences: Preferences = decode(json!({
        "tab":place.tab.as_deref().unwrap_or("overview"),
        "range":place.range.as_deref().unwrap_or("7d"),
        "selection":place.selection,
    }))?;
    let result = method
        .call(
            json!({"kind":"save_preferences","expectedRevision":revision,
        "preferences":preferences}),
            caller,
        )
        .await?;
    match result["kind"].as_str() {
        Some("refresh_required") => Ok(Reply::Conflict),
        Some("preferences") => Ok(Reply::Applied {
            route: place.route(),
        }),
        _ => Err(Error::OutcomeUnknown(
            "Saved view receipt is unavailable".into(),
        )),
    }
}

#[cfg(test)]
mod tests;
