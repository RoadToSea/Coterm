# Coterm 开发约定

## 项目定位

Coterm 是面向人类与 Agent 协同的终端交互基座。支持 SSH、串口（Serial）、本地终端、Telnet 与 Raw TCP，基座负责实例路由、命令块呈现、公共交互、操作记录和 MCP Bridge；底层通信由内置驱动负责。

系统明确业务边界：放弃对底层非标字节流解码、连续波形分析与信号数学计算的扩展，此类功能划归 Lissio 等虚拟仪器上位机。

驱动以 Rust 库链接到桌面主进程；每种驱动使用独立异步任务，同一驱动支持多个实例。任务隔离不提供进程级崩溃隔离。

文档从 `docs/README.md` 进入，按内容分为四类：

- `docs/requirements/`：需求，包含模块划分、各模块细致需求、公共需求、职责和信息交互，不涉及代码实现。
- `docs/implementation/`：技术实现，包含整体代码关系、公共复用、模块内部方案、内部契约、开发验收与实现状态。
- `docs/feedback/`：未解决反馈、复现条件、现场证据和继续调查入口。
- `docs/api/`：对外 API，包含外部接入、公开工具、参数、返回值、错误和调用限制。

设计阅读顺序仍为“模块 → 模块需求 → 公共需求 → 职责与交互 → 代码架构”。前四步位于需求目录，第五步见 `docs/implementation/architecture.md`。实现现状与后续建议必须区分，修改行为时同步需求、交互、实现和受影响的公开契约。

## 目录

```text
apps/desktop/          Tauri + React 桌面应用
apps/shared/           桌面与内置驱动共用的前端类型和交互模块
apps/desktop/src-tauri Tauri Rust 后端
crates/coterm-core     共享类型、实例运行时、审批、操作记录和终端工具
crates/coterm-mcp      MCP Bridge 相关类型和逻辑
crates/coterm-driver   进程内驱动，src/{ssh,serial,local,tcp,telnet}；network/stream 复用内部逻辑
docs/                  需求、技术实现、feedback、对外 API 的集中入口
```

## 常用命令

在仓库根目录执行：

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
pnpm install
pnpm dev
pnpm build
pnpm tauri:dev
pnpm tauri:build
```

修改 Rust 后至少运行 `cargo check --workspace` 和相关测试；修改前端后运行 `pnpm build`。

## 编码规则

- Rust 新代码避免 `unwrap()`、`expect()` 和未处理的 panic；对外命令返回结构化 `Result`。
- Tauri Command 使用明确的请求/响应类型；前端不要用 `any` 接收宿主返回值。
- 耗时 IO 使用异步运行时，不阻塞 UI 线程；事件订阅在组件卸载时注销。
- 不在基座实现具体通信协议，不把不同协议强行改造成统一业务模型。
- 驱动通过类型化 HostCommand 和有界通知队列与运行时通信，不启动独立驱动程序；阻塞设备 IO 必须使用专用线程或 spawn_blocking，停止时释放连接及后台作业。
- Agent 写操作要经过基座的来源识别和审批规则；Bypass 由用户主动开启，不能绕过协议自身的认证和交互。
- 密码、Token 等秘密不得进入普通事件、日志、MCP 返回值或明文持久化数据。
- 高频数据必须有界、可取消，并处理慢消费者、断线和数据缺口；不要无限缓存。
- 不写死机器绝对路径，不提供任意 shell 或任意文件访问代理。
- 新功能新建分支再修改，功能完成及时提交git，测试通过后推送云端

## 调试产物与清理

- Agent 的临时脚本、日志、截图、抓包、测速结果、源码副本和备份统一放在仓库根目录的 `.agent-debug/<任务名>/`；创建时使用相对路径。可配置的测试输出目录也指向这里。
- 正式源码、测试夹具和文档仍放在对应模块；编译器缓存与构建产物沿用工具链目录。浏览器配置等工具要求的系统临时目录由创建者在退出时回收。
- 调试结束先把已验证能力归入 `docs/implementation/verification.md`，将未解决现象和证据归入 `docs/feedback/known-issues.md`，再停止调试进程。确认没有其他任务使用该目录后，Agent 必须调用 `pnpm debug:clean` 并确认目录为空；用户要求保留现场时说明保留内容，不执行清理。
- `pnpm debug:clean --dry-run` 用于预览。清理只使用此脚本，范围固定为 `.agent-debug/` 内容并保留目录本身；清理失败时报告残留与原因，不扩大删除范围。日志和截图先脱敏，凭据不写入调试文件。

## 修改边界

新增依赖、修改 Tauri capability/权限、外部进程启动方式或跨进程协议时，先检查跨平台影响，并同步更新文档和测试。保持改动聚焦，不顺手重构无关代码。

## 文档维护

- 文档按 `requirements/`（需求）、`implementation/`（技术实现）、`feedback/`（反馈）、`api/`（对外 API）分类，根目录只保留文档导航 `README.md`；需求不放源码路径、类定义、实现示例或消息编码。
- 每项稳定要求只在所属需求文件定义，公共要求集中维护。代码架构先写模块间依赖，再写公共复用，最后写模块内部实现；逻辑模块不等于新 crate 或转发层。
- 外部调用方需要的工具、参数、返回值、错误和限制放 `docs/api/`；宿主、驱动、前端的内部控制、配置和容量放 `docs/implementation/contracts.md`，构建、测试、硬件和发布命令放 `docs/implementation/development.md`。
- 已实现能力和有效验收放 `docs/implementation/verification.md`，未解决问题、复现条件和现场证据放 `docs/feedback/`；已解决反馈归入相关需求或实现，不按日期或任务新建文件，不累积开发历史。
- 长篇项目说明统一放在 `docs/`；其他模块只保留必要的接口注释和可执行示例，不另建大量需求、架构、使用及进度文档。
- 整理或移动文档后修复所有相对链接，检查旧路径残留，并运行 `git diff --check`。文档变更完成后按常规新建分支、提交并推送。
