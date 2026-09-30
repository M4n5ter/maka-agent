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

# Maka TUI

[English](README.md)

Runtime Host 的终端客户端，通过 `maka tui` 启动。

侧栏按工作区组织会话，**待处理**直接筛选目录，保留当前对话。窄窗口或专注模式下，`Ctrl+B` 通过浮层打开导航，`Esc` 返回原草稿。Tab 跨过列表和字段，方向键在列表内移动；`F1` 在当前页面上打开帮助，Host 连接详情位于设置中。

`Enter` 发送消息，模型工作时排入下一轮。支持增强键盘协议的终端用 `Shift+Enter` 换行；传统终端（包括通过 WSL 使用的 Windows Terminal）可用 `Ctrl+J`。粘贴多行文字不会自动发送。`Ctrl+K` 打开命令，`Ctrl+F` 查找对话，`Ctrl+N` 新建会话；弹层和搜索优先处理各自的按键。

原生和插件控件统一用底色表示当前值或已打开项，用下划线表示键盘焦点，悬停不会
让另一行看起来被选中。需要审批或回答的会话会在侧栏显示「待处理」；composer
上方同时提供请求入口，点击或按 `Ctrl+G` 进入审阅，不会自动批准。向上阅读历史
时，「回到底部」也显示在 composer 上方。

Read 工具行显示紧凑资源地址，展开参数保留原始输入。`Read.path` 除完整地址外，
支持 `task:<ID前缀>`、`attachment:<ID前缀>` 和 `archive:<事件ID前缀>`。
建议保留 `attachment-` 等固定 ID 前缀，再取后面 12 个字符；精确 ID 优先，
否则必须在当前会话可读资源中唯一匹配，有歧义时补长。后续分页完整传回 `next`。

Composer 支持 `Ctrl+V` 或「+ → 从剪贴板粘贴」，读取 macOS、Windows、Linux
本机剪贴板中的图片、复制的文件或文字。终端传入的完整绝对路径或本地 `file://`
URL 会转为附件，支持引号、转义空格和多文件；其他文字保留在草稿中。
文件沿用附件校验与上传队列，无效路径、目录、超限文件会显示具体附件错误。
剪贴板图片随本地草稿保存，移除并成功保存草稿后回收。`Cmd+V` / `Ctrl+Shift+V`
由终端处理；终端不转发图片数据时使用 `Ctrl+V`。SSH 下读取的是 TUI 所在主机的剪贴板。

跨屏选择正文时，单击起点，滚动后用 `Alt+点击` 选择终点（macOS 使用 Option）。
普通点击清除选区，`Shift+方向键`、`Shift+PageUp/PageDown` 也可扩选。
终端转发 Shift 鼠标事件时，`Shift+点击` 使用同一套选区。Maka 在 Unix 下运行时
通过 `XTSHIFTESCAPE` 请求转发，退出时释放；部分终端仍会保留 Shift 原生选区，
这种选区可能包含侧栏，Maka 无法清除。`Alt+点击` 可避开该 Shift 策略。
Otty 1.5.4 可额外设置 `mouse-shift-to-select = false` 来转发 Shift；这是可选的
终端全局偏好，使用 `Alt+点击` 不需要它。

侧栏的「搜索」打开命令面板，按名称、工作区、ID 和目录展示的标签扫描 Host 的分页会话目录；宽泛搜索可选择「更多匹配会话」继续。模型选择器可跨目录分页搜索已启用的聊天模型。

Host 尚无模型连接或默认模型时，工作区首页会显示相应的设置入口；此时 `Ctrl+N` 进入缺失的设置步骤。现有会话仍可打开。

Chat 与插件 Transcript 共用 Markdown、选区、搜索和流式呈现。新增正文通过短暂明暗渐变显现，不延迟已收到的内容，也不改变排版；减少动态效果和终端默认配色下直接显示。

设置 → 插件可管理本地包、实例、配置与服务绑定。修改提交前需要审查，激活状态自动更新；配置草稿仅保存在内存中，结果不确定时不自动重试。

会话的**技能库**支持导入本地 Markdown、安装来源、审查受管更新及确认目录删除。WorkHub 可配置新任务、修复失效模型，并在协调会话中直接发送消息；其它受管会话须由所属行为明确开放原生输入。Rust 与 JavaScript 插件通过同一套公开组件提供页面、设置、面板与嵌套视图。

- `maka-client` 负责传输和协议校验；TUI 负责呈现和输入。
- 提供商设置使用公开的描述、配置与认证契约。
- 本地状态保存草稿和恢复身份，不保存认证输入。写入结果不确定时查询事实，不自动重放。
- Fluent 文案覆盖英文、简体中文和繁体中文。
