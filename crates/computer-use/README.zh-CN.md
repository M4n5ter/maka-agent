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

每个 Session 拥有独立的 Maka M/四角星合成光标，支持六种配色、标签、显示开关和减弱动画。
输入动作自动显示反馈；`app.moveCursor(indexOrPoint)` / `tab.moveCursor(indexOrPoint)`
只指示当前观察中的位置，不移动系统鼠标，也不消耗截图坐标。
`cua.cursor.configure({label, color, enabled, reducedMotion})` 配置当前光标，
`cua.cursor.getState()` 查看原生和浏览器的显示状态。

原生覆盖层运行在同一 CLI/service 可执行文件的私有显示子进程中，只接收有界绘制命令；
macOS 在子进程主线程运行 AppKit。浏览器使用隔离 world 内不参与布局、命中测试和 AX
的装饰层，空闲后自动删除。Session/reset 清理自己的光标，Host 退出回收显示进程。
原生 macOS 当前沿用上游主屏渲染；Windows 已在真实交互桌面验收，X11 在独立 Xvfb
显示器中验证，未启用 Wayland 覆盖层。多个图案不会隔离操作系统键盘、焦点或输入法。

Linux 构建需要 X11、Xi、Xtst 开发库；Windows MSVC 构建还需要对应的 Spectre CRT 库。
当前固定 [M4n5ter 的 Cua fork](https://github.com/M4n5ter/cua) 提交
`cf1e2ceafd8e8bbf23fad10b713159123ffba018`，修复 X11 跨分块重复标签和字形接缝
（[上游 PR](https://github.com/trycua/cua/pull/4260)）。
回归覆盖标签重叠与屏幕边缘；有／无 compositor 的原生 Xvfb 验证两个标签和独立清理。
这些定向检查不替代 Cua 的完整桌面认证矩阵。

测试包括独立 REPL/权限/取消/资源边界，以及需主动运行的真实 Chrome、AppKit 表单
验收。验证成功的范围与完整 cua_repl 等价、性能更优是不同结论，后两者尚未宣称。
表单编辑验收会短暂切换键盘焦点，需要在用户没有打字时运行；
`native_observation_recovers` 验收始终在后台运行，不发送键盘输入，并检查应用忙后恢复、
旧观察失效、外部编辑保留和窗口关闭后的拒绝。
构建、配置、测试边界与许可说明见 [English README](README.md)。
