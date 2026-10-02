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

# 贡献 Maka

[English](CONTRIBUTING.md)

贡献者对代码的正确性、来源和许可负责。最终评审与合并由人类决定；AI 评审不能替代独立的人类评审。实质性 AI 编写的提交需要保留 `Generated-by: <tool>` trailer。

分支、提交和 PR 标题使用 Conventional Commits。向 `main` 合并需要其他 committer 的批准和通过的 `test` 检查。公开讨论与发布流程遵循[英文贡献指南](CONTRIBUTING.md)。

## 开发

需要 Rust stable、Node.js 24 LTS、npm 11.19.0、just、cargo-nextest 和 Python 3。许可证清单检查还需要 cargo-deny。

常用任务统一使用 [just](https://github.com/casey/just)。推荐通过包管理器安装（macOS：`brew install just`），也可运行 `cargo install --locked just`。Windows 还需将 Git for Windows 的 `sh` 加入 `PATH`。

```sh
just setup                 # 构建时使用的 JavaScript 依赖
just run                   # 原生 TUI
just run --help            # CLI 命令
just check                 # 格式、Clippy、SDK、脚本和 Rust 测试
```

通过 `just test -p maka-runtime-host` 运行单个 crate 的测试。向 `just build` 传递 Cargo 参数选择目标和构建模式，复用现有缓存，避免重复构建。

日常开发使用 Cargo 的 V8 预构建。`just package` 复用我们固定的、不含 LGPL glibc 数学实现的 V8 产物；只有缺少匹配预构建和本地缓存时才源码编译。构建要求见 [CLI 打包说明](packages/cli/README.zh-CN.md)。

官网独立管理依赖：先运行 `just website-setup`，再使用 `just website-dev` 或 `just website-check`。官网依赖更新写入 `website/package-lock.json`，构建工具和插件依赖使用根锁文件。

## 代码约定

- 使用 `name.rs` 配合 `name/`，不使用 `mod.rs`。单元测试放在实现文件末尾，集成测试放在 `tests/`。
- 测试持久行为和重要失败边界，优先扩展已有测试，避免重复夹具、源码文本断言和固定延时。
- 业务插件只使用公共能力；Host 管理准入、事实和资源结算，插件管理领域策略。
- 用类型表达闭合状态，通过 `schemars` 生成 schema。数据库结构变更放入 SQLx migration。
- 协议字段与本地化文案分离，保留完整的 `en`、`zh-CN`、`zh-TW` locale。
- 运行 `just check`，保持中英文 crate 文档一致。

架构见 [ARCHITECTURE.zh-CN.md](ARCHITECTURE.zh-CN.md)。
