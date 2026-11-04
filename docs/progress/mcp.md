# MCP Bridge

## 当前能力

- 工具由已启动插件的握手注册，Bridge 保留标准 inputSchema，统一处理实例名称、Agent 来源、审批和操作收据。
- 对外目录使用扁平 `coterm_<plugin>_<tool>`；`coterm.status` 提供有界健康 / 连接快照，实例工具在唯一确认已连接且无未知状态时可省略 `name`。
- 插件可声明工具 descriptor 的 `_meta["coterm/returnOnApproval"]`，复用运行时状态订阅，在人工审批就绪时返回；未声明的工具和普通运行时提交保持原有等待行为。
- 串口命令可在一次调用中发送文本并收集提示符前的回显，结果同时提供可读文本与结构化字段；近期输出另有无状态快照接口。
- Agent 命令审批支持用户自定义正则：正向放行匹配命令，反向拦截匹配命令；编辑器支持追加/删除、试匹配和显式保存。规则作用于实际命令文本，旧接口级放行不再生效。

## 2026-11-04：MCP 连接、启动方式与状态

- MCP 连接从端口表单拆为独立设置项，使用等宽单选胶囊选择自启动 / 手动启动；手动模式显示启动 / 关闭按钮。按截图修订：将已启动 / 未启动状态移到右侧 MCP 分类标题处，不在搜索框上方占整行；客户端配置拆到连接项下方独立一行，左侧标签、右侧查看按钮。删除端口行冗余说明和桥接地址，客户端弹窗只负责配置查看与复制。
- 本次布局修订验证：`pnpm build` 及 MCP / 设置浏览器测试 4 项通过，新增断言确认状态在分类标题、客户端配置独占连接项后的设置行、切换其他分类不残留 MCP 状态。明暗主题及 360px 中文窗口已目视核对；没有修改 Rust、启动策略或端口契约。
- `startupMode` 与端口保存在同一设备设置文件，默认 `auto`，旧版只有端口的文件兼容自启动。选择 auto 立即尝试启动，manual 保留当前服务、下次启动关闭；手动开关不改变启动方式。绑定或写盘失败保留原模式和原服务，修改端口保留模式、修改模式保留端口。
- 新可信宿主命令 `set_mcp_startup_mode`；桌面启动通过 `start_on_launch` 读取策略。前端 [共享状态控制](../../apps/desktop/src/components/settings/use-mcp-settings.ts) 统一快照、操作锁与错误恢复；[连接设置](../../apps/desktop/src/components/settings/McpConnectionSetting.tsx) 复用现有 ToggleGroup、Button 和客户端配置弹窗，无新依赖、权限、插件消息或网络入口。
- 验证通过：`pnpm build`、`cargo check --workspace --locked`、Desktop / MCP 全目标 Clippy（警告视为错误）；桌面单元 22 项、MCP 测试 14 项；前端完整套件使用 `--test-concurrency=2`，78 项全部通过。浏览器覆盖等宽胶囊、重复点击、单一在途操作、人工开关、模式保存失败、顶部状态、重启恢复及端口既有回归；Rust 使用真实回环监听验证自动启动、自定义端口、占用与保存失败保留原服务。中文 360px 窗口和明暗主题已目视核对，未关闭用户正在运行的桌面，加载 Rust 改动需要重启开发进程。

## 2026-11-04：设置中的 MCP 端口

