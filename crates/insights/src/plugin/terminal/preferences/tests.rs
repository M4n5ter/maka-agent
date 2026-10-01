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
use futures_util::FutureExt;
use std::{collections::BTreeMap, future::Future, sync::Mutex};

fn ready<T>(future: impl Future<Output = T>) -> T {
    future
        .now_or_never()
        .expect("fixture calls settle immediately")
}

#[derive(Default)]
struct State {
    revision: Option<u64>,
    preferences: Value,
    calls: Vec<Value>,
    writes: usize,
    unknown: bool,
}
struct Backend(Mutex<State>);
impl Method for Backend {
    fn call(&self, input: Value, _: Caller) -> BoxFuture<'static, Result<Value, Error>> {
        let result = (|| {
            let mut state = self.0.lock().unwrap();
            state.calls.push(input.clone());
            match input["kind"].as_str().unwrap() {
                "preferences" => Ok(json!({"kind":"preferences","snapshot":{
                    "revision":state.revision,"preferences":state.preferences}})),
                "save_preferences" => {
                    if input["expectedRevision"] != json!(state.revision) {
                        return Ok(json!({"kind":"refresh_required"}));
                    }
                    let _: Preferences =
                        serde_json::from_value(input["preferences"].clone()).unwrap();
                    state.preferences = input["preferences"].clone();
                    state.revision = Some(state.revision.unwrap_or(0) + 1);
                    state.writes += 1;
                    if state.unknown {
                        return Err(Error::OutcomeUnknown("lost preference receipt".into()));
                    }
                    Ok(json!({"kind":"preferences","snapshot":{
                        "revision":state.revision,"preferences":state.preferences}}))
                }
                "activity" => Ok(json!({"kind":"activity","page":{
                    "cursor":"fixed-usage","nextCursor":null,"attempts":[],"total":0}})),
                "summary" => Ok(json!({"kind":"summary","summary":{
                    "models":{"calls":0,"error":0,"input":{"known":0,"missing":0},
                        "output":{"known":0,"missing":0},"cacheRead":{"known":0,"missing":0},
                        "cost":{"knownUsd":0.0,"unvalued":0}},
                    "tools":{"calls":0,"error":0},"byProvider":[],"byModel":[],"byTool":[]}})),
                "prices" => Ok(json!({"kind":"prices","page":{
                    "kind":"page","revision":8,"entries":[],"nextOffset":null}})),
                _ => panic!("unexpected method {input}"),
            }
        })();
        Box::pin(async move { result })
    }
}
struct NoViews;
impl maka_plugins::remote::Views for NoViews {
    fn authorize(
        &self,
        _: maka_plugins::authorization::Request,
    ) -> BoxFuture<'_, Result<maka_plugins::call::Owned, Error>> {
        unreachable!()
    }
    fn session(&self) -> BoxFuture<'_, Result<maka_plugins::remote::SessionView, Error>> {
        unreachable!()
    }
    fn workspace(
        &self,
        _: maka_plugins::remote::WorkspaceViewInput,
    ) -> BoxFuture<'_, Result<maka_plugins::remote::SessionView, Error>> {
        unreachable!()
    }
    fn query_database(
        &self,
        _: maka_plugins::filesystem::database::Read,
    ) -> BoxFuture<
        '_,
        Result<
            Vec<maka_plugins::filesystem::database::Table>,
            maka_plugins::filesystem::database::Error,
        >,
    > {
        unreachable!()
    }
}
struct Fixture {
    backend: Arc<Backend>,
    method: Arc<dyn Method>,
    caller: Caller,
}
impl Fixture {
    fn new() -> Self {
        let backend = Arc::new(Backend(Mutex::new(State {
            revision: Some(7),
            preferences: json!({"tab":"activity","range":"30d","selection":{
                "kind":"model","status":"error","search":"saved filters"}}),
            ..Default::default()
        })));
        Self {
            method: app::method(Overview(backend.clone())),
            backend,
            caller: Caller {
                connection_id: uuid::Uuid::new_v4(),
                document_id: uuid::Uuid::new_v4(),
                client_instance_id: "saved-view-test".into(),
                session_id: None,
                access: maka_plugins::remote::Access::Granted,
                controls: Arc::new(()),
                views: Arc::new(NoViews),
                resources: Arc::default(),
                cancellation: Default::default(),
            },
        }
    }
    fn read(&self, route: Value, locale: &str) -> View {
        let value = ready(self.method.call(
            json!({"kind":"read","route":route,"locale":locale}),
            self.caller.clone(),
        ))
        .unwrap();
        let Reply::View { view } = serde_json::from_value(value).unwrap() else {
            panic!("view")
        };
        view.validate().unwrap();
        view
    }
    fn submit(
        &self,
        shown: &View,
        route: Value,
        action: &str,
        fields: BTreeMap<String, Value>,
    ) -> Result<Reply, Error> {
        let request = shown
            .submission(route, action, fields, "en".into())
            .unwrap();
        ready(
            self.method
                .call(serde_json::to_value(request).unwrap(), self.caller.clone()),
        )
        .map(|value| serde_json::from_value(value).unwrap())
    }
}
fn fields(view: &View) -> BTreeMap<String, Value> {
    view.fields
        .iter()
        .map(|field| {
            (
                field.id.clone(),
                match &field.control {
                    view::Control::Text { value, .. } | view::Control::Choice { value, .. } => {
                        json!(value)
                    }
                    view::Control::Toggle { value } => json!(value),
                },
            )
        })
        .collect()
}
fn destination(node: &Node, key: &str) -> Option<Value> {
    if let Node::Tabs { tabs, .. } = node
        && let Some(tab) = tabs.iter().find(|tab| tab.id == key)
    {
        return Some(tab.route.clone());
    }
    if let Node::Item {
        key: own,
        target: view::Target::Route { route },
        ..
    } = node
        && own == key
    {
        return Some(route.clone());
    }
    node.children()
        .into_iter()
        .find_map(|node| destination(node, key))
}

