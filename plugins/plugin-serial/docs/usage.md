# Serial 使用说明

接口总表、参数和边界见 [接口设计](interfaces.md)。以下命令在仓库根目录执行。

## 构建与启动

按仓库 [README](../../../README.md) 安装依赖，再执行：

```bash
pnpm plugin:serial:build
pnpm tauri:dev
```

前一个命令构建串口后端和前端并生成 `plugins/plugin-serial/dist` 开发包；后一个命令启动 Tauri 桌面。修改插件后重新构建插件包，修改宿主或 Bridge 后重启桌面；MCP 客户端重新连接以刷新工具目录。

在桌面创建 Serial 实例，填写端口与串口参数。界面默认 `115200 8-N-1`、DTR/RTS 置高；创建后在面板点击 Connect。部分开发板会因 DTR 变化复位，可取消打开时置高 DTR。端口名使用 `COM6` 或 `/dev/ttyUSB0` 等本机实际设备名。实例切换不关闭连接。

## 人工操作

| 模式 / 控件 | 使用方式 |
|---|---|
| Shell 终端 | 直接输入按键，由设备负责回显与编辑；没有额外的命令列表编辑器 |
| UART 文本 | 在发送框输入；Enter 发送、Shift+Enter 换行，可选 none/lf/cr/crlf 行结束符 |
| UART HEX | 勾选按 HEX 输入，仅发送解码后的字节；禁用行结束符，不隐式补换行 |
| HEX 接收 | 切到 HEX 接收视图后读取原始字节；退出后停止该读取 |
| 定时重复发送 | UART 文本和 HEX 均可使用有界重复发送；先确认设备能接收这些数据 |
| Ports / DTR / RTS | 创建实例后枚举端口；连接后可修改控制线电平 |

终端快捷键与 SSH 共用：Ctrl+C 有选区时复制，无选区时发送中断；Ctrl+F 搜索、Enter/Shift+Enter 跳转、Esc 关闭搜索。详见 [终端快捷键](../../../docs/architecture/terminal-shortcuts.md)。切换视图不重建连接，也不发送命令。MCP 列表发送期间人工写入与信号修改会被拒绝。

## MCP 调用流程

1. 在桌面创建实例，连接 MCP 后调用 `coterm.list_instances` 获取真实名称；MCP 不提供创建或自动恢复实例的工具。
2. 按需调用 `coterm-plugin-serial__serial.probe` 检查配置和端口，再调用 `coterm-plugin-serial__serial.connect` 连接。
3. 调用文本或 HEX 命令工具；若返回 `pending_approval`，等待用户批准后查询原操作，不重提写入。
4. 查看命令捕获结果，或使用 `get_recent` / `read` / `read_hex` 读取输出。

下面是 JSON-RPC `tools/call` 的 params 对象。示例名称 `COM6@115200` 仅用于说明，必须替换成 `coterm.list_instances` 的实际返回值。

### 发送文本并捕获回显

```json
{
  "name": "coterm-plugin-serial__serial.command",
  "arguments": {
    "name": "COM6@115200",
    "commands": ["pwd"],
    "lineEnding": "cr",
    "waitForPrompt": "# ",
    "waitTimeoutMs": 5000
  }
}
```

`lineEnding` 默认 CR，每项末尾自动追加。提示符必须匹配设备真实输出；它是普通子串而非正则，不会验证命令退出码。仅传 `waitTimeoutMs` 可定时收集；两个等待字段均不传时只确认发送。

列表如 `["cd /tmp", "pwd", "ls -la"]` 只保证顺序发送，并不等待前一条命令执行结束后再发送下一条。交互式任务应逐步确认设备状态。完成结果在 `structuredContent`，捕获文本位于 `structuredContent.output.text`，同时出现在 `content[].text`。

### 发送控制字符与 HEX

| 工具 | arguments 示例 | 实际发送 |
|---|---|---|
| `coterm-plugin-serial__serial.command` | `{"name":"COM6@115200","commands":["\u0003"],"lineEnding":"none"}` | 仅 Ctrl+C，不额外提交 CR |
| `coterm-plugin-serial__serial.command` | `{"name":"COM6@115200","commands":["pwd\r","\u001b[A\r"],"lineEnding":"none","intervalMs":100}` | 原样发送含回车及方向键的片段 |
| `coterm-plugin-serial__serial.command_hex` | `{"name":"COM6@115200","commands":["00 80 FF","0d0a"],"intervalMs":100}` | 两次发送原始字节，不把 HEX 当文本 |

不要同时在文本中写回车又使用默认 CR，除非确实要发送两次回车。登录密码和秘密应通过人工交互输入，不放进 MCP 命令列表。

### 读取输出

| 工具 | arguments 示例 | 适用场景 |
|---|---|---|
| `coterm-plugin-serial__serial.get_recent` | `{"name":"COM6@115200","lines":50}` | 直接查看最近文本，无需保存游标；重复调用可能看到相同内容 |
| `coterm-plugin-serial__serial.read` | `{"name":"COM6@115200","cursor":0}` | 从当前缓冲开头分页读文本，之后传上次返回的 cursor |
| `coterm-plugin-serial__serial.read_hex` | `{"name":"COM6@115200","cursor":0}` | 原字节分页读取，返回小写 HEX 字符串 |

