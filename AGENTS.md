# Coterm 开发约定

## 项目定位

Coterm 是 Tauri v2 桌面基座。基座负责插件生命周期、实例路由、前端挂载、公共交互、操作记录和 MCP Bridge；SSH、串口、MQTT、CAN 等协议由插件负责。

插件可以无 UI，也可以提供自己的完整前端；同一插件支持多个实例。插件自己管理协议数据和持久化，前端只能通过基座 API 访问插件后端。

架构和接口设计以以下文档为准：

- `docs/SPEC.md`：总技术路线
- `docs/architecture/plugin-development.md`：插件开发入口
- `docs/architecture/`：运行时、通信和 MCP 设计

这些接口仍在设计阶段，修改实现时同步更新相关文档。

## 目录

```text
apps/desktop/          Tauri + React 桌面应用
apps/desktop/src-tauri Tauri Rust 后端
crates/coterm-proto    跨进程协议和共享数据类型
crates/coterm-core     会话、输入仲裁、命令块和安全逻辑
crates/coterm-mcp      MCP Bridge 相关类型和逻辑
plugins/               独立插件及 manifest
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
- 插件进程通过运行时协议通信；插件 stdout 只输出协议数据，日志写 stderr 或结构化日志消息。
- Agent 写操作要经过基座的来源识别和审批规则；Bypass 由用户主动开启，不能绕过协议自身的认证和交互。
- 密码、Token 等秘密不得进入普通事件、日志、MCP 返回值或明文持久化数据。
- 高频数据必须有界、可取消，并处理慢消费者、断线和数据缺口；不要无限缓存。
- 不写死机器绝对路径，不提供任意 shell 或任意文件访问代理。
- 新功能新建分支再修改，功能完成及时提交git，测试通过后推送云端

## 修改边界

新增依赖、修改 Tauri capability/权限、外部进程启动方式或跨进程协议时，先检查跨平台影响，并同步更新文档和测试。保持改动聚焦，不顺手重构无关代码。
