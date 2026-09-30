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

# Maka 插件 SDK

[English](README.md)

Rust Runtime Host 的公开 TypeScript 契约。当前为 **Host SDK 3**、
**终端 View 9**，与 Maka 应用版本及 npm workspace 版本分别演进。
此 workspace 仍为私有包，不应假设能从 npm 仓库安装。

插件开发文档统一维护在
[Maka 插件编写 skill](../../skills/maka-plugin-authoring/SKILL.md) 中。
它随 Maka 内置分发，包含可执行模板和完整 SDK 源码契约。
每个 Profile 会自动安装，启动时将完整目录同步到随 Maka 提供的版本，清除过时资源。
启用、禁用和固定设置会保留；自定义内容请另建技能保存。
可用 `/skill:maka-plugin-authoring`，也可由模型按任务发现。
`Skill` 按需读取附带资源，无需向普通文件工具开放 Host 私有目录。

先读[快速开始](../../skills/maka-plugin-authoring/references/quickstart.md)，
使用[可运行插件模板](../../skills/maka-plugin-authoring/assets/starter/)。
[API 索引](../../skills/maka-plugin-authoring/references/api-map.md) 对应完整
[SDK 源码](src/host.ts)；专题指南分别说明工具与输入、权限与状态、终端界面、
模型与执行器、原生 Rust 插件，以及测试和交付。

```sh
npm --workspace @maka-agent/plugin-sdk run build
npm --workspace @maka-agent/plugin-sdk run typecheck
node --test packages/plugin-sdk/tests/*.test.mjs
```

Runtime/UI 入口必须是无未解析 import、无顶层 await 的独立 ESM。
开发时使用 SDK 类型导入，运行时通过实际调用上下文的 Host 能力执行操作。
Manifest、生命周期和实例激活方法见统一指南。
