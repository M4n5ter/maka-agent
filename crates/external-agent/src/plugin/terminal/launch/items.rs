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
use sha2::{Digest, Sha256};
mod display;
pub(in crate::plugin::terminal) use display::view;
#[cfg(test)]
mod tests;

const CHUNK: usize = 2048;
const PAGE: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Args,
    Env,
    Executable,
}
impl Part {
    fn read(route: &Value) -> Result<Self, Error> {
        match route["launch"].as_str() {
            Some("args") => Ok(Self::Args),
            Some("env") => Ok(Self::Env),
            Some("executable") => Ok(Self::Executable),
            _ => Err(invalid("Unknown launch field")),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Args => "args",
            Self::Env => "env",
            Self::Executable => "executable",
        }
    }
    fn digest(self, agent: &Agent) -> String {
        let bytes = match self {
            Self::Args => serde_json::to_vec(&agent.args),
            Self::Env => serde_json::to_vec(&agent.env),
            Self::Executable => serde_json::to_vec(&agent.executable),
        }
        .expect("launch sequence");
        format!("{:x}", Sha256::digest(bytes))
    }
    fn revision(self, configuration: &Configuration, agent: &Agent) -> String {
        format!(
            "{}:{}:{}",
            stamp(configuration),
            self.name(),
            self.digest(agent)
        )
    }
    fn count(self, agent: &Agent) -> usize {
        match self {
            Self::Args => agent.args.len(),
            Self::Env => agent.env.len(),
            Self::Executable => 1,
        }
    }
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn number(route: &Value, key: &str) -> Result<usize, Error> {
    match route.get(key) {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| invalid("Invalid launch position")),
    }
}
fn item(part: Part, agent: &Agent, index: usize, name: bool) -> Result<&str, Error> {
    match part {
        Part::Args if !name => agent.args.get(index).map(String::as_str),
        Part::Executable if index == 0 && !name => Some(agent.executable.as_str()),
        Part::Env => agent
            .env
            .iter()
            .nth(index)
            .map(|(key, value)| if name { key.as_str() } else { value.as_str() }),
        _ => None,
    }
    .ok_or_else(|| invalid("This launch value is no longer present"))
}
fn bounds(value: &str) -> Vec<(usize, usize)> {
    if value.is_empty() {
        return vec![(0, 0)];
    }
    let mut parts = Vec::new();
    let mut start = 0;
    while start < value.len() {
        let mut end = (start + CHUNK).min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        parts.push((start, end));
        start = end;
    }
    parts
}
fn range(value: &str, offset: usize) -> Result<(usize, usize), Error> {
    bounds(value)
        .into_iter()
        .find(|(start, _)| *start == offset)
        .ok_or_else(|| invalid("This fragment no longer matches the saved value"))
}
fn listing(agent: &Agent, part: Part, page: usize) -> Value {
    json!({"agent":agent.id,"launch":part.name(),"items":true,"page":page})
}
fn selected(agent: &Agent, part: Part, index: usize, name: bool, offset: usize) -> Value {
    json!({"agent":agent.id,"launch":part.name(),"items":true,"index":index,"name":name,"offset":offset,"basis":part.digest(agent)})
}
pub(super) fn executable(agent: &Agent) -> Value {
    selected(agent, Part::Executable, 0, false, 0)
}
fn string(submission: &Submission, field: &str) -> Result<String, Error> {
    serde_json::from_str::<String>(submission.text(field)?)
        .map_err(|_| invalid("Use one JSON string, including its quotation marks"))
}
fn edit(agent: &mut Agent, part: Part, submission: &Submission) -> Result<Value, Error> {
    if submission.action == "launch-item-append" {
        let value = string(submission, "new-value")?;
        let index = match part {
            Part::Executable => return Err(invalid("The executable path cannot be appended")),
            Part::Args => {
                let index = agent.args.len();
                agent.args.push(value);
                index
            }
            Part::Env => {
                let name = string(submission, "new-name")?;
                if agent.env.contains_key(&name) {
                    return Err(invalid("An environment variable already has this name"));
                }
                agent.env.insert(name.clone(), value);
                agent
                    .env
                    .keys()
                    .position(|key| *key == name)
                    .expect("inserted variable")
            }
        };
        agent
            .validate()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        return Ok(selected(agent, part, index, false, 0));
    }
    if submission.route["basis"].as_str() != Some(part.digest(agent).as_str()) {
        return Err(invalid(
            "Launch values changed; reopen the list before editing",
        ));
    }
    let index = number(&submission.route, "index")?;
    if submission.action == "launch-item-remove" {
        match part {
            Part::Executable => return Err(invalid("The executable path cannot be removed")),
            Part::Args => {
                if index >= agent.args.len() {
                    return Err(invalid("Argument no longer exists"));
                }
                agent.args.remove(index);
            }
            Part::Env => {
                let name = agent
                    .env
                    .keys()
                    .nth(index)
                    .cloned()
                    .ok_or_else(|| invalid("Variable no longer exists"))?;
                agent.env.remove(&name);
            }
        }
        return Ok(listing(agent, part, index / PAGE));
    }
    if submission.action != "launch-item-save" {
        return Err(invalid("Unknown launch edit"));
    }
    let name = submission.route["name"].as_bool().unwrap_or(false);
    let original = item(part, agent, index, name)?;
    let (start, end) = range(original, number(&submission.route, "offset")?)?;
    let replacement = string(submission, "fragment")?;
    let next = format!("{}{}{}", &original[..start], replacement, &original[end..]);
    match part {
        Part::Args => agent.args[index] = next,
        Part::Executable => agent.executable = next,
        Part::Env => {
            let key = agent
                .env
                .keys()
                .nth(index)
                .cloned()
                .expect("located variable");
            if name {
                if next != key && agent.env.contains_key(&next) {
                    return Err(invalid("An environment variable already has this name"));
                }
                let value = agent.env.remove(&key).expect("located variable");
                agent.env.insert(next, value);
            } else {
                agent.env.insert(key, next);
            }
        }
    }
    agent
        .validate()
        .map_err(|e| Error::Invalid(e.to_string()))?;
    // Renaming may reorder the map. Return to the reviewed list instead of
    // pretending the same ordinal still identifies that variable.
    if name {
        return Ok(listing(agent, part, index / PAGE));
    }
    let value = item(part, agent, index, false)?;
    let offset = bounds(value)
        .into_iter()
        .map(|(start, _)| start)
        .rfind(|offset| *offset <= start)
        .unwrap_or(0);
    Ok(selected(agent, part, index, false, offset))
}
pub(in crate::plugin::terminal) async fn submit(
    this: &Agents,
    mut configuration: Configuration,
    submission: &Submission,
    caller: &Caller,
) -> Result<Reply, Error> {
    let id = submission.route["agent"]
        .as_str()
        .ok_or_else(|| invalid("No agent selected"))?;
    let part = Part::read(&submission.route)?;
    let Some(index) = configuration.agents.iter().position(|agent| agent.id == id) else {
        return Ok(Reply::Conflict);
    };
    if part.revision(&configuration, &configuration.agents[index]) != submission.revision {
        return Ok(Reply::Conflict);
    }
    let route = match edit(&mut configuration.agents[index], part, submission) {
        Ok(route) => route,
        Err(Error::Invalid(message)) => return Ok(Reply::Rejected { message }),
        Err(error) => return Err(error),
    };
    this.management.call(json!({"kind":"configure","expectedRevision":configuration.revision,"agents":configuration.agents}),caller.clone()).await?;
    Ok(Reply::Applied { route })
}
