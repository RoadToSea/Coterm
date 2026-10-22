# Coterm 插件开发者指南

> **状态**：插件开发契约设计稿，尚未冻结为 v1 发布接口。本文不表示 SDK、安装命令或消息编码已经实现。

可运行示例见 [SSH 插件 MVP](ssh-plugin.md)，包括单文件前端构建、可选配置界面、人工秘密输入及当前限制。其受控适配器不代表本文所有运行时能力均已实现。

已建立供评审的[接口声明文件](interface-review.md)：Rust `coterm_proto::runtime` 和 TypeScript `packages/plugin-sdk`。以下历史示例仍用于解释设计，完整返回类型和消息清单以本轮接口评审入口为准，评审通过后统一更新示例。

本文面向通信协议插件的开发者。阅读本文后，开发者应能在不了解 Coterm 基座内部实现的前提下，划分插件模块、设计实例和数据模型、实现领域逻辑，并明确接入基座时需要满足的公开契约。

本文把基座视为黑盒，只描述插件可以依赖的公开契约。基座使用 Tauri、Rust、WebView 还是其他实现，不属于插件的依赖。

已确认的决策是：支持无 UI、多实例、插件自行保存数据、前后端经基座通信、单一 MCP Bridge、第一版可信前端同 WebView 加载。下文 JSON、消息名和 TypeScript 是接口草案示例；正式编码和发布插件前，还需冻结文末列出的接入细节。

## 阅读路径

| 开发者任务 | 阅读内容 |
|---|---|
| 确定插件模块和职责 | 第 1～4 节 |
| 编写协议后端和用户交互 | 第 5～6 节 |
| 添加可选界面 | 第 7 节 |
| 接入 Agent 并共享实例 | 第 8 节 |
| 规划持久化、验证和兼容性 | 第 9～13 节 |

建议让插件内部包含一个领域实现和两个入口：MCP 入口与界面调用入口共同调用领域实现；它们不各自创建独立连接。

```text
MCP 入口 ────────────┐
                     ├── 插件实例/领域实现 ── 协议连接
界面经基座转来的调用 ─┘          │
                                └── 插件自己的数据存储
```

## 1. 插件是什么

一个插件是一个独立的通信能力单元。它可以实现 SSH、UART、MQTT、CAN、Modbus 或任意自定义协议，并决定自己的数据模型和界面。

插件由以下部分组成：

```text
插件包
├── manifest.json       必需：插件身份和入口声明
├── backend/             必需：协议连接、数据和 MCP
├── frontend/            可选：插件自己的界面
└── schemas/             可选：工具、事件和视图数据 schema
```

插件必须自己负责：

- 建立和维护协议连接；
- 定义协议状态、操作、数据和错误；
- 管理自己的缓存、持久化和重连；
- 实现原生 MCP Tools、Resources、Prompts 或通知；
- 决定是否提供前端和如何展示数据；
- 在需要用户参与时向基座请求输入、秘密或确认。

插件不需要负责：

- 创建 Coterm 主窗口；
- 实现插件进程管理；
- 连接外部 MCP 客户端；
- 访问其他插件；
- 理解基座内部的 Tauri 或 Rust 实现。

## 2. 两种插件形态

### 2.1 有 UI 插件

有 UI 插件提供 `frontend` 入口。基座把它挂载到一个容器中，插件自行绘制完整界面。插件可以使用基座提供的通用组件，也可以完全自定义。

适用场景：

- SSH 终端和命令块；
- UART 原始流和发送面板；
- MQTT Topic 树和消息表格；
- CAN 报文分析器；
- 自定义设备控制面板。

### 2.2 无 UI 插件

无 UI 插件不提供 `frontend` 入口，只运行后端和原生 MCP。它仍然可以创建多个实例、保持连接、响应 Agent 调用、请求基座提供用户输入，并通过 MCP 返回结果。

适用场景：

- 后台订阅服务；
- 定时采集器；
- 只需要 MCP 的自动化协议；
- 暂时没有图形界面的开发中插件。

无 UI 不是特殊的兼容模式，而是正式支持的插件形态。

“插件无 UI”和“整个基座运行时没有交互界面”是两件事：插件可以没有自定义前端，仍借用基座输入框；当运行环境完全无人值守时，插件必须使用已授权的凭据，或对交互请求返回 `interaction_unavailable`，不能无限等待或自动批准。

## 3. 插件包和 manifest

插件包可以是目录，也可以是压缩归档。开发阶段建议使用目录，便于重新加载和调试；此处不承诺已有热更新工具。

