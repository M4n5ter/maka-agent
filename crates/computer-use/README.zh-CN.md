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

# Computer Use

Maka 提供独立的 `cua_repl` 和 `cua_reset`，不依赖 Code Mode，也不进入其嵌套工具目录。
REPL 支持顶层 await、跨调用/Turn 的变量、绑定的 App/Tab 对象、自动初始观察及图片输出。
它没有 Node、任意文件系统/网络 API 或网络 inspector 端口。

Host 持有唯一 Cua 驱动；每个 Session 分别拥有 JS 环境、Cua 会话、目标句柄和观察。
每次操作重新校验当前 Run 权限，经过 Host 审批与独立结算。对象不保存授权。
取消/超时后须重置；已经发出的原生操作仍会等待结算，不自动重放。
Session 退役与 Host 退出释放资源，重置不会关闭用户的应用或标签页。

macOS 语义操作复用 Cua 的 AX 遍历与窗口校验，并保留真实窗口/元素引用；支持精确文本
范围选择和元素公开的辅助动作。粘贴保留剪贴板格式，支持 UTF-8 HTML，并在可观察变化
后恢复；期间用户的新复制不会被覆盖。粘贴会临时激活精确目标并恢复其焦点/选区。

浏览器通过插件配置中的已有本机 CDP 提供端接入，例如：

```json
{"browsers":[{"id":"chrome","endpoint":"http://127.0.0.1:9222"}]}
```

连接保留已有 profile，不自动开启调试、重启浏览器或切换 profile。支持标签选择/创建、
AX/截图、输入、值编辑、精确文本选择、HTML 粘贴、导航与关闭。浏览器重启、导航或
标签关闭会使旧引用失效；截图坐标也不能跨观察或几何变化复用。

完整方法与平台差异见 [API reference](src/api.md)。当前尚不支持客户端 tab mention、
extension/IAB 提供端、visibility/sessionName、交付 UI 标记或 CDP 辅助 AX 动作。
跨进程 iframe 不宣称完整覆盖；原生像素滚动暂按页执行；Markdown 粘贴作为文本。
原生文本选择仅 macOS；Linux/Windows 尚未在真实桌面验收。

内置 `maka-cua` skill 可从原生 Skill library 安装，不自动写入用户库。
不包含视觉识别扩展、模型下载、Python 或 Desktop UI 接入。

测试包括独立 REPL/权限/取消/资源边界，以及需主动运行的真实 Chrome、AppKit 表单
验收。验证成功的范围与完整 cua_repl 等价、性能更优是不同结论，后两者尚未宣称。
构建、配置、测试边界与许可说明见 [English README](README.md)。
