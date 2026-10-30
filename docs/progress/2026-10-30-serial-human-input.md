# 串口人工输入与 MCP 命令列表分离

日期：2026-10-30。

## 设计澄清与修正

- 人工 Shell 模式与 SSH 一样，直接在终端内输入，设备负责回显、行编辑与命令执行。
- 人工 UART 调试模式保留发送框，支持文本、HEX、行结束符与定时重复发送。
- 有序命令列表仅用于 MCP 操作设备 Shell，减少 Agent 调用次数；不要求人把终端输入改成列表。

删除此前误加的 Shell 列表编辑器、发送间隔控件、发送/停止队列按钮及相关前端解析、轮询和翻译。两种人工输入继续通过 `serial.write`，MCP 的 `serial.command` 后端队列、审批、取消、限额和未知设备完成语义保持原实现。MCP 进度事件仍更新 TX 计数；队列在途时维持输入隔离，完成后可继续人工输入。

架构约定同步更新至 [串口插件](../architecture/serial-plugin.md)。此前硬件与虚拟配对记录保留为各自测试时点的历史结果。

## 验证

- `pnpm build`：通过。
- `pnpm plugin:serial:build`：通过，更新串口开发包。
- `node --import ./scripts/test-ts-loader.mjs --test tests/serial-terminal-browser.test.mjs`：通过，无跳过。使用实际前端构建、无头浏览器和宿主接口替身。
- `pnpm test`：36 项通过，无失败、无跳过；包含上述浏览器回归。
- `cargo test -p coterm-plugin-serial`：26 条单元、2 条集成测试通过；硬件及虚拟配对用例维持默认 ignored。

浏览器回归通过真实键盘/文本输入事件验证 Enter、Tab、方向键、退格、Ctrl+C、中文输入、设备回显，及 UART 多行文本/HEX/Shift+Enter、MCP 队列占用与恢复、切换视图和语言不重连、断开后不发送、卸载释放订阅。断言人工界面不提交 `serial.command`。

本轮没有重新写入物理设备或虚拟串口，未修改串口后端、MCP 传输或审批协议。