示例目录：

```text
coterm-plugin-mqtt/
├── manifest.json
├── backend/
│   ├── windows-x64/mqtt-plugin.exe
│   └── linux-x64/mqtt-plugin
├── frontend/
│   ├── index.js
│   └── assets/
└── schemas/
    ├── operations.json
    └── events.json
```

manifest 的示例结构：

```json
{
  "manifestVersion": 1,
  "id": "com.example.mqtt",
  "name": "MQTT Connector",
  "version": "0.1.0",
  "runtime": {
    "interfaceMajor": 1,
    "requires": ["instances", "backend-calls", "events", "interactions", "mcp"]
  },
  "backend": {
    "windows-x64": "backend/windows-x64/mqtt-plugin.exe",
    "linux-x64": "backend/linux-x64/mqtt-plugin"
  },
  "frontend": {
    "entry": "frontend/index.js"
  },
  "mcp": {
    "provider": "backend"
  },
  "instances": {
    "type": "mqtt-connection",
    "multiple": true
  },
  "capabilities": [
    "user-input",
    "secret-input",
    "confirmation"
  ]
}
```

字段规则：

- `id` 在生态中必须唯一，建议使用反向域名格式；
- `version` 使用插件自己的版本，`manifestVersion` 标记清单格式，`runtime.interfaceMajor` 标记运行时接口主版本，三者独立；
- `runtime.requires` 声明必需的运行时特性；不兼容时基座拒绝启动并给出原因；
- `backend` 至少提供当前平台的可执行入口；
- `frontend` 可以省略；
- 示例中的 `mcp.provider: "backend"` 表示 MCP 实现位于同一个插件后端，不能再启动一份不共享状态的 MCP 专用连接进程；该字段不指定底层通信编码；
- `instances.multiple: true` 声明满足多实例契约，不应为了简化实现把多实例关闭；
- `capabilities` 只声明插件确实会使用的公共交互能力。

基座不会根据 manifest 推断协议语义。manifest 只用于发现、装配、兼容性检查和权限提示。

包内入口使用相对路径，不允许通过 `..` 跳出插件包；平台名、入口字段和版本字段的最终 JSON Schema 待冻结。插件应打包所需运行时或明确声明外部依赖，不假定目标机器安装了 Python、Node.js 或编译工具。

## 4. 实例模型

插件必须把每个连接或工作单元作为独立实例管理。基座为实例提供三个标识：

```text
pluginId          插件类型，例如 com.example.mqtt
pluginInstanceId  本次实例，例如 instance-42
sessionId         基座的交互会话上下文，不等于协议连接或 MCP 客户端会话
```

同一个插件可以同时运行多个实例，例如：

```text
串口 COM3 -> instance-1
串口 COM7 -> instance-2
串口 /dev/ttyUSB0 -> instance-3
```

实例之间必须隔离：

- 协议连接和配置；
- 操作请求和请求 ID；
- 数据流和订阅；
- 凭据引用；
- 前端状态；
- MCP 调用上下文。

插件可以采用一个进程管理多个实例，也可以每个实例启动一个进程。插件契约不依赖具体进程模型。

### 4.1 生命周期和界面生命周期

| 阶段 | 插件应做什么 |
|---|---|
| 后端启动和握手 | 协商接口版本与特性，注册处理器；未就绪前不接受领域调用 |
| 创建实例 | 以基座分配的 ID 建立实例状态；配置由插件校验；连接可以稍后由领域操作建立 |
| 调用和事件 | 只读写目标实例；界面与 Agent 操作访问同一插件实例 |
| 挂载/卸载界面 | 增减展示订阅；不能隐式创建、断开或销毁协议连接 |
| 销毁实例 | 停止新调用，处理未完成交互，关闭协议资源并清理订阅 |
| 后端退出 | 释放其余资源；重新启动不得重放上次未确认的写操作 |

界面关闭、隐藏、切换标签、重新挂载与销毁实例是不同动作。插件前端卸载后，后台采集或 MCP 调用可以继续；销毁实例必须由明确的生命周期请求触发。新的实例 ID 不得意外关联到旧连接。

### 4.2 操作与编辑

用户和 Agent 不会并行编辑同一份输入。用户在插件界面填写并主动发送；Agent 通过 MCP 提交操作。Agent 的 commit 默认先进入基座审批，界面展示可修改的参数快照，用户批准后才发送。用户也可以直接在界面修改当前内容后发送。插件按协议定义操作边界和并发策略，例如 SSH 命令串行、UART 写入串行、MQTT 发布与订阅并行；冲突时明确排队或返回忙碌错误，不能静默混写输入。