- 设置新增可搜索的 MCP 分类，端口支持 1–65535、默认 51234；显式保存、按设备恢复，提供现有 MCP 连接弹窗作为启停 / 复制配置入口。显示偏好重置不修改端口，中英文及窄窗口沿用现有设置布局。
- 宿主增加受信桌面命令 `set_mcp_stdio_port({request:{port}})`，状态增加 `port / configurationError`。启用时先绑定新端口并原子保存，再替换旧桥；占用或保存失败保留旧服务。停用时仅检查可用性及保存，不自行启用。设置损坏时阻止启用，保存有效端口后恢复。
- 自定义端口的 Stdio 客户端配置自动携带 `--addr`；默认端口保留既有 `mcp stdio` 配置兼容。成功换端口会断开旧 MCP 客户端，需复制新配置并重新连接；插件连接和审批规则不变。只接受 IPv4 回环端口，不增加 HTTP、插件消息、依赖或 capability。
- 新文件 [mcp_settings.rs](../../apps/desktop/src-tauri/src/mcp_settings.rs) 保存非敏感设备偏好，Windows 复用既有原子替换，其他平台使用 rename；保存通过阻塞任务执行。入口：[桥生命周期](../../apps/desktop/src-tauri/src/mcp_commands.rs)、[设置控件](../../apps/desktop/src/components/settings/McpPortSetting.tsx)、[客户端配置](../../crates/coterm-mcp/src/config.rs)、[浏览器验收](../../tests/mcp-settings-browser.test.mjs)。稳定规则见 [MCP 接入](../architecture/mcp-integration.md#端口设置与宿主接口)。
- 验证通过：`pnpm build`、`cargo check --workspace --locked`、桌面单元测试 17 项（`cargo test -p coterm-desktop --bin coterm-desktop --locked`）、MCP 测试 14 项，以及 Desktop / MCP 全目标 Clippy（警告视为错误）。前端完整套件使用 `node --import ./scripts/test-ts-loader.mjs --test --test-concurrency=2 tests/*.test.mjs`，77 项全部通过；窄窗口中文、明暗主题已目视检查。
- 回归补充了实例命名测试对插件配置表单挂载的等待。默认并发运行时，既有 Toast 测试的 500ms 提示在样式断言前过期；该项单独通过，限制测试并发后全套通过，未修改 Toast 实现。运行中的桌面锁定可执行文件，因此桌面测试只运行单元目标；未关闭用户进程、替换正在运行的程序或进行安装包验收。加载本次 Rust 改动需重启桌面开发进程。

## 2026-11-02：命令文本正则审批

- 宿主使用独立策略模块编译、限额和匹配正则；模式为 `allow_matches` / `approve_matches`。列表逐项检查，正向全部匹配才放行，反向任意匹配即进入人工审批；多行原文整体匹配，不拆分 Shell 语法。
- SSH command/exec 与串口文本 command 通过 inputSchema 的可选 `x-coterm-approval-text` 声明命令路径；宿主不写死协议字段。无声明、无效或缺失参数保留原审批；串口文本保持账本脱敏，HEX/原始写入和文件写入不接入文本匹配。
- 新策略进入加密工作区，保存失败保留原运行时策略和编辑内容。旧 `allowedOperations` 兼容读取但不再授予放行，不将操作名转换为正则；已等待的提案不会因规则变化自动执行。
- 前端改为两种模式、规则列表、ls/cd 示例及宿主试匹配；不再获取/展示接口目录。中文和 420px 窄屏目视核对通过，截图在忽略的 `target/approval-settings-narrow.png`。
- 验证通过：`pnpm build`；审批、工作区和实例名称浏览器测试 3 项；Core 策略 3 项、运行时模块 21 项；真实进程 lifecycle 4 项；桌面单元 10 项（串行执行）；SSH/MCP/串口相关回归通过，串口真实设备用例仍需显式设备。桌面首次并行测试出现一次已有工作区写盘失败，单项复测及完整串行复测通过。
- 继续入口：[规则契约与示例](../architecture/command-approval.md)、[策略模块](../../crates/coterm-core/src/approval_policy.rs)、[设置 UI](../../apps/desktop/src/components/host/ApprovalSettings.tsx)、[浏览器验收](../../tests/approval-settings-browser.test.mjs)。不修改协议认证、Bypass 开关或现有提案的人工批准权限。
- `cargo check --workspace`、Core/MCP/SSH/Serial 全目标 Clippy（警告视为错误）通过；SSH、Serial 开发包均已重建。重启基座后需让 MCP 客户端重连，使用新握手目录中的命令字段声明。
- 将暂存区单独导出后，宿主 TypeScript 检查与 `cargo check --workspace --locked` 通过，确认提交不依赖工作区其他未提交的 SDK、Local 插件或配置修改；新增文档链接和暂存差异检查通过。

## 2026-11-02：Agent 健康检查、默认路由与扁平工具名

- 新增 `coterm.status`，一次返回桌面 / Bridge 可达、实例 ready 与真实连接状态；最多采样 64 实例、8 并发，整体收集最多 1 秒，未能确认时返回 null。成功响应才能证明可达，关闭的桌面 / Bridge 仍由 Stdio 明确报连接失败。
- SSH / Serial 的 status descriptor 声明 `coterm/connectionStatus`；基座只接受实例级、read、非敏感、transient 的单一状态工具，无需协议硬编码或认证。第三方未声明工具保持未知。
- 实例 `name` 从公共 schema 必填项移除；仅目标插件唯一确认已连接、无未知且快照完整时默认路由。无连接、多连接、未知或显式无效参数不派发写入；查询 / 取消省略目标时使用全局同样约束，仍验证记录归属。
- 目录发布 `coterm_ssh_exec` / `coterm_serial_command` 等扁平名，旧 `<pluginId>__<native>` 仍可调用但不重复列出；归一化碰撞明确拒绝。插件内部操作和进程协议不变。
- SSH 与 Serial 一样，进入人工审批立即返回 `pending_approval`，批准后查询原操作；原有审批、Bypass、密码 / sudo、取消和断线不重放规则保持。
- 宿主每条输入继续限 16 KiB，输出独立限 192 KiB；适配 SSH 两流各 32 KiB 和可读摘要，超限 / 脱敏保护仍有效。Serial 自身预览不扩容。
- 决策与使用：[MCP Agent 调用体验](../design/mcp-agent-experience.md)；稳定契约：[MCP 接入](../architecture/mcp-integration.md)。已解决反馈从原反馈文件移除。

本轮 Windows 验证：

- `cargo check --workspace` 通过。
- `cargo test -p coterm-mcp -p coterm-plugin-ssh -p coterm-plugin-serial`：101 项通过；2 项原本要求显式设备的串口测试保持忽略。
- Core 完整测试出现 5 项插件安装 fixture 失败：当前工作区 Local manifest 新增 frontend，而既有安装测试包仅写入 backend，触发 `plugin_entry_unavailable`。没有改动该 manifest 或混入 Local 开发；明确排除这 5 项后，其余 37 项 Core 测试通过，合计 138 项相关测试通过。
- `cargo clippy -p coterm-core -p coterm-mcp -p coterm-plugin-ssh -p coterm-plugin-serial --all-targets -- -D warnings` 通过。
- `pnpm plugin:ssh:build` / `pnpm plugin:serial:build` 通过，开发插件包已重建；没有关闭运行中的桌面或插件。启动修复版本需重启桌面，让 MCP 客户端重连并刷新目录。

## 2026-10-31：串口 MCP 使用反馈

已实现：

- 实例工具描述明确 `name` 来自 `coterm.list_instances`；Bridge 兼容 `instance`、`instanceName`、`instance_name`，拒绝同时传多个别名。
- `serial.command` 的默认行结束符为 CR，显式 `lineEnding:"none"` 保留原样输入；人工单次输入和 HEX 不受此默认值变化影响。
- 命令等待输出在 `content[].text` 中直接可读，同时保留 `structuredContent.output`；等待期间支持取消、断开。
- 新增 `serial.get_recent(lines)`，默认 50 行，范围 1～200，返回最近缓冲的快照，包含连接、游标、缺口及截断标记。
- 回显和近期快照按 JSON 转义后的 6 KiB 文本预算截断，两个 MCP 副本合计满足当时宿主的 16 KiB 结果上限；2026-11-02 宿主输出预算扩大，串口预览保持原限额。
- 严格 schema 校验仍启用；未知字段和缺失必填字段的错误提示包含字段名，不包含参数值。
- 串口提交工具进入审批时返回 `pending_approval` 和操作 ID，用户批准后查询原操作，不能重提同一写入。

验证入口：

```bash
cargo test -p coterm-core -p coterm-mcp -p coterm-plugin-serial
cargo check --workspace
cargo test --workspace
```

覆盖命令回显、默认 CR、原样输入、实例别名及冲突、未知字段脱敏、审批及时返回、控制字符的结果限额、等待输出时取消。真实串口和虚拟串口配对用例仍需明确选定设备后执行；普通测试不会打开物理端口。

本次验收结果：

- `cargo check --workspace` 通过。
- `cargo test --workspace --exclude coterm-desktop` 通过，覆盖 Core、MCP、串口、SSH、Local 和协议契约；SSH 原有跨审批等待行为回归通过。
- `cargo test -p coterm-desktop --bin coterm-desktop` 的 9 项桌面单元测试通过。
- `cargo clippy -p coterm-core -p coterm-mcp -p coterm-plugin-serial -- -D warnings` 通过。
- `pnpm plugin:serial:build` 通过，已重建串口后端和前端开发包。
- `cargo fmt -p coterm-plugin-serial -- --check` 和 `git diff --check` 通过。
- 完整 `cargo test --workspace` 因运行中的桌面应用锁定 `target/debug/coterm-desktop.exe`，无法完成桌面集成目标的重建；未关闭用户进程。退出旧桌面后需补跑该命令。

启动修复版本时先退出旧桌面，再运行 `pnpm tauri:dev`；串口插件开发包已生成，MCP 客户端需重新连接以刷新工具目录。

## 限制与后续入口

- 客户端导出 `parameters:null` 尚不能在服务端复现：服务端 `tools/list` 已提供标准 schema；仍需对比使用现场的原始响应与客户端转换结果，不能将客户端导出字段当作服务端缺失的证据。
- `serial.get_recent` 是重复可读的尾部快照，不提供隐式共享游标。需要精确增量时使用 `serial.read(cursor)`。
- 基座协议与边界见 [MCP 接入](../architecture/mcp-integration.md)；串口接口设计和使用说明已整理到插件自己的 [docs 目录](../../plugins/plugin-serial/docs/README.md)，[原架构入口](../architecture/serial-plugin.md) 保留基座集成边界与导航。
- 继续验收从 [待确认反馈](../feedback/serial_mcp_use_feedback.md) 及 `plugins/plugin-serial/tests/serial.rs` 的真实进程回归入口开始。
