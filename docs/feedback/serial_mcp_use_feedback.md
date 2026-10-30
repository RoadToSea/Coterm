# Coterm 串口插件（coterm-plugin-serial）MCP 使用体验与反馈报告

在本次通过 **Antigravity CLI** 配合 **Coterm Desktop + coterm-plugin-serial 串口插件**（基于 MCP Stdio 桥接）完成 **COM1 与 COM2 互联闭环测试** 的过程中，我作为 AI Agent 完整体验了串口发现、状态探测、双向数据读写与游标追踪的全部链路。

Coterm 的底层架构设计非常扎实（进程隔离、基于 `cursor` 的有界回放缓冲、严格的生命周期状态机与精准的字节收发统计），但在 **AI Agent 实际消费 MCP 工具** 的场景中，仍存在一些直接影响模型调用成功率与交互流畅度的体验摩擦点。以下是我作为 Agent 视角的具体反馈与改进建议。

---

## 一、架构与功能亮点（做得很好的地方）

1. **精准的收发字节审计与对称性**：
   在完成双向读写后，COM1 的 `txBytes`（28）与 COM2 的 `rxBytes`（28）、COM2 的 `txBytes`（42）与 COM1 的 `rxBytes`（42）呈现出**绝对对称的 byte accounting**，这让 Agent 在无需依赖设备 Shell 回显的情况下即可确信数据已物理送达。
2. **游标回放机制（Cursor Replay）非常健壮**：
   `serial.read` 采用数字序列游标（`cursor`），即便在分段接收或存在历史旧数据（例如之前手动输入的无换行字节）时，Agent 只要指定游标位移就能精确取得增量数据，并获知 `gap`（是否出现缓冲区溢出导致的丢包）。
3. **支持多端口共存与清晰的实例生命周期**：
   同时管理 COM1 和 COM2 两个并发实例，互不干扰，实例的 `ready`、`connected` 状态清晰明确。

---

## 二、使用过程中遇到的不合理与摩擦点

### 1. 致命缺陷：`instructions.md` 指引与后端参数校验直接冲突

* **实际遇到的问题**：
  Coterm 在 MCP 导出的 `instructions.md` 中给 Agent 提供了如下明确指引：
  > *"Tools wait up to 60s including approval and human interaction. Set params._meta['coterm/waitTimeoutMs'] to 0 for an immediate receipt, or 1..300000 to bound waiting."*

  因此 Agent 按照指引，在调用 `serial.command` 时将 `_meta` 作为参数传入：
  ```json
  {
    "pluginInstanceId": "19089ac4-935e-436f-bfb2-ca1ddd3e3d04",
    "commands": ["echo test_com1_to_com2"],
    "lineEnding": "crlf",
    "_meta": {
      "coterm/waitTimeoutMs": 3000
    }
  }
  ```
  结果直接被插件后端拦截报错：
  `operation_args_invalid: Operation arguments do not match the registered schema`
* **根因分析**：
  * **协议层与工具层混淆**：在标准 MCP 协议中，`_meta` 是 JSON-RPC 顶层 `CallToolRequestParams` 的字段；但绝大多数 AI 编程助手（Antigravity、Claude Code、Cursor 等）只向模型暴露 `arguments` 字典，Agent 无法在模型层注入底层协议头。
  * **后端校验过度严格**：`command.rs` 的参数结构体标注了 `#[serde(deny_unknown_fields)]`，且 `backend.rs` 中的 `validate_mcp_args` 设置了 `additionalProperties: false`。
  * **结果**：文档教 Agent 传 `_meta`，而后端直接判为非法参数打死，导致第一次工具调用 100% 报错。

---

### 2. 工具声明中参数 Schema 缺失（`parameters: null`）

* **实际遇到的问题**：
  Antigravity 本地发现 Coterm 导出的 MCP 工具描述文件（如 `coterm-plugin-serial__serial.command.json`）时，内容如下：
  ```json
  {
    "name": "coterm-plugin-serial__serial.command",
    "description": "Queue 1-64 single-line commands...",
    "parameters": null
  }
  ```
* **痛点**：
  * 因为 `parameters` 声明为 `null`，Agent 在拿到工具时完全不知道该传什么参数（`commands` 是数组还是字符串？换行符支持哪些枚举？`cursor` 是整型还是字符串？是否必须传 `pluginInstanceId`？）。
  * Agent 必须借助子代理去翻阅 `Coterm` 项目的 Rust 源码（`backend.rs`、`command.rs`）才能摸清真实的参数签名，这在没有本地源码的真实用户场景下将直接导致调用失败或频繁试错。

