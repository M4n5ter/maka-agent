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

# 模型提供商

[English](README.md)

通过公共 `maka-plugins::provider` 契约实现随应用分发的模型提供商。提供商负责认证交换、模型发现和请求策略；Host 负责连接、凭据、代理路由与持久结算。

API 提供商拥有随包模型事实、认证、有界模型发现和协议策略。ChatGPT 提供商复用原生 Responses 适配器；提供商认证和发现均不需要 V8。发现可用不代表所有推理协议都已支持。

ChatGPT 订阅模型的最大输出 token 数（`maxOutputTokens`）与上下文窗口（`contextWindow`）是独立字段，例如 [GPT-6 Luna 的最大输出为 128,000 tokens](https://developers.openai.com/api/docs/models/gpt-6-luna)。Host 使用最大输出 token 数计算上下文预算。订阅接口不支持通过请求自定义输出上限：`max_output_tokens`、`max_completion_tokens` 和 `max_tokens` 都会被拒绝。订阅适配器与官方 Codex 的 [HTTP/WebSocket 请求结构](https://github.com/openai/codex/blob/d515b2f85ec1b24a4b5ec3fbd86db27fd51aea3b/codex-rs/codex-api/src/common.rs#L278) 一致，省略该请求字段；普通 Responses API 仍保留输出限制。上下文窗口以账号模型目录为准，已知模型缺失的最大输出 token 数由随包模型事实补齐。提供商插件负责生成可安全展示的登录错误说明，Host 持久化，客户端展示，不解释提供商专属错误码。

Cargo 直接嵌入已提交的模型和计费数据，无需 Node。数据变更须结合提供商测试审查；元数据更新不等于实现新的推理协议。
