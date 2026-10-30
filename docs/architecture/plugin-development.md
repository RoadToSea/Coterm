# 插件开发

后端 JSON 控制协议采用 [v1.0 基线](runtime-communication.md)，前端 SDK 采用 v1.0；二进制流仍在演进。字段以 [Rust 运行时类型](../../crates/coterm-proto/src/runtime) 和 [前端 SDK](../../packages/plugin-sdk/src/plugin-api.ts) 为准，未实现能力见 [项目进度](../progress/project.md)。

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

### 离线安装与分发

桌面左侧的 **安装插件 / Install plugin** 打开原生文件选择器，接收 `.coterm` 或 `.zip` 文件。`.coterm` 是普通 ZIP，压缩包根目录直接包含 `manifest.json`，不能再包一层 `dist/`。可使用 `pnpm plugin:pack <dist目录> <输出.coterm>` 生成；打包器使用相同校验逻辑，并为 manifest 声明的后端入口保存 Unix 执行权限。SSH 的发布构建使用 `pnpm plugin:ssh:build --release`。

安装分为选择/预览和确认两个阶段：

1. 宿主选择本地文件，后台解压到应用数据目录的临时暂存区；不启动包中的后端，也不加载包中的前端。
2. 校验 manifest、运行时能力、当前平台及所有声明的入口；前端入口还须为不超过 2 MiB 的 UTF-8 文件。界面只拿到一次性令牌、插件 ID、名称、版本和平台，不接收文件绝对路径。
3. 用户确认信任后，将暂存目录原子移动到 `plugins/<pluginId>`，立即登记到运行时列表。安装不自动启动插件或创建实例，正常重启后仍可发现。
4. 取消预览、改选文件或校验失败会清理对应暂存内容。每个应用进程只有一个暂存槽；确认令牌 10 分钟后失效。Alpha 拒绝同 ID 覆盖，包含开发包和已安装包。

上限：压缩包 128 MiB、单文件 128 MiB、解压总量 512 MiB、4096 个条目、包内路径 512 字节。拒绝路径穿越、绝对路径、反斜杠、Windows 设备名/尾点/尾空格、大小写重复路径和符号链接等特殊文件；解压后复用运行时入口校验。诊断仅包含稳定错误码，无绝对路径或系统错误原文。

Windows、macOS、Linux 共用 ZIP 校验与原子移动；原生选择器由 Tauri dialog 提供，Linux 沿用 Tauri/GTK 系统依赖。新增 dialog Rust 插件不向 WebView 授予通用文件系统或 dialog IPC 权限，前端只能调用专用安装命令。离线包仍是以当前用户权限执行的可信原生代码，校验不等于签名验证或安全沙箱。

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

导出 `mount(container, api)` 并返回 `{ unmount() }`。可额外导出 `configure(container, api)`，通过 `api.submit(config)` 创建实例。点击插件时，基座在主工作区内联挂载此配置前端，仅展示插件负责的通信协议配置，不再提供操作调用器或原始 JSON 编辑器；缺少配置前端时显示说明。需要人工创建实例的插件应提供 `configure`。

每个打开的实例对应主区域上方一个标签页。切换标签或进入配置页只隐藏面板，不调用 `unmount()`，不得依赖焦点或可见性来维持后端连接；终端应通过尺寸观察器在重新显示后适配。关闭标签、实例退出或失败时释放前端资源；关闭标签不销毁后端实例，仍可从左侧重新打开。实际销毁入口属于实例生命周期管理，须确认；标签与侧栏使用稳定实例 ID 路由，用户界面只展示名称。

SSH/串口配置前端在自己的 Shadow DOM 中为输入框、选择框及提交按钮设置圆角并继承主题变量，不能依赖宿主的 Tailwind 样式穿透。配置提交期间基座禁用表单交互，插件还应在提交处理器中抑制重复请求。

当前前端入口是无相对依赖的单文件 ES bundle，最大 2 MiB，经 Blob URL 加载。使用 `@coterm/plugin-sdk` 类型，不能导入桌面源码、直接调用 Tauri 内部接口或访问其他实例。

