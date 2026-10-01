<!--
  Licensed to the Apache Software Foundation (ASF) under one
  or more contributor license agreements.  See the NOTICE file
  distributed with this work for additional information
  regarding copyright ownership.  The ASF licenses this file
  to you under the Apache License, Version 2.0 (the
  "License"); you may not use this file except in compliance
  with the License.  You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

  Unless required by applicable law or agreed to in writing,
  software distributed under the License is distributed on an
  "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
  KIND, either express or implied.  See the License for the
  specific language governing permissions and limitations
  under the License.
-->

# 架构

[English](ARCHITECTURE.md)

Maka 只有一个 Rust Runtime Host。CLI 和 TUI 通过公共客户端协议连接，执行和持久状态统一归 Host 管理。

- **准入与结算：** `crates/runtime-host` 接纳操作、捕获执行边界并协调资源清理；`crates/event-log` 管理持久执行事实与状态根所有权。
- **Agent 执行：** `crates/agent`、`crates/runtime` 和 `crates/model` 构建模型请求并结算工具效果。`crates/providers` 管理提供商认证、模型发现和策略，`crates/responses` 实现原生 Responses 传输。
- **能力边界：** 业务插件通过 `crates/plugins` 访问存储、模型、执行、凭据、HTTP 和呈现能力。Host 管理权限和生命周期，插件管理领域策略。
- **JavaScript：** `crates/js-runtime` 内嵌 V8，承载 Code Mode 和 JavaScript 插件。提供商 SDK 在构建时打包，Guest 代码通过明确绑定访问 Host 能力。
- **Computer Use：** `crates/computer-use` 拥有独立的 REPL 与会话生命周期，独立于 Code Mode，支持原生应用和浏览器。
- **呈现：** `crates/client` 实现客户端协议，`crates/tui` 渲染原生终端视图。插件 SDK 描述能力与声明式视图，不持有 Host 状态。

TUI 是客户端外壳，负责导航、渲染和本地草稿。业务页面、设置、命令、输入选择器和工作区启动入口均由插件贡献，并随插件激活生命周期撤销入口和动作权限。执行器插件可以发布自己的视图，或使用 `maka_plugins::executor::terminal` 提供的标准表单；规范会话的创建和配置仍由 Host 授权。助手偏好和会话回顾也使用相同的公共 Remote 边界：捕获的 `views` 提供观察，`controls` 根据发起客户端的权限接纳规范写入。提供方的显示名称和连接分类由其描述符声明。

`packages/cli` 的 npm 包只负责选择并启动匹配的可执行文件，不实现运行时、配置存储或工具。

详细契约见各 crate 的 README；JavaScript 插件开发见 [SDK](packages/plugin-sdk/README.zh-CN.md)。
