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

# Responses

[English](README.md)

原生 Rust Responses 编码、流解码及可丢弃的 WebSocket 续接状态，不依赖 V8。凭据、请求准入、规范历史与用量结算由 Host 管理。

设计参考 [OpenAI Codex](https://github.com/openai/codex/tree/94174e44cbc54cece45f6052328ca0c2cd7a8a2a/codex-rs/codex-api)。HTTP 与 WebSocket 共用事件解码器；连接缓存仅作优化，不构成重放权威。

模型出站请求不设置固定的本地 body 字节上限，请求有效性由上下文选择和服务商限制决定；入站响应和事件仍有独立的资源边界。请求超过 WebSocket 缓存预算时仍可发送，只是不保留续接缓存。较大的 JavaScript 适配器请求会独占输入队列窗口，而不是被拒绝。图片和音频按单个资源校验、读取，历史图片不会通过累计字节配额挤掉新图片。

原生 Responses 声明提供商确认的单项完成边界。Host 为这些请求启用逐项接纳：完整且验证通过的输出先以 `ModelObserved` 持久化，本地工具随后即可执行，模型流继续接收。工具执行有并发上限，独占整步的接纳判定仍按到达顺序进行。Code Mode 复用已有的 cell 生命周期与资源边界，嵌套工具共享执行门禁。没有可靠单项完成信号的适配器继续等待完整响应，避免将 SDK 在断流清理时合成的结束事件当作执行依据；旧日志保持原有语义。

WebSocket 接收中断由适配器提供重试安全证据，再交给 Agent 唯一的有界重试流程。没有已接纳进度时，重试保持冻结输入；存在已接纳进度且适配器允许保留输出后继续时，Host 等待已调度工具结束，使用已提交的输出项及工具结果重建历史，并保留原先捕获的模型和工具能力。参数片段不会执行。未知提供商侧效果、无效帧和发送结果不明确时不授权重试；完整的不透明元数据会保留而非丢弃。关闭状态与底层错误保留在诊断中。

执行与恢复职责参考 [Codex 的完整项调度](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/stream_events_utils.rs)、[工具调度器](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/tools/parallel.rs)和[流重试决策](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/responses_retry.rs)。先按指数退避重连 WS；九次重试耗尽后再切换 HTTP 并重置有界预算。retry-after 与取消仍生效，回退后保留现有的五分钟路由冷却。发送前的升级协商仍有独立的有界尝试，此时尚未发送模型请求。明确的策略或协议关闭码仍为终止错误；大小关闭码（1009）允许在重试耗尽后通过 HTTP 恢复。
