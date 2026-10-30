# SSH 与串口 Shell 智能快捷键

本文件跟踪 SSH/串口人工终端交互，包括快捷键与本地搜索。约定见[终端快捷键](../architecture/terminal-shortcuts.md)。

日期：2026-10-30。

## 结果

人工终端在焦点与选区语境中分流常用快捷键，规则见[终端快捷键](../architecture/terminal-shortcuts.md)。有选区的 Ctrl+C、显式复制组合、粘贴、终端全选和 Esc 清除选区在本地处理；无选区的 Ctrl+C 和 Shell 行编辑、历史搜索、控制键继续交给终端。UART 发送框与普通表单保持原生快捷键。

两种插件复用 SDK 的纯前端模块，不新增依赖或 Tauri 权限、不改 MCP 或后端控制协议。粘贴保留原生 paste 事件及 xterm 的换行、bracketed paste 路径；复制失败提示而不发送中断。处理长按、keypress/keyup、焦点离开、卸载和异步复制失败，防止误发控制字符或复制其他编辑器的内容。

## 验证

- `pnpm build`：通过。
- `pnpm plugin:ssh:build`、`pnpm plugin:serial:build`：通过，两个开发包均更新。
- `pnpm test`：46 项通过，无失败、无跳过；包含 8 项快捷键单元回归、SSH/串口各 1 项实际插件构建浏览器回归，以及原有人工 Shell/UART 回归。
- 浏览器回归验证选区复制、无选区中断、Shell 控制键、单次粘贴与 bracketed paste、焦点在其他输入框、复制失败及中文提示、卸载回收。

本轮未修改 Rust。浏览器测试隔离剪贴板写入并注入固定 paste 数据，不读写用户系统剪贴板；未重做真实 Tauri 系统剪贴板、macOS/Linux 真机、SSH 服务器或串口硬件验收。

## 2026-10-30 Ctrl+F 本地搜索

- SSH 与串口 Shell 的 Ctrl+F 打开本地搜索条，macOS Cmd+F 使用相同分流。
- 支持中文、不区分大小写的纯文本匹配、高亮/计数、Enter/Shift+Enter 和按钮循环跳转；新输出更新匹配。
- Esc 或关闭按钮清理高亮并返回终端；切换到 UART 时关闭 Shell 搜索，普通输入框不受影响。
- 查询仅保留在当前前端挂载内存，最多 512 字符，100 ms 防抖，最多高亮 1000 项。只搜索现有终端缓冲，不查询远端文件或执行命令。
- 公共搜索条通过 SDK `components/terminal-search` 子路径复用；两个插件使用纯 JavaScript `@xterm/addon-search ~0.15.0`，无原生依赖、外部进程或 Tauri 权限变更。

验证：桌面构建、SSH/串口插件构建和 `pnpm test`（49 项）通过；浏览器确认搜索输入不进入 `ssh.input`/`serial.write`，覆盖中文、字面量、无匹配、计数更新、跳转、关闭恢复焦点及 UART 视图切换。未重新连接 SSH 服务器或写串口硬件，macOS/Linux 真机待验。