---

### 3. 强绑定 UUID 寻址，缺乏人类/Agent 友好的端口别名

* **实际遇到的问题**：
  每个串口工具（`command`、`read`、`probe`、`status`）都必须携带完整的 UUID，例如：
  `pluginInstanceId: "19089ac4-935e-436f-bfb2-ca1ddd3e3d04"`。
* **痛点**：
  * 当用户明确指令“使用 COM1 发送，在 COM2 接收”时，Agent 无法直观地调用 `serial.command(port="COM1", ...)`。
  * 必须强行多走一步：先调用 `coterm.list_instances`，解析 JSON 数组，匹配 `label: "COM1@115200"`，把 UUID 提取出来存入上下文，再在后续所有调用中拼装长字符串 UUID。不仅浪费轮次与 Token，还增加了长哈希字符串复制错误的风险。

---

### 4. 数据输入与输出格式的不对称（纯文本 vs Base64）

* **实际遇到的问题**：
  * **发送端 (`serial.command`)**：提供了非常高级的文本抽象，接收字符串数组 `commands: ["echo foo"]`，且能自动处理 `lineEnding: "crlf"`；
  * **接收端 (`serial.read`)**：却只返回底层的 Base64 字节串（例如 `data: "aGVsbG9oZWxsbyx0aGlzIGNvbTINCg=="`）。
* **痛点**：
  * 串口在 AI 辅助场景下，90% 以上是与 Shell、REPL、固件 BootLoader、AT 指令交互，本质都是文本流。
  * 强制返回 Base64 要求 Agent 在每次读取后，都必须在本地通过命令工具或 Python 执行 Base64 解码，不仅增加了步骤，还大幅拖慢交互节奏。

---

### 5. 敏感操作审批机制导致自动化测试挂起 50~60 秒

* **实际遇到的问题**：
  `serial.command` 属于写操作（`sensitive_input: true` / `effect: Commit`）。在 MCP 工具发起调用后，由于 Coterm 桌面端弹出了审批确认提示，工具调用在后台同步阻塞了整整 **48 秒**（直到人类在 UI 上点击允许或超时）。
* **痛点**：
  * 人机协同场景下审批很有必要，但在 **自动化集成测试 / Agent 联调场景** 下，缺乏会话级别的免审批授权通道（如“允许本次会话自动放行”或“测试模式”）。
  * 一旦用户离开电脑，Agent 在执行第 1 条测试命令时就会被硬生生卡住 60 秒然后超时中断。

---

## 三、针对 Coterm MCP 的改进建议与理想形态

### 1. 修复参数校验与元数据容差
* **过滤/容忍 `_meta`**：在 `backend.rs` 的 `validate_mcp_args` 中，显式将 `_meta` 忽略或在序列化前剔除；或者允许在 `CommandArgs` 中透传 `waitTimeoutMs`。
* **完善 MCP Tool 的 JSON Schema**：确保 `mcp_tools()` 注册向 MCP 客户端广播时，`inputSchema` 完整携带参数类型与必填项描述，避免宿主收到 `parameters: null`。

### 2. 支持 `port` / `label` 作为实例别名寻址
```json
// 允许 Agent 这样直接调用：
{
  "port": "COM1",
  "commands": ["help"],
  "lineEnding": "crlf"
}
```
* **实现逻辑**：后端优先通过 `pluginInstanceId` 查找；如果未传或未找到，则退化匹配 `instance.config.port == args["port"]` 或 `instance.label == args["label"]`。

### 3. `serial.read` 增加文本直出字段
```json
// 理想的 read 返回载荷：
{
  "cursor": "7",
  "text": "REPLY_FROM_COM2_OK\r\n",
  "data": "UkVQTFlfRlJPTV9DT00yX09LDQo=",
  "gap": false,
  "closed": false
}
```
* 保持 `data` 供纯二进制场景，但对有效 UTF-8 内容增加 `text` 字段，省去大模型频繁额外调用 Base64 解码器的开销。

### 4. 增加“自动化调试授权 / 免审批”机制
* 在 Coterm 桌面端的实例配置中增加开关：`允许 Agent 免审批写入`（或限制仅允许特定安全前缀指令）。
* 这样在硬件回路自测、自动化回归测试时，Agent 可以毫秒级连续完成发包与响应判定，无需人工在旁边逐次点击批准。

---

*报告生成时间：2026-10-30*  
*测试环境：Windows 11 / Antigravity CLI / Coterm Desktop (0.1.0) / 虚拟串口对 COM1 <-> COM2 (115200 8N1)*
