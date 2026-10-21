# 插件原生 MCP 集成

> 插件作者的实例定位、工具入口和用户交互流程见 [插件开发者指南](plugin-development.md#8-原生-mcp)。具体传输和命名规范仍需接口冻结，本文件记录架构职责。

当前已实现固定本机地址的 MCP Streamable HTTP、实例级及插件级 Tools 和后端秘密交互。用户只填写 URL，无需 Token；应用重启后地址不变。传输由基座提供，插件无需各自实现 HTTP 服务。入口与访问边界见 [固定 MCP 连接](mcp-http.md)，工具和交互细节见 [MCP Tools 与秘密输入](mcp-stdio.md)。Resources、Prompts、订阅及完整双向路由仍是目标架构。

## 1. 目标

每个通信插件原生实现自己的 MCP 能力。Coterm 不为插件重新包装一层通用通信 MCP，也不要求 SSH、UART、MQTT 使用相同的工具模型。

## 2. MCP 归属

插件后端是领域 MCP Server 的实现者：

```text
SSH Plugin:
  ssh.connect
  ssh.exec_command
  ssh.send_input

MQTT Plugin:
  mqtt.connect
  mqtt.subscribe
  mqtt.publish
  mqtt.read_messages

Custom Plugin:
  custom_device.read_status
  custom_device.configure
```

工具的名称、参数、返回值、订阅语义和错误模型由插件维护。插件可以自行支持 MCP Tools、Resources、Prompts 或通知，不需要基座替它们翻译成统一领域模型。

## 3. Coterm MCP Bridge

Coterm 对外提供一个 MCP 入口。这个入口由一个很薄的 MCP Bridge 实现，避免用户为每个插件单独配置 MCP 进程。Bridge 的职责只有：

- 发现已启动的插件 MCP Server；
- 为工具和资源增加插件命名空间；
- 转发请求和响应；
- 关联插件实例和 Coterm 会话；
- 处理插件启动、断开和取消。

Bridge 不修改插件工具的领域参数和返回值，也不把所有工具包装成 `execute`、`read_output` 等统一接口。原样转发指保留领域语义；工具名、资源 URI 和请求 ID 等路由信息仍可能需要映射。

实例级工具必须由工具 schema 声明 `pluginInstanceId`，并提供实例发现入口；插件级工具不带实例目标，由宿主根据注册工具所属插件构造授权上下文。不能用当前标签页隐式选择 MCP 操作目标，也不能另建一条与用户界面无关的连接。

```text
Agent
  -> Coterm MCP Bridge
  -> ssh.exec_command
  -> SSH Plugin MCP Server
```

无 UI 插件也必须能够通过这个单一 MCP 入口运行。插件可以在开发和调试时独立启动自己的 MCP Server，但这不是 Coterm 图形模式下的集成方式；图形模式下，插件由基座启动并与插件实例绑定到同一 Bridge。

## 4. 用户交互

MCP 调用不意味着插件可以绕过用户。插件在需要密码、确认、文件选择或人工接管时，通过插件运行时向基座发起交互请求：

```text
MCP call
  -> plugin decides interaction is needed
  -> host displays a request
  -> user responds
  -> plugin resumes the MCP operation
```

基座负责显示和传递结果，插件负责决定交互发生的时机和业务含义。原始秘密只在受控的凭据通道中流转。

插件没有自定义前端时仍可请求基座公共交互；整个运行环境没有交互入口时，返回交互不可用错误。等待用户期间仍处理取消，实例销毁后不接收迟到的交互结果。

## 5. 流式和订阅型能力

订阅型协议不应被伪装成一个无限阻塞的命令工具。插件可以使用资源、通知或游标式读取：

```text
mqtt.subscribe(...) -> subscription_id
mqtt.read_messages(subscription_id, cursor) -> messages
mqtt.unsubscribe(subscription_id)
```

订阅状态、消息缓存、游标、QoS 和重连行为由 MQTT 插件负责。Bridge 只转发 MCP 调用，不理解 Topic 或消息语义。

## 6. 安全与生命周期

Bridge 必须具备插件实例关联、调用超时、取消、断开错误和权限错误的公共处理。高风险工具的确认策略可以由插件声明风险信息，由基座负责统一呈现确认，但基座不需要解析工具的领域参数。

插件崩溃或连接断开时，Bridge 返回明确的插件不可用或状态未知错误，不能伪造协议操作已经成功。