## 5. 后端运行时契约

运行时通信的完整设计、逻辑消息、帧边界、超时/取消、交互、流背压和断线规则见[基座与插件后端运行时通信](runtime-communication.md)。本节保留插件作者需要实现的最小语义和示例；消息名称是逻辑 API，不是已经冻结的线缆格式。

插件后端通过基座提供的运行时通道工作。开发者只需要实现以下概念，不需要知道通道的内部实现：

```text
生命周期：启动、创建实例、停止实例、健康状态
请求：一次有明确结果的领域操作
数据流：持续的文本、二进制或结构化事件
交互：向用户请求输入、秘密、确认或取消
```

基座发送给插件的公共消息：

```text
start
create_instance
destroy_instance
invoke
cancel
interaction_result
shutdown
```

插件发送给基座的公共消息：

```text
ready
instance_status
operation_result
operation_error
stream_open
stream_data
stream_close
interaction_request
log
```

以下标识按消息类型携带，不要求每条消息包含全部字段：

```text
requestId       请求和结果关联
pluginInstanceId 实例关联
streamId        数据流关联，可选
interactionId   用户交互关联，可选
```

这些名称描述逻辑消息，不是最终线缆协议。例如后续可将 `invoke` 编码为 JSON-RPC 请求，但不能直接把本文的 `type` 示例当成 JSON-RPC 2.0 报文。公共 SDK 应封装编码和传输，插件作者只依赖上面的调用语义。

### 5.1 请求

请求承载插件自己的领域操作。基座只负责传递、超时、取消和关联；它会在握手时校验 schema，并在提交和批准时验证参数类型、必填字段及其他声明约束。业务语义仍由插件负责：

```json
{
  "type": "invoke",
  "requestId": "req-17",
  "pluginInstanceId": "instance-1",
  "operation": "mqtt.publish",
  "args": {
    "topic": "device/cmd",
    "payload": "restart",
    "qos": 1
  }
}
```

SSH 插件可以使用 `ssh.exec_command`，UART 插件可以使用 `serial.write`，自定义协议可以使用自己的操作名。操作的 schema 和业务错误由插件定义。

调用约定：

- `requestId` 在当前后端运行期内唯一，结果或错误只能有一个终态；
- 基座提供可识别的调用来源和目标实例，不能信任领域 `args` 自行声称“来自人类”；
- 超时表示调用方不再等待，不表示设备操作已撤销；
- 取消是尽力请求，插件应报告“取消前尚未提交”“已停止”或“已提交且结果未知”；
- 基座不自动重试有副作用的调用；插件需要重试时，自行定义幂等键和去重策略；
- 错误与结果只能发回发起请求的一方；需要同步给界面的状态另发事件。

当前 MVP 的延迟操作与事件接口：