可选前端终端可复用 SDK 的 `@coterm/plugin-sdk/components/terminal-shortcuts`，让本地复制、粘贴、选区操作与远端 Shell 控制键一致分流；`@coterm/plugin-sdk/components/terminal-search` 提供本地搜索条，接收插件自己的搜索适配接口。安装与卸载约定见[终端快捷键](terminal-shortcuts.md)。这些纯前端模块不扩展 `PluginAPI` 或后端控制协议。

### 界面语言

桌面与插件的浅色/深色适配见[桌面界面与主题](desktop-appearance.md)。优先复用继承的 `--coterm-*` 颜色变量；xterm 等独立渲染器可使用 SDK 的 `components/terminal-theme` 原地更新配色，卸载时注销观察器。

基座提供简体中文与 English，首次优先读取保存的语言，否则按系统首选语言选择（中文变体使用简体中文，其余使用 English）。语言选择保存在本地 UI 偏好 `coterm.locale` 中，不包含工作区配置或秘密。切换时原地更新 UI，不卸载插件前端，不重建实例或连接。

`PluginAPI` 与 `PluginConfigurationAPI` 新增可选 `i18n` 能力：`i18n.locale` 为 `"en" | "zh-CN"`，`i18n.subscribe(listener)` 返回注销函数。它是 SDK v1.0 的可选增量，插件须检测存在性；旧宿主可回退 English。监听器只通知 UI 语言变化，不携带协议事件；插件卸载时应注销，基座也会按挂载作用域回收。SSH 的配置、连接状态和前端认证对话框已接入；未接入的插件、后端领域诊断、工具描述及远端输出保留原文。

| API | 当前状态 |
|---|---|
| `invoke/getOperation/recentOperations/cancelOperation` | 可用，绑定当前实例 |
| `approveProposal/rejectProposal` | 可用，基座校验提案与快照 |
| `subscribeEvents/subscribe` | 可用，有界 JSON 事件 |
| `requestText/requestSelection/requestConfirmation` | 宿主对话框；提交、拒绝、取消、超时与不可用显式区分 |
| `requestSecret` | 宿主密码框，前端只拿到一次性秘密引用 |
| `components.blocks` | 可选块列表；块数据、选择结果和上下文由插件管理 |
| `subscribeStream` | 预留，不支持；`api.capabilities.streams === false` |

`api.apiVersion === "1.0"` 是契约版本，与 npm 包版本独立。新增可选能力采用次版本并通过 `capabilities` 检测；破坏性修改升主版本，插件不应静默兼容未知主版本。

公共对话框每挂载作用域最多一个，全局最多 16 个；超时 1–300 秒，默认 60 秒。`requestConfirmation` 提交为 `true`，拒绝为 `rejected`；卸载会取消挂起交互并回收 SDK 订阅。普通文本不能代替秘密输入。后端 v1.0 仍只支持单轮秘密交互。

挂载时先订阅，再读取状态及回放快照；按版本、连接 ID 和游标接续。卸载必须注销订阅、DOM 监听器及自身 UI 资源，但不因此断开后端实例。

样式使用宿主 [CSS token](../../apps/desktop/src/index.css)，如 `var(--coterm-text-primary, #cccccc)`；Shadow DOM 继承变量并提供可读 fallback，不覆盖宿主变量。界面需适应窄容器并支持键盘操作，当前基线为深色主题。

## 开发与验收

- 参考 [SSH](ssh-plugin.md) 或 [串口](serial-plugin.md) 组织后端、前端、manifest 和构建脚本。
- 构建当前平台完整包到插件 `dist`，启动桌面，检查发现诊断与能力握手。
- 至少验证双实例隔离、配置错误、审批修改/拒绝、取消/超时、秘密隔离、慢消费者与输出缺口。
- 验证无 UI 插件仍可请求宿主交互；没有交互入口时明确失败，不能自动批准。
- 验证前端卸载后 MCP 仍可使用实例；插件崩溃或结果不明时不得自动重放写操作。
- 先运行 `cargo test -p coterm-proto --test control_contract`；新增插件另补真实进程、多实例与领域契约测试。流接口需用持续订阅型插件验证后独立冻结。

构建与检查命令见 [仓库 README](../../README.md)。数据归插件保存，宿主操作账本仅保留本次启动内的有界记录。Windows 工作区加密保存实例初始配置与布局；显式恢复仅重新创建实例，连接和写操作由用户重新发起。
