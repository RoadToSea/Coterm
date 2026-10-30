# 终端快捷键

SSH 与串口 Shell 使用同一套快捷键规则。判断依据是**终端是否获得键盘输入、终端是否有文本选区以及修饰键组合**，不是根据命令文本猜测用户意图。焦点位于 UART 发送框、配置表单、审批编辑器或其他输入框时，保留该输入框的原生快捷键。

## 行为

| 快捷键 | 终端中的行为 |
|---|---|
| Ctrl+C | 有选区时复制；无选区时原样发送 Ctrl+C，由设备或远端程序决定如何中断 |
| Ctrl+Shift+C / Ctrl+Insert | 复制选区，无选区时不发送控制字符 |
| Ctrl+V / Ctrl+Shift+V / Shift+Insert | 通过浏览器原生 paste 事件粘贴，由 xterm 处理换行与 bracketed paste，再进入原有人工输入队列 |
| Ctrl+Shift+A | 选中终端全部缓冲内容；不发送命令 |
| Esc | 有选区时只清除选区；无选区时原样发送 Esc |
| Ctrl+A/E、Ctrl+R、Ctrl+D、Ctrl+L、Ctrl+U/W/Z | 保留 Shell 控制键语义，不改成页面全选、搜索、关闭、清屏或撤销 |
| Enter、Tab、方向键、Home/End、退格 | 沿用 xterm 的终端输入行为 |
| Shift+PageUp/PageDown | 沿用 xterm 的本地滚动行为 |
| macOS Cmd+C / Cmd+V / Cmd+A | 分别复制选区、原生粘贴与终端全选；Ctrl 组合仍按终端规则处理 |

复制成功后保留选区，可用 Esc 或重新选择清除；要中断程序，先清除选区再按 Ctrl+C。复制失败显示本地提示，不能退化成向远端发送 Ctrl+C。按住本地快捷键、后续 keypress/keyup、焦点切换均不能误发远端控制字符。IME 组合输入及 Alt/AltGr 不进入本地快捷键分支。

粘贴不读取系统剪贴板 API，不新增 Tauri 剪贴板权限。复制先尝试浏览器 Clipboard API，不可用时仅在原终端仍有焦点、选区未改变的情况下回退到 xterm 的原生 copy 事件。剪贴板内容不进入日志、普通事件或 MCP 结果。粘贴后的人工输入仍受原有连接、队列容量、敏感输入和 MCP 在途列表互斥规则约束。

## 实现与验证

公共实现位于 [`terminal-shortcuts.ts`](../../packages/plugin-sdk/src/components/terminal-shortcuts.ts)，由 SDK 子路径 `@coterm/plugin-sdk/components/terminal-shortcuts` 导出。它只依赖终端的选择和键盘接口，不引入 xterm 依赖、不监听全局键盘，也不改变宿主通信协议。SSH 和串口前端在挂载时安装，卸载时回收。

```bash
pnpm --filter coterm-plugin-ssh-ui build
pnpm --filter coterm-plugin-serial-ui build
node --import ./scripts/test-ts-loader.mjs --test tests/terminal-shortcuts.test.mjs tests/terminal-shortcuts-browser.test.mjs tests/serial-terminal-browser.test.mjs
```

单元测试验证快捷键分流、IME/AltGr、长按、焦点转移、复制失败及卸载。浏览器回归使用两种插件的实际构建与 xterm，验证选区复制、无选区 Ctrl+C、Shell 控制字符、粘贴一次投递及 bracketed paste、其他输入框不受影响。浏览器测试替换剪贴板写入并注入 paste 数据，不读写用户的系统剪贴板，不代表真实桌面剪贴板或 macOS/Linux 真机验收。
