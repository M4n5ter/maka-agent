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

将平台包、收据、源码包及其 SHA-512 文件收集到同一目录。`just release-prepare native-preview` 从已验证源码打包启动器并验证完整集合。`just publish native-preview` 先发布原生平台包，再发布启动器，使用 `rust-preview` npm 标签。

安装脚本不会下载或执行未经验证的辅助程序。运行时行为由 `crates/cli` 实现。
