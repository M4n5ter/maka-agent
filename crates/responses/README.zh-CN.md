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

WebSocket 接收中断由共享解码器判断重放安全性，再交给 Agent 现有的有界重试流程；失效连接被丢弃，使用同一份冻结的模型输入，按指数退避重建 WebSocket；只有九次流重试全部失败后，Agent 才选择 HTTP 回退并重置有界重试预算。服务端的 retry-after 建议和取消仍然生效。无效帧、发送结果不明确、取消，以及已观察到提供商侧工具执行或不透明的重放元数据时，不授权重放。关闭状态与底层传输错误保留在诊断中。

恢复职责参考 [Codex 的流重试决策](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/responses_retry.rs)与[传输重置](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/client.rs)：传输层报告失败，Agent 决定重放。Maka 先重试 WebSocket，再切换传输；选择回退后保留现有的五分钟有界路由冷却。发送前的升级协商保留独立的有界重试，此时尚未发送模型请求。明确的策略或协议关闭码仍为终止错误；大小关闭码（1009）允许在预算耗尽后通过 HTTP 恢复。Codex 逐项接受输出；Maka 则在整个模型响应完成后才接受本地工具调用，因此重试仍使用冻结的历史切面，不执行未确认的工具调用。不透明的重放元数据继续遵守现有保守边界。
