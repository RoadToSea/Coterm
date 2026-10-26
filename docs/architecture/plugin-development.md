# 插件开发

后端 JSON 控制协议采用 [v1.0 基线](runtime-communication.md)，前端 SDK 与二进制流仍在演进。字段以 [Rust 运行时类型](../../crates/coterm-proto/src/runtime) 和 [前端 SDK](../../packages/plugin-sdk/src/plugin-api.ts) 为准，未实现能力见 [项目进度](../PROGRESS.md)。

## 插件边界

插件负责协议连接、领域操作、数据与持久化，可提供自己的前端。基座负责生命周期、实例路由、公共交互与 MCP 入口。协议语义不进入基座，界面和 MCP 必须操作同一实例。

## 插件包与发现

```text
plugin/
├── manifest.json
├── backend/       平台可执行入口
└── frontend/      可选，单文件 ES 模块
```

以 [SSH manifest](../../plugins/plugin-ssh/manifest.json) 或 [无 UI 插件 manifest](../../plugins/plugin-local/manifest.json) 为模板：

- 声明插件 ID/版本、manifest 版本、运行时主版本及必需能力。
- 后端入口按平台声明，使用包内相对路径；不得通过路径穿越访问包外文件。
- 前端可省略；`mcp.provider: "backend"` 表示工具由同一个后端提供，不另开连接。
- 支持多个实例，插件不能依赖宿主当前标签页定位目标。
- 打包所需运行时或明确外部依赖，不假定用户装有编译工具。

基座先扫描应用数据目录的 `plugins/<package>`，开发构建再扫描仓库的 `plugins/<package>/dist`。同 ID 已安装包优先，同来源按目录名排序；目录级符号链接不扫描。

`list_plugins` 返回 `{ plugins, issues }`：坏包被隔离，诊断只包含目录名、可解析的插件 ID、稳定错误码和提示，不暴露绝对路径。具体校验见 [插件管理器](../../crates/coterm-core/src/plugin_manager.rs)。

## 后端

Rust 插件实现 `coterm_proto::runtime::backend::PluginBackend`，通过 `transport::serve(Backend::default())` 运行。共享传输层负责行协议、握手顺序、请求关联、限额与通知输出；其他语言实现同一控制协议。

原有 `transport::PluginBackend` 导入仍可用；曾按未实现的 `handle/BackendHost` 草案接入的代码需迁移到 `request`。

1. 握手使用 `RuntimeHello::negotiate_features/control_limits`，返回与 manifest 一致的身份版本、能力交集、限制及目录；未协商 native-mcp-tools 时不返回工具。
2. 创建实例时校验配置，按 `pluginInstanceId` 独立保存连接和领域状态。
3. 销毁实例或停止插件时清理连接、待交互、提案、订阅与任务；拒绝迟到输入。
4. 长操作立即受理，再发送完成通知；等待用户或协议结果时不占住控制消息循环。
5. stdout 只写协议，日志写 stderr；秘密不进入普通输出。

消息编码、取消和交互细节见 [运行时通信](runtime-communication.md)。参考实现为 [SSH 后端](../../plugins/plugin-ssh/src/backend.rs) 和 [串口后端](../../plugins/plugin-serial/src/backend.rs)。

## 操作与 MCP

每个操作声明名称、`scope`、`effect`、输入 schema，以及可选输出 schema：

| 字段 | 约定 |
|---|---|
| `scope=instance` | 必须定位实例 |
| `scope=plugin` | 不接受实例目标，使用可信插件授权域 |
| `effect=read/draft/commit` | 由注册目录决定，调用方不能自报降级 |
| `sensitiveInput` | 原始输入可能包含秘密时，禁止写入普通账本参数 |
| `transient` | 后台读取等使用独立有界记录区，不挤占命令历史 |

Agent commit 默认先准备提案；准备阶段不能产生发送副作用，批准后执行用户确认的参数快照。Bypass 仅跳过宿主提交审批，仍需协议认证。只有确认未提交或已停止，才可报告取消成功。

MCP Tools 需单独注册 descriptor 与内部 operation 的映射；不会自动暴露整个操作目录。参数、结果及协议领域语义由插件定义，接入规则见 [MCP](mcp-integration.md)。

## 前端

导出 `mount(container, api)` 并返回 `{ unmount() }`。可额外导出 `configure(container, api)`，通过 `api.submit(config)` 创建实例；缺省时基座使用 JSON 配置。

当前前端入口是无相对依赖的单文件 ES bundle，最大 2 MiB，经 Blob URL 加载。使用 `@coterm/plugin-sdk` 类型，不能导入桌面源码、直接调用 Tauri 内部接口或访问其他实例。

| API | 当前状态 |
|---|---|
| `invoke/getOperation/recentOperations/cancelOperation` | 可用，绑定当前实例 |
| `approveProposal/rejectProposal` | 可用，基座校验提案与快照 |
| `subscribeEvents/subscribe` | 可用，有界 JSON 事件 |
| `requestSecret` | 可用，前端只拿到一次性秘密引用 |
| `requestConfirmation` | 当前使用浏览器确认框 |
| `components.blocks` | 可选块列表；块数据、选择结果和上下文由插件管理 |
| `subscribeStream/requestText/requestSelection` | 已有类型，尚未接通 |

挂载时先订阅，再读取状态及回放快照；按版本、连接 ID 和游标接续。卸载必须注销订阅、DOM 监听器及自身 UI 资源，但不因此断开后端实例。

样式使用宿主 [CSS token](../../apps/desktop/src/index.css)，如 `var(--coterm-text-primary, #cccccc)`；Shadow DOM 继承变量并提供可读 fallback，不覆盖宿主变量。界面需适应窄容器并支持键盘操作，当前基线为深色主题。

## 开发与验收

- 参考 [SSH](ssh-plugin.md) 或 [串口](serial-plugin.md) 组织后端、前端、manifest 和构建脚本。
- 构建当前平台完整包到插件 `dist`，启动桌面，检查发现诊断与能力握手。
- 至少验证双实例隔离、配置错误、审批修改/拒绝、取消/超时、秘密隔离、慢消费者与输出缺口。
- 验证无 UI 插件仍可请求宿主交互；没有交互入口时明确失败，不能自动批准。
- 验证前端卸载后 MCP 仍可使用实例；插件崩溃或结果不明时不得自动重放写操作。
- 先运行 `cargo test -p coterm-proto --test control_contract`；新增插件另补真实进程、多实例与领域契约测试。流接口需用持续订阅型插件验证后独立冻结。

构建与检查命令见 [仓库 README](../../README.md)。数据归插件保存，宿主操作账本不能作为永久业务存储。