`more:true` 时继续读取下一页。`gap:true` 表示早期数据已被淘汰，不能当作完整日志；`truncated:true` 表示快照或捕获结果达到预算。连接 ID 变化后重新建立游标，不复用旧连接的 cursor。MCP 客户端通过读取工具查询，没有这些事件的订阅；前端通知推送丢失时从回放缓冲恢复。

### 查询审批结果与取消

| 基座工具 | arguments 示例 | 说明 |
|---|---|---|
| `coterm.get_operation` | `{"name":"COM6@115200","operationId":"<原操作ID>"}` | 待审批、等待到期后查询同一个操作；最终输出在操作记录的 output |
| `coterm.cancel_operation` | `{"name":"COM6@115200","operationId":"<原操作ID>"}` | 请求停止命令列表，按返回结果判断；已发出的字节不能撤回 |

`coterm.get_operation` 返回的 `structuredContent` 是操作记录。对于串口 MCP 操作，记录的 `output` 保留插件的 MCP 结果形状：业务结果路径为 `structuredContent.output.structuredContent`，捕获回显为 `structuredContent.output.structuredContent.output.text`。这与首次调用直接完成时的 `structuredContent.output.text` 路径不同。

这两个是基座工具，不带插件前缀。Agent 不能自行批准、开启 Bypass 或放行。取消不确定、客户端超时和连接断开都不能作为自动重发依据。

需要立即取得收据时，协议级 `_meta` 与 `arguments` 同级：

```json
{
  "name": "coterm-plugin-serial__serial.command",
  "arguments": {
    "name": "COM6@115200",
    "commands": ["pwd"],
    "waitTimeoutMs": 5000
  },
  "_meta": { "coterm/waitTimeoutMs": 0 }
}
```

`coterm/waitTimeoutMs` 控制本次工具调用等待收据的时间，不取消后台操作；`arguments.waitTimeoutMs` 控制插件捕获设备输出的时间。客户端不支持设置协议级 `_meta` 时直接省略，不将它塞入业务参数。

## 前端调用

插件前端在已绑定实例的 `PluginAPI` 上调用业务操作，不传 MCP 名称和实例路由参数。例如：

```ts
const receipt = await api.invoke<{ connected: boolean }>("serial.connect", {});
if (receipt.status === "completed" && receipt.output) {
  console.log(receipt.output.connected);
}
```

Shell 输入调用 `serial.write` 并显式使用 `lineEnding:"none"`；UART 发送框按所选模式调用 `serial.write` 或 `serial.write_hex`。通过 `api.subscribeEvents()` 接收事件并保留注销函数，组件卸载时注销。完整类型见 [PluginAPI](../../../packages/plugin-sdk/src/plugin-api.ts)，事件与返回结构见 [接口设计](interfaces.md)。

## 验证

### 无需硬件

```bash
cargo test -p coterm-plugin-serial
pnpm --filter coterm-plugin-serial-ui build
node --import ./scripts/test-ts-loader.mjs --test tests/serial-terminal-browser.test.mjs
```

Rust 普通契约与基座加载测试不打开或写入物理端口；内存写端验证列表顺序、取消、故障、计数和回显。浏览器验收需要 Chrome/Edge/Chromium，验证实际前端构建在宿主 API 替身下的 Shell 按键、UART 输入及队列隔离。

### 物理串口

硬件用例默认 ignored。确认目标设备可接收测试写入后，替换为实际端口再执行：

```powershell
$env:COTERM_SERIAL_LIVE = "COM6"
$env:COTERM_SERIAL_LIVE_BAUD = "115200"
cargo test -p coterm-plugin-serial --test live -- --ignored --nocapture
```

用例验证真实字节往返、收发计数与 DTR/RTS，不验证设备业务语义；缺少设备配置会失败。手工诊断可用 `cargo run -p coterm-plugin-serial --example live -- COM6 115200`。

### 虚拟串口配对

先创建一对互联且未被占用的虚拟串口，再显式选择两端；测试会向两端发送数据，默认 ignored：

```powershell
$env:COTERM_SERIAL_PAIR_A = "COM1"
$env:COTERM_SERIAL_PAIR_B = "COM2"
$env:COTERM_SERIAL_PAIR_BAUD = "115200"
cargo test -p coterm-plugin-serial --test virtual_pair -- --ignored --nocapture
```

该测试启动真实插件进程，经宿主审批、stdio 和系统驱动验证发送、反向二进制回传、取消、容量、断开不重放与重连。可交换 A/B 再测，不验证提示符识别或物理线路时序。

### 真实桌面界面

需要可枚举的串口，默认 `COM6`，可用 `COTERM_SERIAL_LIVE_PORT` 覆盖：

```bash
pnpm plugin:serial:build
pnpm --filter coterm-desktop dev
```

保持前端开发服务运行，在另一个终端执行：

```bash
node scripts/test-serial-ui.mjs
```

脚本连接开发板并按需登录（`COTERM_SERIAL_USER` / `COTERM_SERIAL_PASSWORD`，默认 root/root），验证 Shell/UART、UTF-8/HEX、计数、控制线、语言切换和断开状态。它会向物理设备写入字节。设备上的列表发送节奏仍需按实际接收能力验收。

已完成的 MCP 验收及待确认的客户端 schema 导出问题见 [MCP 进度](../../../docs/progress/mcp.md) 与 [使用反馈](../../../docs/feedback/serial_mcp_use_feedback.md)。
