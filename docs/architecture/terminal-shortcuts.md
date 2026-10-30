# 终端快捷键

SSH 与串口 Shell 使用同一套快捷键规则。判断依据是**终端是否获得键盘输入、终端是否有文本选区以及修饰键组合**，不是根据命令文本猜测用户意图。焦点位于 UART 发送框、配置表单、审批编辑器或其他输入框时，保留该输入框的原生快捷键。

## 行为

| 快捷键 | 终端中的行为 |
|---|---|
| Ctrl+C | 有选区时复制；无选区时原样发送 Ctrl+C，由设备或远端程序决定如何中断 |
| Ctrl+Shift+C / Ctrl+Insert | 复制选区，无选区时不发送控制字符 |
| Ctrl+V / Ctrl+Shift+V / Shift+Insert | 通过浏览器原生 paste 事件粘贴，由 xterm 处理换行与 bracketed paste，再进入原有人工输入队列 |
| Ctrl+Shift+A | 选中终端全部缓冲内容；不发送命令 |
| Ctrl+F / macOS Cmd+F | 打开当前终端缓冲的本地搜索，不发送 Ctrl+F 控制字符 |
| 搜索框 Enter / Shift+Enter | 下一处 / 上一处，首尾循环跳转 |
| Esc | 搜索框获得焦点时关闭搜索并回到终端；终端有选区时只清除选区；否则原样发送 Esc |
| Ctrl+A/E、Ctrl+R、Ctrl+D、Ctrl+L、Ctrl+U/W/Z | 保留 Shell 控制键语义，不改成页面全选、搜索、关闭、清屏或撤销 |
| Enter、Tab、方向键、Home/End、退格 | 沿用 xterm 的终端输入行为 |
| Shift+PageUp/PageDown | 沿用 xterm 的本地滚动行为 |
| macOS Cmd+C / Cmd+V / Cmd+A | 分别复制选区、原生粘贴与终端全选；Ctrl 组合仍按终端规则处理 |

复制成功后保留选区，可用 Esc 或重新选择清除；要中断程序，先清除选区再按 Ctrl+C。复制失败显示本地提示，不能退化成向远端发送 Ctrl+C。按住本地快捷键、后续 keypress/keyup、焦点切换均不能误发远端控制字符。IME 组合输入及 Alt/AltGr 不进入本地快捷键分支。

粘贴不读取系统剪贴板 API，不新增 Tauri 剪贴板权限。复制先尝试浏览器 Clipboard API，不可用时仅在原终端仍有焦点、选区未改变的情况下回退到 xterm 的原生 copy 事件。剪贴板内容不进入日志、普通事件或 MCP 结果。粘贴后的人工输入仍受原有连接、队列容量、敏感输入和 MCP 在途列表互斥规则约束。

## 终端搜索

Ctrl+F 搜索的是该实例当前终端保留的输出（含现有 3000 行滚动缓冲），不是远端文件、设备命令或整份历史记录。使用不区分大小写的纯文本匹配，不启用正则表达式；支持中文、软换行、首尾循环及匹配位置计数。输入最多 512 个字符，输入搜索防抖 100 ms，最多高亮 1000 处；达到上限时计数标记 `1000+`，而不是宣称完整统计了所有结果。新输出到达时更新匹配。

打开搜索会聚焦输入框；有单行选区且尚无查询时，以选区作为初始查询。搜索框内再次 Ctrl+F 选中查询，Enter/Shift+Enter 或按钮跳转；Esc 或关闭按钮清除搜索高亮与选区并回到终端。当前挂载期间保留查询，卸载时清理控件、定时器和事件订阅，不持久化查询。串口切到 UART 模式时隐藏 Shell 搜索，保留发送框的原生快捷键。焦点在其他输入框时，不劫持其 Ctrl+F。

搜索框不会向 `ssh.input` 或 `serial.write` 提交查询和导航按键。搜索条改变终端尺寸时仍可能触发既有 SSH PTY resize；这不是执行远端命令。

## 实现与验证

公共实现位于 [`terminal-shortcuts.ts`](../../packages/plugin-sdk/src/components/terminal-shortcuts.ts)，由 SDK 子路径 `@coterm/plugin-sdk/components/terminal-shortcuts` 导出。它只依赖终端的选择和键盘接口，不引入 xterm 依赖、不监听全局键盘，也不改变宿主通信协议。SSH 和串口前端在挂载时安装，卸载时回收。

搜索条位于 [`terminal-search.ts`](../../packages/plugin-sdk/src/components/terminal-search.ts)，由 `@coterm/plugin-sdk/components/terminal-search` 导出。插件前端各自加载 `@xterm/addon-search ~0.15.0`，兼容现有 xterm 5.5；SDK 只接收搜索适配接口。该依赖为纯浏览器 JavaScript，无原生进程、OS IO 或 Tauri 权限变化；通过 `allowProposedApi` 开启的是 xterm 渲染器的高亮装饰接口，不是宿主权限。Windows 浏览器构建与回归已覆盖，macOS/Linux 真机仍需独立验收。

```bash
pnpm --filter coterm-plugin-ssh-ui build
pnpm --filter coterm-plugin-serial-ui build
node --import ./scripts/test-ts-loader.mjs --test tests/terminal-shortcuts.test.mjs tests/terminal-shortcuts-browser.test.mjs tests/serial-terminal-browser.test.mjs
```

单元测试验证快捷键分流、IME/AltGr、长按、焦点转移、复制失败及卸载。浏览器回归使用两种插件的实际构建与 xterm，验证选区复制、无选区 Ctrl+C、Shell 控制字符、粘贴一次投递及 bracketed paste、其他输入框不受影响。浏览器测试替换剪贴板写入并注入 paste 数据，不读写用户的系统剪贴板，不代表真实桌面剪贴板或 macOS/Linux 真机验收。
