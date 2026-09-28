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

# Maka npm 启动器

[English](README.md)

本包启动当前操作系统和 CPU 对应的原生 Maka CLI，通过精确版本的可选依赖分发可执行文件。命令参数、标准输入输出和退出状态直接传递给原生进程。

预览分发支持 macOS arm64、使用 glibc 的 Linux x64 和 Windows x64。WSL 使用 Linux 可执行文件，并由原生分发层获取匹配的 Windows 辅助程序。

## 构建与发布

通过 `just source <version>` 从已提交代码生成源码候选包。各平台分别运行：

```sh
just package <source.tar.gz> <target> <build-id> native-preview
```

打包命令会在输出目录写入平台包及 `<target>.json` 回执。将三个平台的产物、源码包及其 SHA-512 文件收集到同一目录后：

```sh
just release-prepare native-preview
just release-check native-preview
just publish native-preview
```

`release-prepare` 从同一源码包生成启动器并校验四个包；`release-check` 在当前平台离线安装启动器和原生包，实际运行 CLI/V8。三个平台均应完成验收。`publish` 先发布 Windows 辅助组件，再发布其余原生包，最后发布启动器，并确认各包的 `rust-preview` 标签。中断后重试同一份产物；不要重新构建同一版本。一个发布通道同一时刻只允许一个发布者，npm 标签没有条件写入接口。

GitHub 使用已登记的 `npm-publication.yml`。在功能分支可运行 `gh workflow run npm-publication.yml --ref <branch> -f publish=false`，得到完整产物与三平台验收结果。实际发布须显式选择 `publish=true`，且仅在 `main`、现有 `npm-publication` 环境中执行。

正式打包会从固定的上游提交构建 V8，关闭其可选的 LGPL glibc 数学实现，再使用匹配的静态库和 Rust bindings。需要 Python 3、Git、可用的 Clang/libclang（19 或更新）及原生 C++ 工具链。macOS 构建需要 Xcode 26 或更新版本，以编译 Computer Use 的 Swift 绑定；CI 使用 macOS 26 构建，并在 macOS 15 验证安装。Linux 还需要 glib 开发包和 cargo-zigbuild/Zig。首次构建会下载 Chromium 工具链，可能耗时超过一小时，请预留数十 GiB 临时空间。构建完成后临时目录自动清理。

完成的 V8 静态库、bindings 和声明保存在 `target/v8`（或 `$CARGO_TARGET_DIR/v8`）。缓存未命中时，打包程序下载 `scripts/rust/v8-prebuilts.json` 中固定的匹配产物；只有缺少对应条目才源码编译。key 绑定 V8 源码、平台、功能和生产代码，不绑定 Maka/deno_core 版本或下载工具。下载必须匹配源码内固定的 SHA-256，每次使用还会校验包内文件摘要；下载异常或缓存损坏会明确报错。每次打包仍执行实际链接和原生冒烟测试。

对于尚未固定的 key，原生 CI 导出 `v8-prebuilt-*` 产物，包含压缩包、摘要和完成验证的提交及运行记录。完整工作流通过后，将同一批压缩包发布为不可覆盖的 GitHub 依赖预构建资产，并把 URL、摘要和验证记录提交到 `v8-prebuilts.json`。不要覆盖既有资产或改变同一 key 的含义。Actions 缓存加速尚未发布的构建，源码内的清单负责认证下载内容。缓存损坏时删除对应的本地条目及 Actions 缓存后重试；单独删除本地缓存通常会重新下载已固定的预构建。

依赖变化后运行 `node scripts/rust/notices.mjs generate` 并审查许可证快照。它从固定依赖版本收集全文和嵌套声明，少数上游缺少独立许可文件的例外记录在 `notices-sources.json`。发布校验快照与源码输入的绑定，并追加实际 V8 构建图和 Rust 标准库声明；许可证 TSV 仅用作清单，不能代替全文。

安装脚本不会下载或执行未经验证的辅助程序。运行时行为由 `crates/cli` 实现。
