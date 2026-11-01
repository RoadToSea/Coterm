# Serial 实施进度

## 当前能力

领域接口与容量限制见 [接口设计](interfaces.md)，交互和运行方式见 [使用说明](usage.md)。插件独立管理配置、连接、收发、输出缓冲、命令队列及前端界面。

## 2026-11-01：调试界面

- 顶栏切换终端 / UART，右侧连接与配置；配置幕布覆盖接收区，独立滚动，Esc 收起，支持减少动画设置。
- 通信、接收、发送分组；断开后通过 `serial.configure` 应用通信参数；控制线电平与自动硬件流控明确区分。
- UART 支持 Enter 单次发送、Shift+Enter 换行和中文输入法保护；底部提供文件导入、临时发送历史、ASCII / HEX 与定时发送。
- 每次定时发送完成后才安排下一次；输入变化、切模式、断线、失败或卸载后停止，丢弃未发送内容，不自动重试。
- 接收快照使用用户指定文件名下载，历史与文件导入都遵循内存 / 单次发送预算。

## 验收入口

`pnpm --filter coterm-plugin-serial-ui build` 后执行 `node --import ./scripts/test-ts-loader.mjs --test tests/serial-terminal-browser.test.mjs`。它加载真实插件 bundle，在无头浏览器检查键盘、IME、配置覆盖、文件导入、历史、慢写入、停止 / 失败 / 卸载和窄窗口。Rust 通过 `cargo test -p coterm-plugin-serial` 验证配置补丁与原有串口契约；物理与虚拟串口测试需显式选择设备。

本次验收：`pnpm build`、`pnpm plugin:serial:build` 通过；完整 `pnpm test` 57 项通过、0 失败、0 跳过。`cargo check --workspace` 和串口 Clippy 通过，串口 Rust 测试 46 项通过，2 项需设备的用例保持 ignored。人工复核真实插件构建的深色、浅色与 360px 窄窗口截图；同时单独构建并验证本次提交的前端内容。原有工作区截图逐字节保留。

## 限制与后续入口

- 接收保存是当前缓冲快照；草图中的持续录制到指定路径仍需受限的用户文件授权能力，不能用任意路径读写代理替代。宿主 WebView 的原生文件选择和下载落盘需在真实桌面继续验收。
- `serial.configure` 只改当前插件进程的实例配置，不更新宿主恢复快照或既有实例名称；需要跨重启保留时，应先设计配置更新契约。
- 当前不实现 RTS/CTS 自动硬件流控；DTR / RTS 只表示打开时与连接中的控制线电平。
- 界面入口为 `frontend/src/index.ts` 和 `frontend/src/console-settings.ts`；领域入口为 `src/backend.rs`。
