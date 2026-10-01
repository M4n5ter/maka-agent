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
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

pub(super) struct Backend {
    pub state: Mutex<(u64, Vec<Agent>)>,
    pub writes: AtomicUsize,
    pub unknown: AtomicBool,
}
impl Method for Backend {
    fn call(&self, input: Value, _: Caller) -> BoxFuture<'static, Result<Value, Error>> {
        let result = (|| {
            let mut state = self.state.lock().unwrap();
            if input["kind"] == "configure" {
                if input["expectedRevision"].as_u64() != Some(state.0) {
                    return Err(Error::Provider("CAS conflict".into()));
                }
                let agents: Vec<Agent> = serde_json::from_value(input["agents"].clone())
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                for agent in &agents {
                    agent
                        .validate()
                        .map_err(|e| Error::Invalid(e.to_string()))?;
                }
                state.0 += 1;
                state.1 = agents;
                self.writes.fetch_add(1, Ordering::SeqCst);
                if self.unknown.load(Ordering::SeqCst) {
                    return Err(Error::OutcomeUnknown("lost configure reply".into()));
                }
            }
            Ok(json!({"revision":state.0,"agents":state.1}))
        })();
        Box::pin(async move { result })
    }
}
struct NoSetup;
impl StreamProvider for NoSetup {
    fn open(
        &self,
        _: Value,
        _: Caller,
    ) -> BoxFuture<'static, Result<Box<dyn remote::Stream>, Error>> {
        unreachable!("editing launch settings must not start the process or installer")
    }
}
struct NoViews;
impl remote::Views for NoViews {
    fn authorize(
        &self,
        _: maka_plugins::authorization::Request,
    ) -> BoxFuture<'_, Result<maka_plugins::call::Owned, Error>> {
        unreachable!()
    }
    fn session(&self) -> BoxFuture<'_, Result<remote::SessionView, Error>> {
        unreachable!()
    }
    fn workspace(
        &self,
        _: remote::WorkspaceViewInput,
    ) -> BoxFuture<'_, Result<remote::SessionView, Error>> {
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
pub(super) struct Fixture {
    pub method: Arc<dyn Method>,
    pub backend: Arc<Backend>,
    caller: Caller,
}
impl Fixture {
    pub fn new(agent: Agent) -> Self {
        agent.validate().unwrap();
        let backend = Arc::new(Backend {
            state: Mutex::new((1, vec![agent])),
            writes: AtomicUsize::new(0),
            unknown: AtomicBool::new(false),
        });
        let method = app::method(Agents {
            management: backend.clone(),
            setup: Arc::new(NoSetup),
            pages: auth::Pages::default(),
        });
        Self {
            method,
            backend,
            caller: Caller {
                connection_id: uuid::Uuid::new_v4(),
                document_id: uuid::Uuid::new_v4(),
                client_instance_id: "launch-editor".into(),
                session_id: None,
                access: remote::Access::HostPaths,
                controls: Arc::new(()),
                views: Arc::new(NoViews),
                resources: Arc::default(),
                cancellation: CancellationToken::new(),
            },
        }
    }
    pub fn with_auth(mut self, checked: sign_in::Checked, phase: Option<auth::Phase>) -> Self {
        self.method = app::method(Agents {
            management: self.backend.clone(),
            setup: Arc::new(NoSetup),
            pages: auth::Pages::observed_for_test(&self.caller, checked, phase),
        });
        self
    }
    pub fn stored(&self) -> Agent {
        self.backend.state.lock().unwrap().1[0].clone()
    }
    pub async fn read(&self, route: Value) -> View {
        self.read_locale(route, "en").await
    }
    pub async fn read_locale(&self, route: Value, locale: &str) -> View {
        let value = self
            .method
            .call(
                json!({"kind":"read","route":route,"locale":locale}),
                self.caller.clone(),
            )
            .await
            .unwrap();
        assert!(serde_json::to_vec(&value).unwrap().len() <= view::MAX_BYTES);
        let Reply::View { view } = serde_json::from_value(value).unwrap() else {
            panic!("app read");
        };
        view.validate().unwrap();
        view
    }
    pub async fn submit(
        &self,
        shown: &View,
        route: Value,
        action: &str,
        fields: BTreeMap<String, Value>,
    ) -> Result<Reply, Error> {
        let request = shown
            .submission(route, action, fields, "en".into())
            .unwrap();
        let value = serde_json::to_value(request).unwrap();
        assert!(serde_json::to_vec(&value).unwrap().len() <= view::MAX_BYTES);
        let value = self.method.call(value, self.caller.clone()).await?;
        Ok(serde_json::from_value(value).unwrap())
    }
}
pub(super) fn destination(view: &View, key: &str) -> Value {
    fn find<'a>(node: &'a Node, key: &str) -> Option<&'a Value> {
        if node.key() == key
            && let Node::Item {
                target: view::Target::Route { route },
                ..
            } = node
        {
            return Some(route);
        }
        node.children()
            .into_iter()
            .find_map(|child| find(child, key))
    }
    find(&view.root, key)
        .unwrap_or_else(|| panic!("missing destination {key}"))
        .clone()
}
pub(super) fn text_field(view: &View, id: &str) -> String {
    match &view.field(id).unwrap().control {
        view::Control::Text { value, .. } => value.clone(),
        _ => panic!("text field"),
    }
}
pub(super) fn applied(reply: Reply) -> Value {
    let Reply::Applied { route } = reply else {
        panic!("applied action");
    };
    route
}
pub(super) fn agent() -> Agent {
    Agent {
        id: "fixture".into(),
        display_name: "Fixture".into(),
        executable: std::env::temp_dir()
            .join("fixture")
            .to_string_lossy()
            .into_owned(),
        args: vec![],
        env: BTreeMap::new(),
    }
}
