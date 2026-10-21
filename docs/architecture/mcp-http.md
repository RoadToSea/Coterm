# 固定 MCP 连接

Coterm 桌面启动时提供 MCP Streamable HTTP 入口：

```text
http://127.0.0.1:23517/mcp
```

在支持 Streamable HTTP 的 Agent 客户端中添加此 URL 即可，无需 Token、认证请求头、环境变量或转接程序。顶部 **MCP** 面板提供地址和复制按钮。保持 Coterm 打开；应用重启后 URL 不变，客户端重新初始化会话并发现实例。

开发时执行 `pnpm plugin:ssh:build` 和 `pnpm tauri:dev`，无需另外构建或启动 MCP 转接程序。基座仅提供一个入口，复用桌面中已经创建的插件实例。工具和等待语义见 [MCP Tools 与秘密输入](mcp-stdio.md#可用工具)。

## 传输与会话

- `POST /mcp` 接收单条 JSON-RPC 消息；请求返回 `application/json`，通知和客户端响应返回空的 `202`。不支持批量请求。POST 的 Accept 须包含 `application/json` 和 `text/event-stream`。
- `initialize` 不携带旧会话 ID；成功应答通过 `Mcp-Session-Id` 返回新会话，客户端自动在后续请求携带它。支持协议版本 `2025-06-18`、`2025-03-26`，其他版本协商为 `2025-06-18`。
- 后续请求校验 `MCP-Protocol-Version` 与会话协商结果；省略时按 MCP 规范视为 `2025-03-26`。客户端发送 `notifications/initialized` 后才能调用工具。
- `GET /mcp` 提供可选 SSE 流，发送 `notifications/tools/list_changed`；每秒检查一次目录，15 秒发送一次保活注释。每会话最多一个 SSE 流；重新订阅先发送目录变化通知，客户端重新获取完整目录。没有事件 ID、回放缓冲或断点续传；插件终端输出仍走桌面内部事件接口。
- `DELETE /mcp` 结束该客户端会话，不销毁插件实例、不撤销待审批操作，也不取消或重放已提交命令。旧会话、空闲过期会话及应用重启前的会话返回 `404`，客户端应重新初始化。

最多 8 个会话，空闲 30 分钟过期；活动 SSE 流会保持会话。每会话最多 16 个在途请求，HTTP 请求处理全局最多 128 个；请求和 JSON 结果各不超过 1 MiB，读取请求体最长 10 秒。达到上限立即报错，不无限排队。SSE 随消费者按需拉取目录通知，不缓存终端数据。应用关闭时停止监听、关闭 SSE 并结束等待中的 HTTP 请求；基座操作不会因客户端断开而自动重放。

工具调用复用 `server.rs` 中的工具发现与 Agent 路由，原请求默认等待审批、秘密交互及执行结果。基座在路由前按插件握手注册的 JSON Schema 校验参数；审批修改后的快照也会再次校验，校验失败只返回错误并保留待审批操作。传输层不理解 SSH、MQTT 等协议，也不改变操作记录、审批快照或 Bypass 规则。仅客户端明确要求立即返回或等待到期时，才需按操作 ID 查询结果。

## 本机访问边界

只监听 IPv4 `127.0.0.1`，无客户端 Token，不区分本机进程或系统用户身份。要求 Host 与监听地址一致；请求带 Origin 时仅允许该服务自身的 HTTP Origin，拒绝其他来源，不开放通配 CORS。该模式不用于局域网、远程访问或多用户隔离。

所有外部调用仍固定为 Agent 来源；Agent 不能批准自己的写操作、伪造 Human 来源或开启 Bypass。用户主动启用 Bypass 后沿用其选择。无 Token 不影响插件自己的主机校验、密码认证和秘密输入。

固定端口被占用时，桌面仍可正常使用，MCP 面板显示明确错误；不自动换端口、不终止其他进程。关闭占用端口的应用后重启 Coterm。当前不提供自定义端口、多进程目标选择或自动启动桌面。

## 实现与验证

`crates/coterm-mcp/src/http.rs` 使用 Axum/Tokio 的跨平台 TCP/HTTP 实现，不新增 Tauri capability、不启动外部进程。旧 stdio 适配器保留用于开发与回归测试，桌面不再发布它的随机地址或 Token。

自动化覆盖仅 URL 初始化、工具发现、会话关闭、同地址重启、端口冲突、Host/Origin 校验、版本校验、消息上限、会话与 SSE 上限，以及活动流在关闭时退出。真实 SSH 插件子进程测试覆盖 HTTP 请求跨人工审批等待、并发状态查询、修改后输出和非零退出码，以及结束 HTTP 会话不撤销或重放待审批操作。已通过 `cargo check --workspace`、28 项工作区测试、`cargo clippy --workspace -- -D warnings` 和 `pnpm build`。另使用官方 Python MCP SDK 连接本机测试服务，验证仅 URL 初始化、工具发现、实例查询及正常会话关闭；该客户端测试关闭环境代理以直连本机。其他操作系统、真实桌面窗口及远程 SSH 的 HTTP 实机验收应单独记录。
