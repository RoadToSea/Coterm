# Serial 使用说明

接口总表、参数和边界见 [接口设计](interfaces.md)。以下命令在仓库根目录执行。

## 构建与启动

按仓库 [README](../../../README.md) 安装依赖，再执行：

```bash
pnpm plugin:serial:build
pnpm tauri:dev
```

前一个命令构建串口后端和前端并生成 `plugins/plugin-serial/dist` 开发包；后一个命令启动 Tauri 桌面。修改插件后重新构建插件包，修改宿主或 Bridge 后重启桌面；MCP 客户端重新连接以刷新工具目录。

在桌面创建 Serial 实例，先从自动查询的本机串口列表选择端口，再填写串口参数；未启动的插件由宿主在查询前启动。每次点击或键盘展开端口下拉框都会重新查询，插拔设备后直接再次点击即可，保留仍存在的选项，不再单独显示刷新按钮。查询失败显示错误，再次点击下拉框可重试，只有成功返回空列表才显示「未检测到串口」。界面默认 `115200 8-N-1`、DTR/RTS 置高；创建后在面板点击 Connect。部分开发板会因 DTR 变化复位，可取消打开时置高 DTR。端口名使用 `COM6` 或 `/dev/ttyUSB0` 等本机实际设备名。实例切换不关闭连接。

## 人工操作

| 模式 / 控件 | 使用方式 |
|---|---|
| Shell 终端 | 直接输入按键，由设备负责回显与编辑；没有额外的命令列表编辑器 |
| 退格键 | 普通 Backspace 默认发送 BS（`0x08`）；设备要求 DEL 时，在配置中将「退格键」改为 DEL（`0x7F`）。不附加换行，由设备负责删除和回显；UART 发送框仍是本地编辑 |
| UART 文本 | 在发送框输入；Enter 发送、Shift+Enter 换行，可选 none/lf/cr/crlf 行结束符 |
| UART HEX | 在底部发送格式中选 HEX，仅发送解码后的字节；禁用行结束符，不隐式补换行 |
| 配置 | 右上角展开，面板从顶栏下方向下覆盖接收区；再次点击或按 Esc 收起，不挤压发送栏 |
| 连接按钮 | 同时显示连接状态：连接绿色、未连接灰色，点击连接 / 断开；不再额外显示端口及状态文字 |
| 通信参数 | 先断开，在平铺配置中编辑波特率、数据位、停止位、校验和打开时的 DTR / RTS；下次连接前自动校验并应用，没有应用按钮或端口列表；当前进程内生效 |
| HEX 接收 | 在快捷栏用 ASCII / HEX 分段滑块选择；HEX 视图读取原始字节，退出后停止该读取 |
| 接收换行补全 | 配置中勾选「LF 补 CR」可将设备输出的单独 `\n` 显示为 CRLF，勾选「CR 补 LF」可将单独 `\r` 显示为 CRLF；可以同时勾选，默认均不选，已有 CRLF 不重复补全 |
| 文件导入 | 输入框旁文件夹按钮选择文件，ASCII 按 UTF-8 读入，HEX 将原字节转成 HEX；只填入草稿，Enter 才发送 |
| 发送历史 | 点击文件夹下方的回转时钟图标，选中历史后恢复输入、格式和换行符；仅当前挂载内保留，最多 50 项 / 64 KiB |
| 定时重复发送 | 默认 100 ms，可设 20–60000 ms；一次完成后再等待间隔，避免积压；改输入、切模式、断线或卸载停止 |
| 接收保存 | 配置中填写下载文件名，快捷栏保存图标导出当前有界接收缓冲快照；保存位置由系统下载设置决定，不是持续落盘日志 |
| 暂停 / 清空 | 快捷栏使用带名称的图标；暂停显示不停止设备接收，继续时提示期间收到的字节数 |
| 快捷栏设置 | 点击配置右侧的调整按钮，控件围绕中心轻微快速晃动；按住拖动排序，拖动时下方出现垃圾桶，拖入即移除。无需复选框列表；可单项加回、恢复默认，点击完成或按 Esc 退出。也可聚焦控件后按 Alt + 左右方向键调序、Delete / Backspace 移除；减少动画设置下不晃动，只保存 UI 布局偏好 |
| DTR / RTS | 配置复选框：断开时表示下次打开电平，连接时实时调整。它们不等同 RTS/CTS 自动硬件流控 |
| 消息提示 | 公共非模态圆角提示：错误红色、警告黄色，默认 4 秒自动消失；悬停 / 聚焦暂停计时，可手动关闭，不抢焦点 |

