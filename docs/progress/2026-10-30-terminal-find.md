# SSH 与串口终端 Ctrl+F 搜索

日期：2026-10-30。

## 完成

- SSH 与串口 Shell 的 Ctrl+F 打开本地搜索条；macOS Cmd+F 使用同一分流。
- 支持中文、不区分大小写的纯文本匹配、匹配高亮与计数、Enter/Shift+Enter 和按钮循环跳转、新输出更新匹配。
- Esc/关闭按钮清理高亮并回到终端；串口切换 UART 时关闭 Shell 搜索，普通输入框不受影响。
- 查询只保留在当前挂载的前端内存，不进入后端、MCP 或持久化。搜索仅覆盖现有终端缓冲，不查询远端文件或执行命令。
- 输入最多 512 字符、防抖 100 ms，最多高亮 1000 项；达到上限标记 `1000+`。

公共搜索条通过 SDK 的 `components/terminal-search` 子路径复用。两个插件前端新增纯 JavaScript `@xterm/addon-search ~0.15.0`，其 peer 范围兼容现有 xterm 5.5；没有原生依赖、外部进程或 Tauri 权限变更。完整约定见[终端快捷键](../architecture/terminal-shortcuts.md)。

## 验证

- `pnpm build`：通过。
- `pnpm plugin:ssh:build`、`pnpm plugin:serial:build`：通过，开发包已更新。
- `pnpm test`：49 项通过，无失败、无跳过；包含 Ctrl+F 分流单元测试和两个插件实际构建的搜索浏览器回归。
- 浏览器确认搜索查询、Enter/Shift+Enter、关闭操作不进入 `ssh.input`/`serial.write`；覆盖前后跳转、循环、中文、字面量 `.*`、无匹配、中文标签、新输出匹配计数、关闭恢复焦点与 UART 视图切换。

未修改 Rust 后端。验收使用 Windows 无头浏览器和宿主接口替身，没有重新连接 SSH 服务器或写入串口硬件；macOS/Linux 真机仍待单独验证。
