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

pub(super) fn append(
    words: &Words,
    agent: &Agent,
    part: Part,
    route: &Value,
    children: &mut Vec<Node>,
    fields: &mut Vec<view::Field>,
    actions: &mut Vec<Action>,
) -> Result<(), Error> {
    let index = number(route, "index")?;
    let name = route["name"].as_bool().unwrap_or(false);
    let value = item(part, agent, index, name)?;
    let offset = number(route, "offset")?;
    let (start, end) = range(value, offset)?;
    let chunks = bounds(value);
    let position = chunks
        .iter()
        .position(|(at, _)| *at == start)
        .expect("selected fragment");
    children.push(
        link(
            "list",
            words.t("All values", "全部值", "全部值"),
            listing(agent, part, index / PAGE),
        )
        .into(),
    );
    if part == Part::Env {
        let label = if name {
            words.t("Edit value", "编辑值", "編輯值")
        } else {
            words.t("Edit variable name", "编辑变量名", "編輯變數名稱")
        };
        children.push(link("component", label, selected(agent, part, index, !name, 0)).into());
    }
    children.push(text(
        "position",
        words.t(
            &format!(
                "Value {} · fragment {}/{} · bytes {}–{} of {}",
                index + 1,
                position + 1,
                chunks.len(),
                start,
                end,
                value.len()
            ),
            &format!(
                "值 {} · 片段 {}/{} · 字节 {}–{} / {}",
                index + 1,
                position + 1,
                chunks.len(),
                start,
                end,
                value.len()
            ),
            &format!(
                "值 {} · 片段 {}/{} · 位元組 {}–{} / {}",
                index + 1,
                position + 1,
                chunks.len(),
                start,
                end,
                value.len()
            ),
        ),
        Tone::Subtle,
    ));
    children.push(text("meaning",words.t("Save replaces only this exact saved fragment. Other fragments stay unchanged. Use a JSON string to preserve spaces, line breaks and control characters.","保存只替换当前选定的原始片段，其他片段保持不变。JSON 字符串能精确保留空白、换行和控制字符。","儲存只替換目前選定的原始片段，其他片段保持不變。JSON 字串能精確保留空白、換行和控制字元。"),Tone::Subtle));
    fields.push(area(
        "fragment",
        document(&&value[start..end]),
        view::MAX_TEXT,
    ));
    children.push(input(
        "fragment",
        "fragment",
        words.t(
            "Fragment (JSON string)",
            "片段（JSON 字符串）",
            "片段（JSON 字串）",
        ),
    ));
    actions.push(Action {
        fields: vec!["fragment".into()],
        ..action(
            "launch-item-save",
            words.t("Save fragment", "保存片段", "儲存片段"),
        )
    });
    children.push(button("save", "launch-item-save", Role::Primary));
    let mut adjacent = Vec::new();
    if position > 0 {
        adjacent.push(
            link(
                "previous-fragment",
                words.t("Previous fragment", "上一片段", "上一片段"),
                selected(agent, part, index, name, chunks[position - 1].0),
            )
            .into(),
        );
    }
    if position + 1 < chunks.len() {
        adjacent.push(
            link(
                "next-fragment",
                words.t("Next fragment", "下一片段", "下一片段"),
                selected(agent, part, index, name, chunks[position + 1].0),
            )
            .into(),
        );
    }
    if !adjacent.is_empty() {
        children.push(row("fragments", adjacent));
    }
    if part != Part::Executable {
        actions.push(Action {
            confirm: Some(Confirm {
                title: if part == Part::Args {
                    words.t("Delete this argument?", "删除此参数？", "刪除此參數？")
                } else {
                    words.t("Delete this variable?", "删除此变量？", "刪除此變數？")
                },
                message: words.t(
                    &format!(
                        "Delete value {} from {}. All of its fragments are removed.",
                        index + 1,
                        clean(&agent.display_name)
                    ),
                    &format!(
                        "从 {} 删除第 {} 个值及其全部片段。",
                        clean(&agent.display_name),
                        index + 1
                    ),
                    &format!(
                        "從 {} 刪除第 {} 個值及其全部片段。",
                        clean(&agent.display_name),
                        index + 1
                    ),
                ),
                destructive: true,
            }),
            ..action(
                "launch-item-remove",
                words.t("Delete whole value", "删除整个值", "刪除整個值"),
            )
        });
        children.push(button("delete", "launch-item-remove", Role::Destructive));
    }
    Ok(())
}
