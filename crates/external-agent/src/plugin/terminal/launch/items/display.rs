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

pub(in crate::plugin::terminal) fn view(
    words: &Words,
    configuration: &Configuration,
    agent: &Agent,
    route: &Value,
) -> Result<View, Error> {
    let part = Part::read(route)?;
    let title = match part {
        Part::Args => words.t("Arguments", "参数", "參數"),
        Part::Env => words.t("Environment variables", "环境变量", "環境變數"),
        Part::Executable => words.t("Executable path", "可执行路径", "執行路徑"),
    };
    let mut children = vec![
        link(
            "agent",
            clean(&agent.display_name),
            json!({"agent":agent.id}),
        )
        .into(),
    ];
    let mut fields = Vec::new();
    let mut actions = Vec::new();
    if route.get("index").is_some() {
        if route["basis"].as_str() != Some(part.digest(agent).as_str()) {
            children.push(text(
                "changed",
                words.t(
                    "Saved values changed. Reopen the list before editing.",
                    "已保存的值已变更，请重新打开列表后编辑。",
                    "已儲存的值已變更，請重新開啟清單後編輯。",
                ),
                Tone::Warning,
            ));
            children.push(
                link(
                    "list",
                    words.t("Review current values", "审查当前值", "審查目前值"),
                    listing(agent, part, 0),
                )
                .into(),
            );
        } else {
            detail::append(
                words,
                agent,
                part,
                route,
                &mut children,
                &mut fields,
                &mut actions,
            )?;
        }
    } else {
        let count = part.count(agent);
        let page = number(route, "page")?.min(count.saturating_sub(1) / PAGE);
        let start = page * PAGE;
        children.push(text(
            "count",
            words.t(
                &format!("{count} saved values"),
                &format!("已保存 {count} 个值"),
                &format!("已儲存 {count} 個值"),
            ),
            Tone::Subtle,
        ));
        for index in start..(start + PAGE).min(count) {
            let value = item(part, agent, index, false)?;
            let name = if part == Part::Executable {
                words.t("Executable", "可执行文件", "可執行檔")
            } else if part == Part::Args {
                format!("#{}", index + 1)
            } else {
                document(&item(part, agent, index, true)?)
                    .chars()
                    .take(80)
                    .collect()
            };
            children.push(
                link(
                    format!("item-{index}"),
                    name,
                    selected(agent, part, index, false, 0),
                )
                .detail(document(&value).chars().take(100).collect::<String>())
                .into(),
            );
        }
        let mut pages = Vec::new();
        if page > 0 {
            pages.push(
                link(
                    "previous",
                    words.t("Previous", "上一页", "上一頁"),
                    listing(agent, part, page - 1),
                )
                .into(),
            );
        }
        if start + PAGE < count {
            pages.push(
                link(
                    "next",
                    words.t("Next", "下一页", "下一頁"),
                    listing(agent, part, page + 1),
                )
                .into(),
            );
        }
        if !pages.is_empty() {
            children.push(row("pages", pages));
        }
        if part != Part::Executable {
            // Two 12 KiB JSON fields still fit after escaping in the 64 KiB request.
            let mut sent = Vec::new();
            if part == Part::Env {
                fields.push(area("new-name", "\"\"", 12 * 1024));
                sent.push("new-name".into());
                children.push(input(
                    "new-name",
                    "new-name",
                    words.t(
                        "New name (JSON string)",
                        "新名称（JSON 字符串）",
                        "新名稱（JSON 字串）",
                    ),
                ));
            }
            fields.push(area("new-value", "\"\"", 12 * 1024));
            sent.push("new-value".into());
            children.push(input(
                "new-value",
                "new-value",
                words.t(
                    "New value (JSON string)",
                    "新值（JSON 字符串）",
                    "新值（JSON 字串）",
                ),
            ));
            children.push(text("append-help",words.t("Add an initial value, then edit its fragments to extend it. Empty arguments and empty variable values are valid.","先添加初始值，再编辑片段即可扩展。参数和变量值都可以为空。","先新增初始值，再編輯片段即可擴充。參數和變數值都可以為空。"),Tone::Subtle));
            actions.push(Action {
                fields: sent,
                ..action(
                    "launch-item-append",
                    words.t("Add value", "添加值", "新增值"),
                )
            });
            children.push(button("append", "launch-item-append", Role::Primary));
        }
    }
    Ok(View {
        version: VERSION,
        title,
        revision: part.revision(configuration, agent),
        fields,
        actions,
        root: column("root", children),
    })
}
mod detail;
