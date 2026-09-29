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
