# SSH 插件

实现位于 [plugin-ssh](../../plugins/plugin-ssh)。SSH 连接、指纹、认证、PTY、输出与界面均归插件，基座负责加载、实例路由和公共交互。

## 使用

按 [README](../../README.md) 安装依赖后，执行 `pnpm plugin:ssh:build` 和 `pnpm tauri:dev`。

1. 左侧选择 SSH，填写 Host、Port、Username，创建实例。
2. 在实例中 Connect，通过可信来源核对 SHA256 指纹，再输入密码。
3. 用户终端与 MCP 共用该 PTY；可同时创建多个实例，切换界面不关闭连接。

MCP 客户端先用 `coterm.list_instances` 定位实例，再 `ssh.probe` 获取并核对指纹、`ssh.connect` 提交连接。密码由宿主交互收集，不放入工具参数；审批/密码等待后返回原操作结果。外部工具名称带插件命名空间，见 [MCP](mcp-integration.md)。

## 操作

| 操作 | effect | 用途 |
|---|---|---|
| `ssh.status/probe` | read | 状态、指纹探测；探测不等于信任服务器 |
| `ssh.connect/disconnect` | commit | 认证连接或释放连接 |
| `ssh.read` | read | 按游标回放 base64 字节，报告 gap/closed/more |
| `ssh.input` | commit | 人工原始按键，账本隐藏输入参数 |
| `ssh.command` | commit | 共享 POSIX PTY 上执行命令并返回退出码 |
| `ssh.resize` | draft | 调整终端尺寸 |
| `ssh.plugin_info` | read | 插件级信息 |

MCP 显式注册 status、probe、connect、disconnect、command、read，并映射到 `mcp.ssh.*`；不开放原始按键和密码输入。基座可调用的插件级信息操作不代表自动注册了同名 MCP Tool。

## 命令与取消

命令使用随机标记确定输出边界，通过当前 POSIX Shell 的 `eval` 执行，因此 `cd`、`export` 会保留。远端必须处于可接收命令的位置；当前不识别未完成的人工命令或 Windows Shell。

- 每连接同时只跟踪一个命令，最长 120 秒；与客户端等待上限分别计时。
- 源命令最多 4096 字节，转义包装后还受 4000 字节 PTY 单行上限约束。
- 结果含 base64 输出、退出码和截断标志，最多捕获 8 KiB；非零退出的 MCP 结果 `isError=true`。
- 取消尽力发送 Ctrl+C，不能证明远端进程停止，结果为 unknown。
- 断线、结束标记缺失或跟踪到期也可能 unknown；不推测成功、不自动重发。超时后拒绝下一条受跟踪命令，直到原标记到达或重连。

## 输出与限制

每连接回放缓冲 256 KiB，每块/单次读取 8 KiB；事件为 `ssh.output` 和 `ssh.status`，携带连接 ID 和游标。前端先订阅再回放，缺口时恢复；输入队列上限 16 KiB，控制队列 64 项，终端滚动缓存 3000 行。

指纹每次连接确认，不持久化。尚无私钥、ssh-agent、MFA、SFTP、端口转发、自动重连和凭据保存；命令块选择与上下文链路也未接通。秘密和背压规则见 [运行时通信](runtime-communication.md)。

## 验证

`cargo test -p coterm-plugin-ssh` 使用本地测试 SSH 服务，覆盖指纹/密码失败、多实例、真实插件进程、审批、秘密隔离、共享 PTY、输出缺口和 MCP。远程服务器及真实桌面验收需单独执行，最近验证范围见 [项目进度](../PROGRESS.md)。
