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

use super::{ElementRef, failed};
use maka_runtime::tools::ToolError;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

// Indices identify nodes, not their current row positions. Removed indices
// cannot acquire a new meaning, including after navigation or reconnection.
#[derive(Default)]
pub(super) struct Projection {
    document: String,
    indices: HashMap<String, u64>,
    next: u64,
}

impl Projection {
    pub(super) fn render(
        &mut self,
        document: &str,
        nodes: &[Value],
    ) -> Result<(String, HashMap<u64, ElementRef>), ToolError> {
        if self.document != document {
            self.document = document.to_owned();
            self.indices.clear();
        }
        let nodes: HashMap<&str, &Value> = nodes
            .iter()
            .take(2000)
            .filter_map(|node| Some((node["nodeId"].as_str()?, node)))
            .collect();
        let mut walk = Walk {
            projection: self,
            nodes: &nodes,
            seen: HashSet::new(),
            retained: HashSet::new(),
            rows: Vec::new(),
            refs: HashMap::new(),
        };
        // CDP supplies parents before children. Sort root IDs for a stable
        // presentation rather than relying on HashMap iteration order.
        let mut roots: Vec<_> = nodes
            .iter()
            .filter(|(_, node)| {
                node["parentId"]
                    .as_str()
                    .is_none_or(|id| !nodes.contains_key(id))
            })
            .map(|(id, _)| *id)
            .collect();
        roots.sort_unstable();
        for root in roots {
            walk.visit(root, 0, "")?;
        }
        walk.projection
            .indices
            .retain(|key, _| walk.retained.contains(key));
        Ok((walk.rows.join("\n"), walk.refs))
    }
}

struct Walk<'a> {
    projection: &'a mut Projection,
    nodes: &'a HashMap<&'a str, &'a Value>,
    seen: HashSet<&'a str>,
    retained: HashSet<String>,
    rows: Vec<String>,
    refs: HashMap<u64, ElementRef>,
}
impl<'a> Walk<'a> {
    fn visit(&mut self, id: &'a str, depth: usize, parent_name: &str) -> Result<(), ToolError> {
        if depth > 60 || !self.seen.insert(id) {
            return Ok(());
        }
        let Some(node) = self.nodes.get(id).copied() else {
            return Ok(());
        };
        let role = node["role"]["value"].as_str().unwrap_or("unknown");
        if role == "InlineTextBox" {
            return Ok(());
        }
        let name = node["name"]["value"].as_str().unwrap_or("");
        let content = &node["value"]["value"];
        let transparent = node["ignored"] == true
            || (matches!(role, "generic" | "none" | "LabelText")
                && name.is_empty()
                && node["actions"].as_array().is_none_or(Vec::is_empty))
            || (role == "StaticText" && !name.is_empty() && name == parent_name);
        let mut child_depth = depth;
        if !transparent {
            let scope = node["makaFrame"]
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let loader = node["makaFrame"]
                .get("loader")
                .and_then(Value::as_str)
                .unwrap_or("");
            let identity = node["backendDOMNodeId"]
                .as_u64()
                .map(|id| format!("{scope}:{loader}:dom:{id}"))
                .unwrap_or_else(|| format!("ax:{id}"));
            let index = if let Some(index) = self.projection.indices.get(&identity) {
                *index
            } else {
                let index = self.projection.next;
                self.projection.next = index
                    .checked_add(1)
                    .filter(|next| *next <= 9_007_199_254_740_991)
                    .ok_or_else(|| failed("accessibility index limit reached; reset Cua"))?;
                self.projection.indices.insert(identity.clone(), index);
                index
            };
            self.retained.insert(identity);
            if let Some(backend) = node["backendDOMNodeId"].as_u64() {
                self.refs.insert(
                    index,
                    ElementRef {
                        backend,
                        frame: serde_json::from_value(node["makaFrame"].clone()).ok(),
                    },
                );
            }
            let mut row = format!("{}{index} {role} {}", "  ".repeat(depth), json!(name));
            if !content.is_null() && content != "" {
                row.push_str(&format!(" value={content}"));
            }
            if let Some(actions) = node
                .get("actions")
                .filter(|value| value.as_array().is_some_and(|actions| !actions.is_empty()))
            {
                row.push_str(&format!(" actions={actions}"));
            }
            if let Some(properties) = node["properties"].as_array() {
                for property in properties {
                    let key = property["name"].as_str().unwrap_or("");
                    let value = &property["value"]["value"];
                    match key {
                        "disabled" | "focused" | "readonly" | "required" | "busy"
                            if value == true =>
                        {
                            row.push_str(&format!(" [{key}]"));
                        }
                        "checked" | "selected" | "expanded" if !value.is_null() => {
                            row.push_str(&format!(" [{key}={value}]"));
                        }
                        _ => {}
                    }
                }
            }
            self.rows.push(row);
            child_depth += 1;
        }
        let child_name = if transparent { parent_name } else { name };
        if let Some(children) = node["childIds"].as_array() {
            for child in children {
                if let Some(child) = child.as_str() {
                    self.visit(child, child_depth, child_name)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_decision_state_and_never_reassigns_removed_indices() {
        let mut projection = Projection::default();
        let tree = json!([
            {"nodeId":"1","role":{"value":"RootWebArea"},"name":{"value":"Form"},"childIds":["2","3"]},
            {"nodeId":"2","parentId":"1","backendDOMNodeId":10,"role":{"value":"button"},"name":{"value":"Delete"},"childIds":["4"],"properties":[{"name":"disabled","value":{"value":true}}]},
            {"nodeId":"3","parentId":"1","backendDOMNodeId":11,"role":{"value":"textbox"},"name":{"value":"Name"},"value":{"value":"你好\nvalue"},"properties":[{"name":"focused","value":{"value":true}}]},
            {"nodeId":"4","parentId":"2","role":{"value":"StaticText"},"name":{"value":"Delete"},"childIds":["5"]},
            {"nodeId":"5","parentId":"4","role":{"value":"InlineTextBox"},"name":{"value":"Delete"}}
        ]);
        let (state, refs) = projection
            .render("document1", tree.as_array().unwrap())
            .unwrap();
        assert_eq!(state.lines().count(), 3, "{state}");
        assert!(
            state.contains("  1 button \"Delete\" [disabled]"),
            "{state}"
        );
        assert!(
            state.contains("value=\"你好\\nvalue\" [focused]"),
            "{state}"
        );
        let removed = *refs.iter().find(|(_, id)| id.backend == 10).unwrap().0;
        let mut replacement = tree.as_array().unwrap().clone();
        replacement.retain(|node| !matches!(node["nodeId"].as_str(), Some("2" | "4" | "5")));
        replacement[0]["childIds"] = json!(["3", "6"]);
        replacement.push(json!({"nodeId":"6","parentId":"1","backendDOMNodeId":12,"role":{"value":"button"},"name":{"value":"Submit"}}));
        let (_, fresh) = projection.render("document1", &replacement).unwrap();
        assert!(!fresh.contains_key(&removed));
        assert_eq!(fresh.get(&2).map(|element| element.backend), Some(11));
        let (_, navigated) = projection
            .render("document2", tree.as_array().unwrap())
            .unwrap();
        assert!(navigated.keys().all(|key| !fresh.contains_key(key)));
    }
}