#[test]
fn public_app_loads_shared_defaults_only_for_empty_routes_and_saves_visible_filters() {
    for locale in ["en", "zh-CN", "zh-TW"] {
        let fixture = Fixture::new();
        let shown = fixture.read(Value::Null, locale);
        assert_eq!(fields(&shown)["range"], "30d");
        assert_eq!(fields(&shown)["search"], "saved filters");
        assert_eq!(destination(&shown.root, "load-view"), Some(Value::Null));
        assert_eq!(fixture.backend.0.lock().unwrap().writes, 0);
        let mut edited = fields(&shown);
        edited.insert("range".into(), json!("24h"));
        edited.insert("search".into(), json!("visible draft"));
        edited.insert("session".into(), json!("local-session"));
        let Reply::Applied { route } = fixture
            .submit(&shown, Value::Null, "save-view", edited)
            .unwrap()
        else {
            panic!("saved")
        };
        let state = fixture.backend.0.lock().unwrap();
        assert_eq!(state.writes, 1);
        assert_eq!(
            state.preferences,
            json!({"tab":"activity","range":"24h","selection":{
            "kind":"model","status":"error","search":"visible draft"}})
        );
        assert_eq!(route["session"], "local-session");
        drop(state);
        fixture.backend.0.lock().unwrap().preferences["range"] = json!("all");
        let resumed = fixture.read(route.clone(), locale);
        assert_eq!(fields(&resumed)["range"], "24h");
        assert_eq!(fields(&resumed)["search"], "visible draft");
        let fresh = fixture.read(json!({}), locale);
        assert_eq!(fields(&fresh)["range"], "all");
        let explicit = fixture.read(json!({"tab":"activity"}), locale);
        assert_eq!(fields(&explicit)["range"], "7d");
        assert_eq!(fields(&explicit)["search"], "");
        let Reply::Applied { route } = fixture
            .submit(&fresh, Value::Null, "filter", fields(&fresh))
            .unwrap()
        else {
            panic!("filters")
        };
        assert_eq!(route["range"], "all");
        assert_eq!(fixture.backend.0.lock().unwrap().writes, 1);
    }
}

#[test]
fn public_app_saved_view_conflicts_and_unknown_writes_never_retarget_or_replay() {
    let fixture = Fixture::new();
    let initial = fixture.read(Value::Null, "en");
    fixture.backend.0.lock().unwrap().revision = Some(8);
    assert!(matches!(
        fixture
            .submit(&initial, Value::Null, "save-view", fields(&initial))
            .unwrap(),
        Reply::Conflict
    ));
    assert_eq!(fixture.backend.0.lock().unwrap().writes, 0);
    let route = json!({"tab":"overview","range":"24h","selection":{"search":"explicit"}});
    let shown = fixture.read(route.clone(), "en");
    fixture.backend.0.lock().unwrap().revision = Some(9);
    assert!(matches!(
        fixture
            .submit(&shown, route.clone(), "save-view", BTreeMap::new())
            .unwrap(),
        Reply::Conflict
    ));
    let shown = fixture.read(route.clone(), "en");
    assert!(shown.action("save-view").unwrap().recovery.is_none());
    fixture.backend.0.lock().unwrap().unknown = true;
    assert!(matches!(
        fixture.submit(&shown, route.clone(), "save-view", BTreeMap::new()),
        Err(Error::OutcomeUnknown(_))
    ));
    assert_eq!(fixture.backend.0.lock().unwrap().writes, 1);
    fixture.read(route, "en");
    assert_eq!(fixture.backend.0.lock().unwrap().writes, 1);
}

#[test]
fn public_app_all_saved_tabs_keep_explicit_navigation_and_render_save_controls() {
    let fixture = Fixture::new();
    for tab in [
        "overview",
        "activity",
        "providers",
        "models",
        "tools",
        "pricing",
    ] {
        let route = json!({"tab":tab,"range":"24h","selection":{"search":"local"}});
        let shown = fixture.read(route.clone(), "en");
        shown.action("save-view").unwrap();
        let Reply::Applied { route: saved } = fixture
            .submit(&shown, route, "save-view", fields(&shown))
            .unwrap()
        else {
            panic!("saved")
        };
        assert_eq!(saved["tab"], tab);
        assert_eq!(fixture.backend.0.lock().unwrap().preferences["tab"], tab);
        let reopened = fixture.read(Value::Null, "en");
        reopened.action("save-view").unwrap();
        if tab == "pricing" {
            let route = destination(&reopened.root, "activity").unwrap();
            assert_eq!(route["range"], "24h");
            assert_eq!(route["selection"]["search"], "local");
        }
    }
}

#[test]
fn public_app_saves_the_first_shared_view_with_the_absent_record_basis() {
    let fixture = Fixture::new();
    {
        let mut state = fixture.backend.0.lock().unwrap();
        state.revision = None;
        state.preferences = serde_json::to_value(Preferences::default()).unwrap();
    }
    let shown = fixture.read(Value::Null, "en");
    assert!(shown.revision.starts_with("preferences:none:"));
    assert!(matches!(
        fixture
            .submit(&shown, Value::Null, "save-view", BTreeMap::new())
            .unwrap(),
        Reply::Applied { .. }
    ));
    assert_eq!(fixture.backend.0.lock().unwrap().writes, 1);
}