发送区不保留单次发送按钮。Enter 单次发送，Shift+Enter 在发送框中插入换行；中文输入法组合和按住 Enter 不会重复提交。ASCII / HEX 分段滑块可点击、左右键或滑动选择；选 HEX 将 UTF-8 草稿转换为 `字节 字节`，选 ASCII 只接受有效 UTF-8，非法内容保留原稿并提示。换行符在配置中设置，单次发送上限 16 KiB，包含附加的换行符。导入文件超过上限时拒绝，不会截断后自动发送。定时控件显示间隔、ms 和定时发送按钮，运行时绿色；写入失败会停止，不自动重试。

终端快捷键与 SSH 共用：Ctrl+C 有选区时复制，无选区时发送中断；Ctrl+F 搜索、Enter/Shift+Enter 跳转、Esc 关闭搜索。详见 [终端快捷键](../../../docs/architecture/terminal-shortcuts.md)。切换视图不重建连接，也不发送命令。MCP 列表发送期间人工写入与信号修改会被拒绝。

接收换行补全用于嵌入式设备只输出 LF 导致终端后续行不回到行首，或只输出 CR 导致内容覆盖的问题。两个复选框可在连接中调整；都不勾选即保留原样。终端从后续输出开始生效，UART 文本会重新渲染当前缓存；原始数据、HEX、保存文件、计数和 MCP 返回均不改变，也不会向设备发送附加字节。此配置与发送用的「行结束符」相互独立。

终端 / UART 视图、接收 ASCII / HEX、时间戳、自动滚动、换行补全和退格编码会自动保存到当前实例的固定保存项，重启后恢复原保存项即可还原。不同保存项即使用同一个 COM 口也互不覆盖；名称、COM 分配与运行实例 ID 不作为保存主键。删除实例后重新创建属于新的保存项。不会恢复发送草稿、发送历史或定时发送任务。需要同时更新桌面和串口前端；旧桌面未提供固定标识时只能在当前运行实例内保留。以前已重启丢失归属的换行偏好需重新勾选一次，不能仅凭 COM 或名称猜测恢复。

## MCP 调用流程

先在创建页或断开后的配置中选择实际的数据模式。默认 `stream` 只读取逻辑行，不生成对话；设备确实运行 Shell 时选择 `shell`，填写设备的完整可见提示符（例如独占一行的 `# `，或 `root@board:~# `）。终端 / UART 视图切换不改变此配置。登录和密码由人工终端完成，Agent 通过 `serial.status` 的 `shellReady` 确认已经观察到 Shell 提示符，再发命令。

1. 在桌面创建实例，连接 MCP 后调用 `coterm.list_instances` 获取真实名称；MCP 不提供创建或自动恢复实例的工具。
2. 按需调用 `coterm_serial_probe` 检查配置和端口，再调用 `coterm_serial_connect` 连接。
3. 调用文本或 HEX 命令工具；若返回 `pending_approval`，等待用户批准后查询原操作，不重提写入。
4. 用 `read_lines` 读取后端尾部逻辑行；Shell 还可用 `read_selected_lines` 读取人工选区、`read_dialogue_pairs(2)` 读取最近两次对话。

也可先调用无参数 `coterm.status` 查看连接状态。恰有一个确认已连接的串口实例且无未知状态时，后续实例工具可省略 `name`；连接前或多连接时仍使用真实名称。审批后查询建议保留原实例名称，避免连接状态变化影响默认路由。

下面是 JSON-RPC `tools/call` 的 params 对象。示例名称 `COM6@115200` 仅用于说明，必须替换成 `coterm.list_instances` 的实际返回值。

### 发送文本并捕获回显

```json
{
  "name": "coterm_serial_command",
  "arguments": {
    "name": "COM6@115200",
    "commands": ["pwd"],
    "lineEnding": "cr",
    "waitForPrompt": "# ",
    "waitTimeoutMs": 5000
  }
}
```

此例要求实例为 Shell 模式，且实际提示符的完整可见行是 `# `。`lineEnding` 默认 CR，每项末尾自动追加。提示符按完整可见行匹配，忽略尾部空白，支持分包和 ANSI 颜色；它不是正则，也不能证明退出码。省略 `waitForPrompt` 时使用实例配置；Shell 默认每条等待 5 秒，stream 仅传 `waitTimeoutMs` 表示定时收集。

