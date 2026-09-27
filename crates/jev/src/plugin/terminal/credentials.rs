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
use crate::settings::{HeaderPatch, SecretChange, SecretsPatch};
use view::{Confirm, Field, Node};

fn secret(words: &Words, saved: bool) -> Field {
    Field {
        control: Control::Text {
            value: String::new(),
            max_bytes: 4096,
            multiline: false,
            secret: true,
            placeholder: if saved {
                words.t(
                    "Saved value is kept unless replaced or removed",
                    "未替换或移除时保留已保存值",
                    "未替換或移除時保留已儲存值",
                )
            } else {
                words.t("Enter a value", "输入值", "輸入值")
            },
        },
        ..line("key", "", 4096)
    }
}
fn change(words: &Words, saved: bool) -> Field {
    choice(
        "secret-change",
        if saved { "keep" } else { "replace" },
        vec![
            (
                "keep".into(),
                words.t("Keep saved value", "保留已保存值", "保留已儲存值"),
            ),
            (
                "replace".into(),
                words.t("Replace value", "替换值", "替換值"),
            ),
            ("remove".into(), words.t("Remove value", "移除值", "移除值")),
        ],
    )
}
pub(super) fn append(
    words: &Words,
    snapshot: &Snapshot,
    fields: &mut Vec<Field>,
    actions: &mut Vec<Action>,
    children: &mut Vec<Node>,
) {
    fields.extend([
        change(words, snapshot.api_key_configured),
        secret(words, snapshot.api_key_configured),
    ]);
    children.push(text("auth-endpoint", words.t(
        "Authentication belongs to the saved endpoint. Save an endpoint change before editing its credentials.",
        "认证信息属于已保存的端点。修改端点后，请先保存再编辑其凭据。",
        "認證資訊屬於已儲存的端點。修改端點後，請先儲存再編輯其憑證。"), Tone::Subtle));
    children.push(stack(
        "api-key",
        vec![
            input(
                "key-change",
                "secret-change",
                words.t("API key", "API 密钥", "API 金鑰"),
            ),
            input("key", "key", words.t("New value", "新值", "新值")),
        ],
    ));
    actions.push(Action {
        fields: vec!["secret-change".into(), "key".into()],
        ..action(
            "save-key",
            words.t("Save key change", "保存密钥变更", "儲存金鑰變更"),
        )
    });
    let mut buttons = vec![button("save-key", "save-key", Role::Normal)];
    if snapshot.api_key_configured {
        actions.push(Action {
            confirm: Some(Confirm {
                title: words.t("Remove the Jev key?", "移除 Jev 密钥？", "移除 Jev 金鑰？"),
                message: words.t(
                    "Custom request headers remain saved for this endpoint.",
                    "此端点的自定义请求头会保留。",
                    "此端點的自訂請求標頭會保留。",
                ),
                destructive: true,
            }),
            ..action("forget", words.t("Remove key", "移除密钥", "移除金鑰"))
        });
        buttons.push(button("forget", "forget", Role::Destructive));
    }
    children.push(row("key-controls", buttons));
    let mut headers = vec![text(
        "headers-title",
        words.t("Request headers", "请求头", "請求標頭"),
        Tone::Muted,
    )];
    for (index, name) in snapshot.header_names.iter().enumerate() {
        headers.push(
            link(
                format!("header-{index}"),
                clean(name),
                json!({"header":name}),
            )
            .into(),
        );
    }
    headers.push(
        link(
            "add-header",
            words.t("Add request header", "添加请求头", "新增請求標頭"),
            json!({"header":null}),
        )
        .into(),
    );
    children.push(stack("headers", headers));
}
pub(super) fn header(words: &Words, snapshot: &Snapshot, name: Option<&str>) -> View {
    let saved = name.is_some_and(|name| snapshot.header_names.iter().any(|old| old == name));
    let mut field = line("header-name", name.unwrap_or_default(), 128);
    field.enabled = name.is_none();
    View {
        version: VERSION,
        revision: stamp(snapshot),
        title: words.t("Jev request header", "Jev 请求头", "Jev 請求標頭"),
        fields: vec![field, change(words, saved), secret(words, saved)],
        actions: vec![Action {
            fields: if name.is_none() {
                vec!["header-name".into(), "secret-change".into(), "key".into()]
            } else {
                vec!["secret-change".into(), "key".into()]
            },
            ..action(
                "save-header",
                words.t("Save header change", "保存请求头变更", "儲存請求標頭變更"),
            )
        }],
        root: column(
            "root",
            vec![
                link("back", "Jev", Value::Null).into(),
                text("endpoint", clean(&snapshot.settings.url), Tone::Subtle),
                input(
                    "name",
                    "header-name",
                    words.t("Header name", "请求头名称", "請求標頭名稱"),
                ),
                input(
                    "change",
                    "secret-change",
                    words.t("Saved value", "已保存值", "已儲存值"),
                ),
                input("value", "key", words.t("New value", "新值", "新值")),
                button("save-header", "save-header", Role::Primary),
            ],
        ),
    }
}
fn requested(submission: &Submission) -> Result<SecretChange, Error> {
    let value = submission.text("key")?.to_owned();
    match submission.text("secret-change")? {
        "replace" => Ok(SecretChange::Replace { value }),
        "keep" if value.is_empty() => Ok(SecretChange::Keep),
        "remove" if value.is_empty() => Ok(SecretChange::Remove),
        "keep" | "remove" => Err(Error::Invalid(
            "Choose Replace value to use the entered secret".into(),
        )),
        _ => Err(Error::Invalid("Unknown credential change".into())),
    }
}
pub(super) async fn submit(
    this: &Settings,
    snapshot: &Snapshot,
    submission: &Submission,
    cx: Cx,
) -> Result<Reply, Error> {
    let change = if submission.action == "forget" {
        SecretChange::Remove
    } else {
        match requested(submission) {
            Ok(change) => change,
            Err(error) => return rejected(error),
        }
    };
    if matches!(change, SecretChange::Keep) {
        return Ok(Reply::Applied { route: Value::Null });
    }
    let patch = if submission.action == "save-header" {
        let name = match submission.route["header"].as_str() {
            Some(name) => name.to_owned(),
            None => submission.text("header-name")?.to_owned(),
        };
        SecretsPatch {
            api_key: SecretChange::Keep,
            headers: vec![HeaderPatch { name, change }],
        }
    } else {
        SecretsPatch {
            api_key: change,
            headers: vec![],
        }
    };
    match this
        .call(
            json!({"kind":"credential_patch","url":snapshot.settings.url,
        "expectedRevision":snapshot.credential_revision,"patch":patch}),
            cx.caller,
        )
        .await
    {
        Ok(_) => Ok(Reply::Applied { route: Value::Null }),
        Err(error) => rejected(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_fields_are_secret_and_endpoint_settings_never_submit_them() {
        let snapshot = Snapshot {
            revision: Some(1),
            settings: Setup {
                enabled: true,
                url: "https://example.com/jev".into(),
                model: "one".into(),
                timeout_ms: 1000,
            },
            credential_revision: Some(3),
            api_key_configured: true,
            header_names: vec!["X-Gateway".into()],
        };
        for locale in ["en", "zh-CN", "zh-TW"] {
            let words = Words::new(locale);
            let setup = form(&words, &snapshot, None);
            setup.validate().unwrap();
            assert!(
                !setup
                    .action("save")
                    .unwrap()
                    .fields
                    .iter()
                    .any(|field| field == "key")
            );
            assert_eq!(
                setup.action("save-key").unwrap().fields,
                ["secret-change", "key"]
            );
            for name in [None, Some("X-Gateway")] {
                let shown = header(&words, &snapshot, name);
                shown.validate().unwrap();
                assert!(
                    matches!(&shown.field("key").unwrap().control,Control::Text {secret:true,value,..} if value.is_empty())
                );
                let mut fields = std::collections::BTreeMap::from([
                    ("secret-change".into(), json!("keep")),
                    ("key".into(), json!("")),
                ]);
                if name.is_none() {
                    fields.insert("header-name".into(), json!("X-New"));
                }
                shown
                    .submission(json!({"header":name}), "save-header", fields, locale.into())
                    .unwrap();
            }
        }
    }
}
