# 插件运行时与前端模块

> 本文解释基座与插件的架构关系。插件作者从 [插件开发者指南](plugin-development.md) 开始阅读即可；公开接口示例和行为契约以该指南为集中入口，当前仍为待冻结的设计稿。

## 1. 目标

插件是 Coterm 的主要扩展单元。一个插件应能够独立实现一种通信协议，提供协议后端和原生 MCP，并按需提供领域界面。基座只提供让这些部分共同运行的最小运行时。

本方案解决三个问题：

1. 插件后端如何被基座启动和管理；
2. 插件前端如何挂载到 Tauri WebView；
3. 插件如何请求用户输入、凭据和确认，并与自己的协议会话关联。

本方案不规定 SSH、UART、MQTT 等协议的内部数据模型。

## 2. 插件包

插件是一个目录包或等价的可分发归档。前端目录是可选的，无 UI 插件可以只运行后端和原生 MCP：

```text
plugin-package/
  manifest.json
  backend/
    <platform-specific executable>
  frontend/                 # 可选
    index.js
    assets/
  icon/
  schemas/
```

manifest 至少描述：

- 插件 ID、版本和兼容的运行时版本；
- 后端入口和支持的平台；
- 可选的前端入口模块；
- MCP Server 的启动或连接信息；
- 插件可创建的实例类型；
- 所需的公共运行时能力；
- 可选的资源限制和数据大小上限。

manifest 描述如何装配插件，不描述协议的完整语义。协议工具、消息模型和视图数据 schema 由插件自己实现和维护。

## 3. 插件实例

基座为每个打开的连接或插件工作单元分配实例标识。同一个插件必须能够创建和管理多个并行实例，例如同时打开多个串口终端：

```text
plugin_id
plugin_instance_id
session_id
```

插件后端、插件前端和 MCP 调用都通过 `plugin_instance_id` 关联到同一个实例。实例之间的协议状态、凭据引用、事件流、MCP 调用和界面状态必须隔离。

插件后端拥有实例的协议数据和持久化责任。基座不建立统一的协议数据仓库；基座只保存运行时所需的实例索引、生命周期状态和临时路由信息。

是否“一进程多实例”或“每实例一个进程”属于运行时实现选择。涉及凭据、崩溃影响和资源隔离时，优先选择更清晰的实例隔离边界。

## 4. 前端受控模块加载

“受控模块加载”指插件前端作为 Web 模块挂载到基座创建的容器中：

```text
Coterm WebView
  └── <div id="plugin-root">
        └── plugin frontend module
```

插件前端导出统一的生命周期函数：

```typescript
export function mount(
  container: HTMLElement,
  api: PluginAPI
): PluginInstance;

interface PluginInstance {
  unmount(): void;
}
```

基座负责解析 manifest、定位入口、创建容器、传入实例上下文、调用 `mount`，以及在释放视图时调用 `unmount`。插件负责容器内部所有 DOM、状态、事件和布局。视图卸载不等于销毁实例或断开协议连接；没有前端入口的插件不创建界面容器，仍可以被 MCP 和后台运行时使用。

这不是基座规定的统一页面模板。插件可以使用 React、Vue、Svelte、原生 Web Components 或其他适合 WebView 的技术，只要实现运行时约定的挂载接口。

## 5. PluginAPI

基座只向插件前端传入绑定当前实例的公共 API。插件前端访问后端、订阅数据、发起用户交互都必须经过该 API；插件前端不能直接访问本地进程、文件系统、其他插件或宿主内部状态。草案类型和挂载示例统一维护在 [插件开发者指南：插件前端](plugin-development.md#7-插件前端)，避免多份声明发生偏差。

`invoke` 和 `subscribe` 只表示“调用插件后端”和“订阅插件事件”，不规定参数的领域含义。SSH 插件可以传递命令参数，MQTT 插件可以传递订阅参数，自定义插件可以传递自己的 schema。

秘密输入返回短期 `SecretHandle` 或等价的受控引用，而不是把原始密码暴露给前端普通状态。插件后端通过运行时通道使用该引用，普通事件和日志不得包含秘密内容。

## 6. 通用组件

基座可以提供可选的组件 SDK，但组件不构成领域协议标准：

```text
TerminalView
CommandBlockList
StreamView
LogView
TableView
KeyValueView
SecretInput
ConfirmationDialog
```

例如，SSH 插件可以使用 `TerminalView` 和 `CommandBlockList`；MQTT 插件可以使用 `TableView`；自定义通信协议可以完全跳过组件 SDK，绘制自己的界面。

组件只能消费插件提供的数据和事件，不能要求基座理解这些数据的协议语义。`CommandBlockList` 是终端类插件的可选能力，不是 Coterm 的全局命令模型。

## 7. 后端运行时通道

基座与插件后端只统一生命周期、请求、数据流和用户交互的承载。逻辑消息、关联标识、帧边界、取消、错误、背压和断线契约统一维护在[基座与插件后端运行时通信](runtime-communication.md)；插件作者可以通过[插件开发者指南：后端运行时契约](plugin-development.md#5-后端运行时契约)按 SDK 语义接入。

插件后端负责协议级解析、缓存和持久化；插件前端通过基座运行时订阅数据并自行呈现。界面事件订阅与协议业务订阅分开，关闭界面不会自动取消 Broker 订阅或关闭物理连接。

第一版物理实现建议采用插件进程 stdio 上的明确长度帧：控制帧使用 JSON，持续二进制使用数据帧，由 SDK 隐藏编码细节。高频协议数据需要有界队列、背压、取消和断线语义，不能要求所有数据都逐条封装成控制请求。完整契约见[运行时通信设计](runtime-communication.md)。

## 8. 与插件 MCP 的关系

插件后端拥有原生 MCP Server。插件的 MCP 调用和插件前端使用同一插件实例，因此 MCP 调用需要用户输入时，可以通过运行时通道向基座发起交互请求。

```text
Agent -> plugin MCP tool
      -> plugin backend
      -> host interaction request
      -> user input in plugin UI or host dialog
      -> plugin backend continues
```

MCP 工具的参数和结果由插件定义。基座只负责注册、路由、实例关联、生命周期和公共安全交互。

## 9. 第一版实现与后续改进

第一版采用同一 Tauri WebView 中的动态模块加载：基座把插件前端挂载到指定 `div`，通过 `PluginAPI` 注入实例上下文和运行时能力。这条路径实现成本低，适合先验证多实例、用户交互和插件 MCP 的共同会话。

第一版明确接受以下限制：同一 WebView 中的插件不是强安全沙箱，插件前端可能影响同一页面的 DOM、CSS 和资源。后续面向不可信第三方插件时，再评估 iframe、独立 WebView、签名验证、权限限制和资源配额，并保持 `PluginAPI` 作为上层兼容边界。

## 10. 加载方式的信任边界

同一 WebView 中的模块加载主要提供 API 边界和生命周期管理，不提供强安全隔离。第一版只面向受信任或本地开发插件，以获得简单的通用组件复用和较低的跨平台成本。

面向不可信第三方插件时，需要另行采用 iframe、独立 WebView、签名验证、权限限制和资源配额。插件市场不能把“同一 WebView 的模块加载”当成安全沙箱。