Shell 列表逐条发送，每条观察到提示符后才发下一条；当前条超时、取消或断线会停止余项，不自动重放。Shell 结果在 `structuredContent`，始终保留 `stdin/stdout/stderr`；串口输出合并在 `stdout`，`stderr` 是空串，`exitCode` 是 null。运行中输出及最近对话可用读取工具查询。stream 列表仅保证发送顺序，其定时捕获文本位于 `structuredContent.output.text`，不形成对话。

### 发送控制字符与 HEX

下表的原样文本列表用于 stream 模式。人工终端输入在两种模式下均保留控制字符语义；Shell 的命令列表需要已就绪提示符，不能冒充登录或任意交互输入。

| 工具 | arguments 示例 | 实际发送 |
|---|---|---|
| `coterm_serial_command` | `{"name":"COM6@115200","commands":["\u0003"],"lineEnding":"none"}` | 仅 Ctrl+C，不额外提交 CR |
| `coterm_serial_command` | `{"name":"COM6@115200","commands":["pwd\r","\u001b[A\r"],"lineEnding":"none","intervalMs":100}` | 原样发送含回车及方向键的片段 |
| `coterm_serial_command_hex` | `{"name":"COM6@115200","commands":["00 80 FF","0d0a"],"intervalMs":100}` | 两次发送原始字节，不把 HEX 当文本 |

不要同时在文本中写回车又使用默认 CR，除非确实要发送两次回车。登录密码和秘密应通过人工交互输入，不放进 MCP 命令列表。

### 读取输出

| 工具 | arguments 示例 | 适用场景 |
|---|---|---|
| `coterm_serial_read_lines` | `{"name":"COM6@115200","lines":50}` | Shell / stream 均支持，读取后端最近逻辑行，无需游标 |
| `coterm_serial_read_selected_lines` | `{"name":"COM6@115200"}` | 仅 Shell；读取人工在 xterm 中选中的文本，未选中则为空 |
| `coterm_serial_read_dialogue_pairs` | `{"name":"COM6@115200","count":2}` | 仅 Shell；最近两次输入及输出，最新在前，检查状态和截断标记 |

这些读取不暴露游标，不需要 Agent 维护状态。`dropped` 表示旧数据曾被淘汰，`truncated:true` 表示结果达到预算；对话可能仍在运行，也可能是超时、取消或断线后的未知结果。`capabilities` 指示目标实例支持哪些读取；共享工具目录会列出 Shell 专用工具，但 stream 目标会明确拒绝它们。前端通知推送丢失时仍使用内部回放接口恢复，这不是 MCP 的读取契约。

### 查询审批结果与取消

| 基座工具 | arguments 示例 | 说明 |
|---|---|---|
| `coterm.get_operation` | `{"name":"COM6@115200","operationId":"<原操作ID>"}` | 待审批、等待到期后查询同一个操作；最终输出在操作记录的 output |
| `coterm.cancel_operation` | `{"name":"COM6@115200","operationId":"<原操作ID>"}` | 请求停止命令列表，按返回结果判断；已发出的字节不能撤回 |

`coterm.get_operation` 返回的 `structuredContent` 是操作记录。对于串口 MCP 操作，记录的 `output` 保留驱动的 MCP 结果形状：业务结果路径为 `structuredContent.output.structuredContent`。Shell 输出读取该对象的 `stdout`；stream 定时捕获读取该对象的 `output.text`。首次调用直接完成时，业务结果直接位于 `structuredContent`。

这两个是基座工具，不带插件前缀。Agent 不能自行批准、开启 Bypass 或放行。取消不确定、客户端超时和连接断开都不能作为自动重发依据。

需要立即取得收据时，协议级 `_meta` 与 `arguments` 同级：

```json
{
  "name": "coterm_serial_command",
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

Shell 专项验收要求人工先完成登录、释放串口占用，并提供精确提示符；仅发送回车、两条 `printf` 及 `uname -s`。打开时 DTR / RTS 配置为低电平，不执行额外的控制线切换，也不自动输入账号密码：

```powershell
$env:COTERM_SERIAL_SHELL_LIVE = "<实际端口>"
$env:COTERM_SERIAL_SHELL_PROMPT = "<完整可见提示符>"
$env:COTERM_SERIAL_LIVE_BAUD = "115200"
cargo test --locked -p coterm-plugin-serial --test shell_live -- --ignored --nocapture
```

验证两个对话互不串输出、stderr 合并、退出码未知、无游标行读取、断开后的后端快照，以及 stream 明确拒绝对话读取。默认忽略，避免普通回归向硬件发送数据。

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
