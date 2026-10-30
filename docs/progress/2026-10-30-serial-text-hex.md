# 串口文本与 HEX 接口拆分

## 契约

- 人工 Shell/UART 文本：`serial.write({text,lineEnding?})`；默认不附加行结束符。
- Agent 文本列表：`serial.command({commands,lineEnding?,intervalMs?})`；与人工文本共用编码函数，保留所有控制字符及多行输入。
- 人工 HEX：`serial.write_hex({hex})`；Agent HEX 列表：`serial.command_hex({commands,intervalMs?})`，项为 HEX 字符串，绝不隐式追加行结束符。
- 文本读取 `serial.read` 返回 `text`；HEX 读取 `serial.read_hex` 返回 `hex`。两者共享块游标并报告实际字节数。
- 前端 Shell 和文本视图不调用二进制接口；仅 HEX 发送选项和 HEX 接收视图调用相应 HEX 接口。
- 串口插件移除 Base64 依赖及收发编码；Base64 是编码而非加密。基座认证和审批不变。
- Modbus、固件烧录等协议归独立插件，不在通用串口插件增加帧协议、CRC 或下载握手。

## 验证

普通测试覆盖控制字符、分段 UTF-8、无效 HEX、全部 256 种字节、参数混用拒绝、有界读取、审批及公共 MCP 名称路由；浏览器测试验证 Shell/UART 文本和 HEX 接口分离、复制粘贴、控制键与搜索。

本轮通过：`cargo test -p coterm-plugin-serial -p coterm-mcp`（46 项非硬件测试）、`cargo check --workspace`、串口全部 target 的 Clippy、`pnpm build`、`pnpm plugin:serial:build`，以及串口交互、串口/SSH 快捷键和桌面主题切换的浏览器测试。串口开发包已同步生成在 `plugins/plugin-serial/dist`。

本轮尝试 COM1/COM2 虚拟串口回环时，打开端口失败，系统返回“找不到指定的文件”。因此物理字节回环尚未验收；虚拟串口测试已更新，配置端口后可显式运行。

接口变化需同时重建串口插件后端和前端，旧版 `data`/Base64 请求不再兼容；MCP 客户端需重新发现工具。
