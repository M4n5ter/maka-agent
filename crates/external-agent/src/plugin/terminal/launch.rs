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

pub(super) mod items;

use super::*;
use view::Field;

fn plain(id: &str, agent: &Agent) -> Option<String> {
    let text = if id == "args" {
        if agent
            .args
            .iter()
            .any(|arg| arg.is_empty() || arg.contains(['\n', '\r']))
        {
            return None;
        }
        agent.args.join("\n")
    } else {
        if agent
            .env
            .iter()
            .any(|(name, value)| name.contains(['\n', '\r']) || value.contains(['\n', '\r']))
        {
            return None;
        }
        agent
            .env
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    (text.len() <= view::MAX_TEXT && view::build::clean(&text, true) == text).then_some(text)
}
pub(super) fn field(id: &str, agent: Option<&Agent>) -> Field {
    let text = agent.map_or(Some(String::new()), |agent| plain(id, agent));
    let enabled = text.is_some();
    let mut field = area(id, text.unwrap_or_default(), view::MAX_TEXT);
    field.enabled = enabled;
    field
}
pub(super) fn executable_field(agent: Option<&Agent>) -> Field {
    let value = agent
        .map(|agent| agent.executable.as_str())
        .unwrap_or_default();
    let editable = value.len() <= 4096 && clean(value) == value;
    let mut field = line("executable", if editable { value } else { "" }, 4096);
    field.enabled = editable;
    field
}
pub(super) fn normal(
    submission: &Submission,
    id: String,
    previous: Option<&Agent>,
) -> Result<Agent, Error> {
    let args = match submission.fields.get("args").and_then(Value::as_str) {
        Some(value) if previous.and_then(|old| plain("args", old)).as_deref() != Some(value) => {
            if value.is_empty() {
                vec![]
            } else {
                value.split('\n').map(str::to_owned).collect()
            }
        }
        _ => previous.map(|old| old.args.clone()).unwrap_or_default(),
    };
    let env = match submission.fields.get("env").and_then(Value::as_str) {
        Some(value) if previous.and_then(|old| plain("env", old)).as_deref() != Some(value) => {
            let mut env = BTreeMap::new();
            for line in value.split('\n').filter(|line| !line.is_empty()) {
                let (name, value) = line.split_once('=').ok_or_else(|| {
                    Error::Invalid("Write each environment variable as NAME=value".into())
                })?;
                if env.insert(name.to_owned(), value.to_owned()).is_some() {
                    return Err(Error::Invalid(
                        "An environment variable appears twice".into(),
                    ));
                }
            }
            env
        }
        _ => previous.map(|old| old.env.clone()).unwrap_or_default(),
    };
    let executable = submission.fields.get("executable").and_then(Value::as_str);
    let executable = match executable {
        Some(value) => previous
            .filter(|old| clean(&old.executable) == value)
            .map_or_else(|| value.to_owned(), |old| old.executable.clone()),
        None => previous
            .map(|old| old.executable.clone())
            .ok_or_else(|| Error::Invalid("Missing executable".into()))?,
    };
    Ok(Agent {
        id,
        display_name: submission.text("name")?.trim().to_owned(),
        executable,
        args,
        env,
    })
}
pub(super) fn links(words: &Words, agent: &Agent) -> Node {
    let mut children = Vec::new();
    for (id, en, cn, tw) in [
        ("args", "Advanced arguments", "高级参数", "進階參數"),
        (
            "env",
            "Advanced environment",
            "高级环境变量",
            "進階環境變數",
        ),
    ] {
        let detail = if plain(id, agent).is_none() {
            words.t(
                "Saved complex values are kept unchanged",
                "已保存的复杂值会原样保留",
                "已儲存的複雜值會原樣保留",
            )
        } else {
            String::new()
        };
        children.push(
            link(
                format!("advanced-{id}"),
                words.t(en, cn, tw),
                json!({"agent":agent.id,"launch":id}),
            )
            .detail(detail)
            .into(),
        );
    }
    if agent.executable.len() > 4096 || clean(&agent.executable) != agent.executable {
        children.push(
            link(
                "executable-fragments",
                words.t(
                    "Edit full executable path",
                    "编辑完整可执行路径",
                    "編輯完整執行路徑",
                ),
                items::executable(agent),
            )
            .into(),
        );
    }
    stack("advanced", children)
}
fn document<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .expect("launch configuration")
        .chars()
        .fold(String::new(), |mut text, ch| {
            if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                text.push_str(&format!("\\u{:04x}", ch as u32));
            } else {
                text.push(ch);
            }
            text
        })
}
pub(super) fn advanced(
    words: &Words,
    configuration: &Configuration,
    agent: &Agent,
    part: &str,
) -> Result<View, Error> {
    let (title, source) = match part {
        "args" => (
            words.t("Advanced arguments", "高级参数", "進階參數"),
            document(&agent.args),
        ),
        "env" => (
            words.t("Advanced environment", "高级环境变量", "進階環境變數"),
            document(&agent.env),
        ),
        _ => return Err(Error::Invalid("Unknown launch field".into())),
    };
    let editable = source.len() <= view::MAX_TEXT;
    let mut children = vec![
        link("back", clean(&agent.display_name), json!({"agent":agent.id})).into(),
        text("hint", words.t("JSON preserves empty arguments, whitespace and line breaks. Other launch values stay unchanged.",
            "JSON 会保留空参数、空白和换行；其他启动配置保持不变。", "JSON 會保留空參數、空白和換行；其他啟動設定保持不變。"), Tone::Subtle),
    ];
    let (fields, actions) =
        if editable {
            children.extend([
                input("json", "launch-json", "JSON"),
                button("save-launch", "save-launch", Role::Primary),
            ]);
            (
                vec![area("launch-json", source, view::MAX_TEXT)],
                vec![Action {
                    fields: vec!["launch-json".into()],
                    ..action("save-launch", words.t("Save", "保存", "儲存"))
                }],
            )
        } else {
            children.push(text("limit", words.t(
            "Use individual values below to edit this large configuration without truncation.",
            "请通过下方逐项编辑完整修改大型配置，内容不会被截断。",
            "請透過下方逐項編輯完整修改大型設定，內容不會被截斷。"), Tone::Subtle));
            (vec![], vec![])
        };
    children.push(
        link(
            "individual",
            words.t("Edit individual values", "逐项编辑", "逐項編輯"),
            json!({"agent":agent.id,"launch":part,"items":true}),
        )
        .into(),
    );
    Ok(View {
        version: VERSION,
        title,
        revision: stamp(configuration),
        fields,
        actions,
        root: column("root", children),
    })
}
pub(super) fn update(agent: &mut Agent, submission: &Submission) -> Result<(), Error> {
    let source = submission.text("launch-json")?;
    match submission.route["launch"].as_str() {
        Some("args") => {
            agent.args = serde_json::from_str(source)
                .map_err(|_| Error::Invalid("Arguments must be a JSON array of strings".into()))?
        }
        Some("env") => {
            agent.env = serde_json::from_str(source).map_err(|_| {
                Error::Invalid("Environment must be a JSON object of string values".into())
            })?
        }
        _ => return Err(Error::Invalid("Unknown launch field".into())),
    }
    agent
        .validate()
        .map_err(|error| Error::Invalid(error.to_string()))
}

/// Escaping increases wire bytes. Never let a large ordinary field make the
/// entire editor unreadable or turn a rename into a truncated launch command.
pub(super) fn fit(view: &mut View) {
    for id in ["env", "args"] {
        if serde_json::to_vec(view).is_ok_and(|bytes| bytes.len() <= view::MAX_BYTES - 64) {
            break;
        }
        if let Some(field) = view.fields.iter_mut().find(|field| field.id == id) {
            field.enabled = false;
            if let view::Control::Text { value, .. } = &mut field.control {
                value.clear();
            }
            for action in &mut view.actions {
                action.fields.retain(|field| field != id);
            }
        }
    }
}

#[cfg(test)]
mod tests;