- manifest/握手声明 `async-operations`、`plugin-events`，控制方法须在 10 秒内应答。长操作返回 `{ "_cotermDeferred": "operation-id" }`，随后发送 JSON-RPC 通知，`method="coterm.notification"`，`params={"type":"operation_completed","operationId":"operation-id","result":{...}}`。无法确认执行结果时使用 `operation_failed` 和公共 `RuntimeError`，基座记为 `unknown`；明确的领域失败也可以作为完成结果中的错误返回。
- 界面事件使用同一通知方法，`params={"type":"event","pluginInstanceId":"instance-id","name":"自定义事件名","payload":{...}}`。事件不得包含秘密，持续字节须分成有界小块。当前适配器是 JSON，正式二进制流另行实现。
- 前端使用 `await api.subscribeEvents(handler)`，订阅安装完成后再读取快照。handler 可返回 Promise，处理完成后才确认投递；返回的函数负责取消。收到 `runtime.gap` 后自行按领域游标恢复，收到 `runtime.closed` 后处理通道关闭。基座不理解协议数据结构。
- 操作目录可声明 `transient: true`，供后台快照读取、按键及 resize 使用。它们仍经宿主来源/权限校验，进入独立的有界记录区，不占普通最近 N 步；需要审批时自动进入普通区。MCP 的等待选项及示例见 [等待与立即返回](mcp-stdio.md#等待与立即返回)。

### 5.2 数据流

数据流用于终端输出、串口字节、MQTT 消息、CAN 报文等持续数据。插件负责数据的协议级缓存和解析，基座负责承载、关联和传递。

```json
{
  "type": "stream_data",
  "streamId": "stream-3",
  "pluginInstanceId": "instance-1",
  "sequence": 18,
  "timestamp": 1792100000000,
  "contentType": "application/json",
  "payload": {
    "topic": "device/status",
    "value": { "online": true }
  }
}
```

`payload` 可以是文本、二进制或结构化数据。长期订阅和高频数据必须支持关闭、取消、背压或有界缓存；插件不能假设基座会无限保存数据。

数据通道约定：

- 结构化载荷使用 JSON 安全类型；二进制必须声明编码或使用协商的二进制通道，不能把任意字节当 UTF-8 字符串；
- `sequence` 在单条流中递增，不能用跨实例或跨流的全局顺序推断业务先后；`timestamp` 只表示记录时间；
- 基座只承载有界的在途队列，窗口大小和溢出行为由握手协商；不能默认无限缓存；
- 界面订阅取消只停止该界面的投递，不自动取消 MQTT 订阅或物理连接；
- 慢消费者、重新挂载和重启可能产生缺口；插件应提供快照或带游标的历史读取，并明确 `gap`、已过期或已截断；
- 不承诺跨崩溃的恰好一次投递。需要完整日志的插件应先保存到自己的存储，再提供读取入口。

例如 UART 插件可以按块投递字节，同时在本地保存原始数据；MQTT 插件可以按消息投递并用自己的游标恢复。基座不替它们决定保留策略。

### 5.3 状态和错误

插件对基座报告生命周期状态，协议状态放在插件自己的数据中。下面是建议状态词，字段和状态集合仍需冻结：

```text
starting
ready
busy
waiting_for_user
disconnected
failed
stopping
```

实例状态与操作状态应分开：实例保持 `ready` 时可以有一个操作正在等待用户，不能让整个实例因此停止处理取消和其他交互。

| 错误类别 | 插件或调用方的处理 |
|---|---|
| 参数错误/实例不存在 | 修正参数或重新选择实例 |
| 不支持的功能/版本不兼容 | 拒绝调用并报告缺失特性 |
| 忙碌/资源冲突 | 等待、取消或明确改用另一个实例 |
| 认证失败/连接失败/协议错误 | 交给插件决定是否重连，避免盲目重试 |
| 用户拒绝/取消/交互不可用 | 结束等待，不隐式批准 |
| 超时/插件断开 | 返回明确错误，标记已提交操作可能未知 |
| 插件内部错误 | 隐去秘密与内部敏感信息，保留诊断关联 ID |

错误应包含稳定的 `code`、可读的 `message` 和可选的插件领域 `details`。公共传输错误与插件领域错误分开，前端不要靠解析错误文案来控制流程。已经提交到底层设备但未收到响应的操作不能报告为成功。

## 6. 用户交互

插件遇到需要用户参与的步骤时，向基座发起交互请求。插件不需要创建密码弹窗，也不需要知道基座使用何种 UI。

```json
{
  "type": "interaction_request",
  "interactionId": "interaction-8",
  "pluginInstanceId": "instance-1",
  "requestId": "req-17",
  "kind": "secret",
  "title": "MQTT authentication",
  "prompt": "Password",
  "timeoutMs": 60000,
  "allowCancel": true
}
```

支持的交互类型包括：

```text
text        普通文本
secret      密码、Token、密钥等秘密
confirmation 确认高风险操作
selection   从插件提供的选项中选择
```

文件选择涉及跨平台路径、访问授权和大文件传输，单独作为可选扩展；不要把普通选项字符串直接当成文件访问授权。取消是每种交互和操作的结果，不需要弹出一种独立的“取消输入”表单。

交互结果必须区分 `submitted`、`rejected`、`cancelled`、`timed_out`、`unavailable`。后端等待期间仍须处理其他请求和取消；不得阻塞整个消息循环。发起操作取消、实例销毁或后端退出时，相应交互也结束，迟到的输入不得流向其他实例。

原始秘密只通过受控通道交给目标插件后端，或返回仅该实例可使用的短期引用；不能交给 Agent 和插件前端的普通状态。秘密引用按实例、用途和有效期约束，失效后重新申请。插件应在使用完成后尽快释放秘密，不得写入普通事件、日志、MCP 结果或明文持久化数据。

秘密输入不会让不可信插件变得可信：插件后端最终需要使用凭据，因此不向外泄漏也是插件必须遵守的契约。永久保存凭据应使用系统安全存储，不能写入插件配置或历史数据库。

## 7. 插件前端

前端是可选的。提供前端时，插件导出：

```typescript
export function mount(
  container: HTMLElement,
  api: PluginAPI
): { unmount(): void };
```

`PluginAPI` 的作用是让前端通过基座访问自己的后端和公共交互：

```typescript
interface PluginAPI {
  pluginId: string;
  pluginInstanceId: string;
  sessionId: string;

  invoke(operation: string, args: unknown): Promise<unknown>;
  subscribe(event: string, handler: (event: unknown) => void): () => void;
  subscribeEvents(handler: (event: { name: string; payload: unknown }) => void | Promise<void>): Promise<() => void>;

  requestText(request: TextRequest): Promise<string>;
  requestSecret(request: SecretRequest): Promise<SecretHandle>;
  requestConfirmation(request: ConfirmationRequest): Promise<boolean>;

  components: CommonComponents;
}
```

上面的类型用于说明职责，`TextRequest` 等完整类型尚待 SDK 接口冻结。这里的 `subscribe` 订阅插件界面事件，与 MQTT 的协议订阅无关。

基座向实例 API 注入可选的通用块列表组件 `api.components.blocks(container, props)`。插件传入自己识别和保存的块，字段包括稳定 `id`、显示序号、命令摘要、`human`/`robot` 来源和可选退出码；组件以左侧编号栏、选中颜色标记和来源文字呈现。`update(props)` 更新列表，`unmount()` 清理挂载。选择变化通过 `onSelectionChange(selectedBlockIds)` 回传插件，插件决定选择如何影响自己的 MCP 上下文。该组件不切分协议数据、不保存块，也不授权操作；来源标签只是展示值，插件应从可靠的调用上下文生成，基座的安全审批仍以运行时识别的来源为准。插件可完全不使用它并实现自有界面。

前端必须通过 `PluginAPI` 访问后端，不得直接创建本地连接、访问插件进程、读取其他插件状态或调用 Tauri 内部 API。基座应将 API 绑定到当前实例，调用方不从输入参数中选择另一个实例。协议后端独立校验目标资源，不能把前端传来的标识视为授权。

### 7.1 最小自定义界面示例

下面是使用上述草案接口的普通 JavaScript 示例，不依赖基座的前端框架。`serial.write` 和 `serial.received` 是示例串口插件自己定义的操作和事件：

```javascript
export function mount(container, api) {
  const input = document.createElement("input");
  const send = document.createElement("button");
  const output = document.createElement("pre");
  send.textContent = "发送";
  container.replaceChildren(input, send, output);

  let disposed = false;
  const listeners = new AbortController();
  const unsubscribe = api.subscribe("serial.received", (event) => {
    if (!disposed) output.textContent += event.text;
  });

  send.addEventListener("click", async () => {
    try {
      await api.invoke("serial.write", { text: input.value });
    } catch (error) {
      if (!disposed) output.textContent += `\n发送失败：${String(error)}`;
    }
  }, { signal: listeners.signal });

  return {
    unmount() {
      disposed = true;
      listeners.abort();
      unsubscribe();
      container.replaceChildren();
    }
  };
}
```

示例只展示挂载、调用、订阅和清理。实际串口流可能是二进制，需要插件自己的解码和有界展示缓存；不能无限追加字符串。后端在 `serial.write` 内执行并发和权限检查，发送按钮不是执行权限的依据。

### 7.2 界面恢复和清理

首次挂载或重新挂载时，前端向后端查询当前状态，再按版本/游标接续事件，避免把历史当成新消息。具体快照与订阅的衔接由插件定义，不能假定先查询后订阅一定不丢数据。

卸载时释放事件订阅、DOM 监听、定时器和前端缓冲；忽略已销毁视图的异步结果。界面卸载不销毁后端实例。样式应限定在容器或插件命名空间内，不修改基座和其他插件的全局样式。

前端可以使用以下可选通用组件：

```text
TerminalView
CommandBlockList
StreamView
LogView
TableView
KeyValueView
```

组件只负责展示和通用交互，不要求插件把自己的数据改造成统一模型。插件也可以完全不用这些组件。

第一版前端加载使用同一 Tauri WebView 的受控模块方式，适合受信任和本地开发插件。它不是安全沙箱；插件市场的签名、权限、iframe 或独立 WebView 隔离属于后续改进。

### 7.3 共享视觉 Token 与 Shadow DOM 样式契约

为了保持桌面与独立插件界面的统一感知，基座在根样式（`apps/desktop/src/index.css` 的 `:root`）与 Tailwind 体系中定义了一组轻量视觉 token，并支持通过 CSS Custom Properties 穿透 Shadow DOM 边界进行继承。

#### 7.3.1 Token 命名体系

- **Surface 背景**：
  - `--coterm-surface`：主工作区与终端背景（默认 `#1e1e1e`）
  - `--coterm-surface-raised`：面板、对话框、弹窗卡片背景（默认 `#252526`）
  - `--coterm-surface-panel`：二级分区与表头背景（默认 `#2d2d2d`）
  - `--coterm-surface-sidebar`：插件图标导航栏（默认 `#333333`）
  - `--coterm-surface-toolbar`：顶部工具栏（默认 `#323233`）
  - `--coterm-surface-hover`：列表与按钮悬浮态（默认 `#2a2d2e`）
  - `--coterm-surface-active`：当前选中项/激活态（默认 `#37373d`）
- **Border 边框**：
  - `--coterm-border`：主结构分割线（默认 `#2b2b2b`）
  - `--coterm-border-subtle`：内部次级线条（默认 `#383838`）
  - `--coterm-border-input`：输入框与通用按钮边框（默认 `#454545`）
- **Text 文本**：
  - `--coterm-text-primary`：正文主要文本（默认 `#cccccc`）
  - `--coterm-text-muted`：次要说明与辅助提示文本（默认 `#969696`）
  - `--coterm-text-subtle`：表单标签与弱强调文本（默认 `#aaaaaa`）
  - `--coterm-text-bright`：标题、按钮高亮文字（默认 `#ffffff`）
- **Accent 强调色**：
  - `--coterm-accent`：主要操作背景（默认 `#0e639c`）
  - `--coterm-accent-hover`：主要操作悬浮态（默认 `#1177bb`）
  - `--coterm-accent-active`：选中条目底色（默认 `#094771`）
  - `--coterm-accent-glow`：活跃状态指示圆点与高亮图标（默认 `#75beff`）
- **Danger 危险/错误**：
  - `--coterm-danger`：错误文本与失败退出码（默认 `#f48771`）
  - `--coterm-danger-surface`：错误警报背景（默认 `#332626`）
  - `--coterm-danger-border`：错误警报边框（默认 `#65433d`）
- **Focus 焦点与排版**：
  - `--coterm-focus`：统一焦点环边框色（默认 `#007fd4`）
  - `--coterm-font-sans`：系统无衬线字体序列（默认 `"Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif`）
  - `--coterm-font-mono`：等宽控制台与代码字体（默认 `Consolas, "Liberation Mono", Menlo, Courier, monospace`）

#### 7.3.2 Shadow DOM 注入与 Fallback 契约

Web Components Shadow DOM 拥有天然的样式封装壁垒，外部普通的类选择器无法渗透。但 CSS Custom Properties（变量）依照规范可自动穿透 Shadow Root 继承。

插件编写前端样式及使用 `@coterm/plugin-sdk` 通用组件时，必须遵守以下约定：

1. **必须携带 Fallback**：所有引用均采用 `var(--coterm-*, <default>)` 形式，如 `color: var(--coterm-text-primary, #cccccc)`。当插件在独立测试环境、脱离宿主或在早期加载未注入变量时，回退至可读、高对比度的深色默认值。
2. **拒绝硬编码覆写**：插件 Shadow DOM 内部不要无故覆盖根变量名，以免阻断上层宿主的主题下发。
3. **主题现状说明**：当前 Coterm 视觉基线为 VS Code 风格深色主题（Dark Theme）。浅色主题（Light Theme）与动态换肤尚不在当前阶段交付范围内，但所有样式已全部解耦至语义 token，未来基座切换全局主题时无需修改插件代码。

#### 7.3.3 窄窗口与响应式两栏布局行为

基座支持灵活的响应式折叠与展开：
- **宽屏（>=1024px，如 1280px）**：插件图标栏（64px/72px）与实例列表栏（200px/240px）完整呈现，主工作区获取充足自适应空间。
- **窄屏（<900px，如 768px）**：实例列表栏自动响应式折叠隐藏，主工作区独占剩余宽度，优先保留终端/数据可视化所需横向空间。
- **展开与恢复交互**：顶部工作区工具栏常驻恢复/展开控制按钮（包含 `aria-controls="instances-sidebar"`、`aria-expanded`、`aria-label` 无障碍属性），支持纯键盘（Tab/Enter/Space）无障碍操作；实例栏头部同时提供折叠按钮，杜绝不可逆收起。

## 8. 原生 MCP

当前固定 URL 接入方式见 [固定 MCP 连接](mcp-http.md)，操作等待和秘密交互见 [MCP Tools 与后端秘密输入](mcp-stdio.md)。插件通过握手 `mcpTools` 显式提交原生工具及内部操作映射，完成时返回原生 MCP CallToolResult；不自动公开整个操作目录。实例级工具 schema 要求显式 `pluginInstanceId`；插件级工具不应声明实例目标，宿主从已注册工具命名空间构造 `pluginId` 授权上下文。后端秘密交互使用协商后的 `_cotermInteraction` 控制应答。Resources、Prompts、订阅等 MCP 能力仍按下述目标架构设计。

插件后端自己实现 MCP Server。工具和资源的名称、参数、返回值和协议语义完全由插件决定：

```text
ssh.exec_command
ssh.send_input
mqtt.subscribe
mqtt.publish
serial.write
device.read_status
```

Coterm 对外提供一个 MCP Bridge。插件开发者只实现自己的 MCP Server；Bridge 负责聚合目录、命名空间和路由，不重新实现领域工具。

MCP 调用和插件界面必须使用同一个 `pluginInstanceId`。当 MCP 调用需要密码或确认时，插件通过运行时交互请求暂停，等待用户响应后继续。

### 8.1 让 Agent 定位正确实例

多个实例存在时，不允许用“当前标签页”或“最后一个连接”隐式决定 MCP 目标。否则用户切换标签可能让 Agent 操作另一块设备。

建议的契约是：插件提供实例发现工具，并在实例级领域工具的输入 schema 中显式声明 `pluginInstanceId`。例如：

```text
插件 MCP：list_instances()
  -> [{ pluginInstanceId: "instance-1", label: "COM3" },
      { pluginInstanceId: "instance-2", label: "COM7" }]

插件 MCP：write({ pluginInstanceId: "instance-2", text: "status\r\n" })
  -> 只写入 COM7
```

实例清单来源于运行时关联；新建和销毁仍经过基座分配/释放实例标识，插件不能另建一套 Agent 专用实例表。实例不存在时返回错误，不自动切换到另一连接。

`pluginInstanceId` 由插件自己的工具 schema 声明和校验，Bridge 不在转发过程中悄悄插入领域参数。Bridge 映射外部工具名和协议请求 ID 等路由信息，工具的领域参数和结果保持插件定义。

### 8.2 SSH 密码交互的完整路径

```text
1. 用户或 Agent 创建一个 SSH 实例，得到 instance-1。
2. 前端挂载到 instance-1；Agent 的 MCP 调用也明确指定 instance-1。
3. 插件领域实现执行命令或连接，发现需要认证输入。
4. 插件向基座发起与实例、请求关联的 secret 交互。
5. 基座收集用户输入，经秘密通道交给该插件实例。
6. 插件恢复操作，向界面发出脱敏状态/输出，并向 MCP 返回结果。
7. 若用户取消、运行环境无交互入口或等待超时，操作以对应错误结束。
```

SSH 的非交互 exec 通道通常有结束和退出状态；交互 Shell 未必有明确命令边界，需要插件自行处理 Shell 集成。UART 则可以完全没有命令或退出码模型，两者不影响运行时接入。

### 8.3 MQTT 长期订阅

`subscribe` 可以返回插件自己的订阅 ID，后续用资源、通知或游标读取消息；界面通过基座订阅同一后端的数据事件。关闭消息面板只取消界面的事件投递，只有明确的协议 `unsubscribe` 才终止 Broker 订阅。订阅恢复、消息过期和游标语义均由插件实现。

## 9. 数据和持久化

插件拥有自己的领域数据。插件可以使用自己的数据库、文件格式或远端存储保存：

- 连接配置和协议参数；
- 订阅、消息、命令或报文历史；
- 设备状态和缓存；
- 插件专属 UI 状态。

基座只保存运行时索引、插件安装信息和路由所需的最小元数据。插件不要依赖基座提供统一数据库，也不要假设基座理解自己的数据结构。

插件使用独立的可写数据目录，按实例或配置档案组织持久化数据，不往安装包目录写历史。基座可以提供目录位置等运行环境信息，但文件格式、迁移、配额、备份和清理由插件负责。秘密只保存为系统安全存储引用。

## 10. 本地开发流程

插件开发者可以按以下顺序验证插件：

1. 在 manifest 中声明插件 ID、后端入口和实例类型；
2. 单独启动后端，使用测试设备或模拟设备验证协议逻辑；
3. 验证后端可以创建两个实例，并确保实例状态互不影响；
4. 验证 MCP Server 可以独立调用工具和返回领域结果；
5. 接入基座运行时，验证用户输入、秘密和确认请求；
6. 添加前端模块（如果需要），通过 `PluginAPI` 访问后端；
7. 通过 Coterm MCP Bridge 验证 Agent 调用和 UI 会话共享同一实例；
8. 注入断线、超时、取消、插件崩溃和未确认操作，检查错误状态。

## 11. 插件验收清单

插件发布前至少应满足：

- 无 UI 模式可以正常启动和提供 MCP；
- 多实例不会混用连接、数据、凭据或 UI 状态；
- 所有前端后端调用都经过 `PluginAPI`；
- 所有长期数据流都有关闭、取消或有界缓存策略；
- 用户交互有超时、拒绝和取消处理；
- 原始秘密不出现在日志、事件、MCP 返回值和明文持久化数据中；
- MCP 调用失败时返回明确错误，不伪造成功；
- 插件停止或崩溃后，资源可以释放，未完成操作会报告状态未知；
- manifest 的版本、平台入口和运行时兼容范围正确。

建议使用以下场景验收公开契约，测试时可以替换为模拟协议，不需要启动真实硬件：

| 场景 | 必须观察到的结果 |
|---|---|
| 同时创建两个串口实例 | 前端调用、MCP 调用和字节事件都只到达各自实例 |
| 用户和 Agent 同时写同一连接 | 遵循插件声明的排队/冲突策略，没有静默混写 |
| MCP 操作等待密码，随后取消 | 等待结束，交互撤销，迟到密码不被其他操作使用 |
| 插件无前端，基座有界面 | 能使用基座公共交互并经 Bridge 提供 MCP |
| 整个运行环境无交互入口 | 返回交互不可用，不无限等待或自行批准 |
| 关闭前端后继续 MCP 调用 | 实例仍存在，协议连接不因界面卸载断开 |
| 消费速度低于数据产生速度 | 内存有界，缺口/截断可见，历史从插件读取 |
| 写操作提交后插件崩溃 | 返回断开或结果未知，不自动重复写入 |

## 12. 版本和扩展约定

长期兼容依赖接口版本与特性协商，不能依赖插件读取基座源码适配：

- 插件版本、manifest 格式版本、运行时接口版本和 MCP 协议版本分别管理；
- 启动时校验必需特性，缺少时拒绝启动；可选特性在探测后启用；
- 同一主版本内只增加可选字段或可选特性，不改变既有字段含义；
- 接收方忽略可兼容的未知元数据字段；遇到未知必需特性或方法返回“不支持”，不能猜测执行；
- 插件领域操作、事件和数据库自行版本化；破坏性改动提供迁移或新版本入口；
- 弃用先记录替代方式和兼容窗口，公共名称不直接删除。

接口冻结前应至少用命令型和持续流/订阅型两种插件走通契约。前端隔离未来可以改变传输方式，但保留生命周期、实例关联、调用和事件的可观察行为；不可序列化的 DOM 挂载和通用 UI SDK 单独版本化。

## 13. 接口冻结前还需补齐的接入规范

本文完成插件设计层的契约；以下内容仍未在讨论中最终确定，因此不能假定已有可运行 SDK：

| 接入项 | 冻结时需交付的公开材料 |
|---|---|
| manifest | JSON Schema、平台标识、完整示例和验证方式 |
| 后端通道 | 启动参数、握手、消息编码/帧边界、stdout/stderr 用途及通道关闭行为 |
| 高频数据 | 二进制编码、分块上限、背压协商和溢出错误 |
| 公共交互 | 完整请求/结果类型、秘密引用使用方式和无交互环境行为 |
| MCP 接入 | 同进程 MCP 与运行时的通道复用方式、工具/资源命名空间、动态目录与实例发现规范 |
| 前端 SDK | 可导入的类型、模块构建约定、资源加载方式及通用 UI SDK 版本 |
| 开发工具 | 插件校验、加载、重载、运行时模拟和打包的实际命令 |

这些材料必须面向插件作者发布，不能要求作者阅读基座实现来补齐。本文中的草案示例与正式 Schema/SDK 应在接口冻结时统一，未通过至少两类插件验证前不宣称 v1 已稳定。
