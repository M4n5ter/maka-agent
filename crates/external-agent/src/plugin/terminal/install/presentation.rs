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
    attempt: Option<Attempt>,
    connected: bool,
) -> View {
    let mut children = vec![link("back", words.t("All agents", "所有代理", "所有代理"), Value::Null).into(),
        text("intro", words.t("Download the official Antigravity distribution into this plugin's private storage. After installation, review and add its launch configuration separately.",
            "将官方 Antigravity 发行版下载到本插件的私有存储。安装完成后，可单独审查并添加启动配置。", "將官方 Antigravity 發行版下載到本插件的私有儲存。安裝完成後，可單獨審查並新增啟動設定。"), Tone::Muted)];
    let mut actions = Vec::new();
    let can_start = attempt
        .as_ref()
        .is_none_or(|attempt| matches!(attempt.phase, Phase::Cancelled | Phase::Failed));
    if can_start {
        actions.push(Action {
            enabled: connected,
            ..action(
                "install-antigravity",
                if connected {
                    words.t("Download and install", "下载并安装", "下載並安裝")
                } else {
                    words.t("Connecting…", "正在连接…", "正在連線…")
                },
            )
        });
        children.push(button(
            "install-antigravity",
            "install-antigravity",
            Role::Primary,
        ));
    }
    if let Some(attempt) = attempt {
        let label = match attempt.phase {
            Phase::Installing => words.t(
                "Waiting for approval or installing…",
                "正在等待授权或安装…",
                "正在等待授權或安裝…",
            ),
            Phase::Cancelling => words.t(
                "Cancelling; waiting for cleanup…",
                "正在取消，等待清理完成…",
                "正在取消，等待清理完成…",
            ),
            Phase::Installed => words.t(
                "Installed. Review the launch configuration before adding it.",
                "安装完成。添加前请审查启动配置。",
                "安裝完成。新增前請審查啟動設定。",
            ),
            Phase::Cancelled => words.t(
                "Installation cancelled. No launch configuration was added.",
                "安装已取消，未添加启动配置。",
                "安裝已取消，未新增啟動設定。",
            ),
            Phase::Failed => words.t(
                "Installation failed. No launch configuration was added.",
                "安装失败，未添加启动配置。",
                "安裝失敗，未新增啟動設定。",
            ),
            Phase::Unknown => words.t(
                "Installation outcome is unknown. No automatic retry or adoption.",
                "安装结果未知，不会自动重试或采用配置。",
                "安裝結果未知，不會自動重試或採用設定。",
            ),
        };
        children.push(text(
            "status",
            label,
            if matches!(attempt.phase, Phase::Failed | Phase::Unknown) {
                Tone::Warning
            } else {
                Tone::Normal
            },
        ));
        if let Some(diagnostic) = &attempt.diagnostic {
            children.push(text(
                "diagnostic",
                clean(diagnostic).chars().take(512).collect::<String>(),
                Tone::Warning,
            ));
        }
        if attempt.active() {
            let id = format!("cancel-install-{}", attempt.id);
            actions.push(Action {
                enabled: attempt.phase == Phase::Installing,
                ..action(&id, words.t("Cancel installation", "取消安装", "取消安裝"))
            });
            children.push(button("cancel-install", id, Role::Normal));
        }
        if let Some(agent) = attempt.agent.filter(|_| attempt.phase == Phase::Installed) {
            children.push(code(
                "launch",
                view::build::clean(
                    &serde_json::to_string_pretty(&agent).expect("installed agent"),
                    true,
                ),
            ));
            let replace = configuration
                .agents
                .iter()
                .any(|existing| existing.id == agent.id);
            let id = format!("adopt-install-{}", attempt.id);
            actions.push(Action {
                confirm: Some(Confirm {
                    title: if replace {
                        words.t(
                            "Replace this agent's launch configuration?",
                            "替换此代理的启动配置？",
                            "替換此代理的啟動設定？",
                        )
                    } else {
                        words.t(
                            "Add this installed agent?",
                            "添加已安装的代理？",
                            "新增已安裝的代理？",
                        )
                    },
                    message: format!("{}\n{}", agent.id, clean(&agent.executable)),
                    destructive: replace,
                }),
                ..action(
                    &id,
                    if replace {
                        words.t("Replace configuration", "替换配置", "替換設定")
                    } else {
                        words.t("Add installed agent", "添加已安装代理", "新增已安裝代理")
                    },
                )
            });
            children.push(button("adopt", id, Role::Primary));
        }
    }
    View {
        version: VERSION,
        title: words.t(
            "Install Antigravity",
            "安装 Antigravity",
            "安裝 Antigravity",
        ),
        revision: stamp(configuration),
        fields: vec![],
        actions,
        root: column("root", children),
    }
}
