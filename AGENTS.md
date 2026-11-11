# Coterm 开发约定

## 项目定位

Coterm 是 Tauri v2 桌面基座。基座负责驱动生命周期、实例路由、前端挂载、公共交互、操作记录和 MCP Bridge；SSH、串口、MQTT、CAN 等协议由驱动负责。

驱动以 Rust 库链接到桌面主进程，每种驱动使用独立异步任务；同一驱动支持多个实例。驱动可以无 UI，也可以提供自己的完整前端。驱动自己管理协议数据和持久化，前端只能通过基座 API 访问驱动后端。

架构和接口设计以以下文档为准：

- `docs/SPEC.md`：总技术路线
- `docs/architecture/driver-development.md`：驱动开发入口
- `docs/architecture/`：运行时、通信和 MCP 设计

这些接口仍在设计阶段，修改实现时同步更新相关文档。

## 目录

```text
apps/desktop/          Tauri + React 桌面应用
apps/shared/           桌面与内置驱动共用的前端类型和交互模块
apps/desktop/src-tauri Tauri Rust 后端
crates/coterm-proto    跨进程协议和共享数据类型
crates/coterm-core     会话、输入仲裁、命令块和安全逻辑
crates/coterm-mcp      MCP Bridge 相关类型和逻辑
drivers/               独立驱动及 manifest
docs/                  架构和阶段技术方案
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
- 调试结束先把必要结论写入对应进度或反馈文档，再停止调试进程。确认没有其他任务使用该目录后，Agent 必须调用 `pnpm debug:clean` 并确认目录为空；用户要求保留现场时说明保留内容，不执行清理。
- `pnpm debug:clean --dry-run` 用于预览。清理只使用此脚本，范围固定为 `.agent-debug/` 内容并保留目录本身；清理失败时报告残留与原因，不扩大删除范围。日志和截图先脱敏，凭据不写入调试文件。

## 修改边界

新增依赖、修改 Tauri capability/权限、外部进程启动方式或跨进程协议时，先检查跨平台影响，并同步更新文档和测试。保持改动聚焦，不顺手重构无关代码。

## 文档维护

- `docs/` 根目录只保留 `SPEC.md`；整体架构和稳定接口以 `SPEC.md` 与 `docs/architecture/` 为准，修改实现时同步更新相关架构文档。
- `docs/design/` 记录复杂业务实现前的方案、取舍、接口草案和验收条件。方案落地后，将仍有效的约束同步到 `architecture/`，实施状态记录到 `progress/`。
- `docs/feedback/` 保存外部用户、Agent 和现场使用后的反馈、复现条件及观察结果。反馈确认后，把产品决策写入 `design/` 或 `architecture/`，把实施进度写入 `progress/`。
- `docs/progress/` 按长期模块维护，不按日期或单次任务新建文件。模块文件至少记录当前能力、限制、重要实现/验收结果和下次继续开发的入口；同一模块已有文件时追加日期章节并合并维护。项目级状态集中在 `docs/progress/project.md`。
- `docs/release/` 记录发布范围、构建步骤、发布前验收和已知限制；开发过程仍记录在 `progress/`。
- 整理或移动文档后，修复所有相对链接，检查 `docs/` 根目录内容，并运行 `git diff --check`。文档变更完成后按常规新建分支、提交并推送。
