# 终端聚焦与内置驱动架构决策

## 定位与取舍

Coterm 面向人类与 Agent 协同，支持 SSH、串口、本地终端、Telnet 与 Raw TCP。非标字节流解码、连续波形分析和信号数学计算由专用工具承担，不在基座扩展统一设备业务模型。

五个第一方内置驱动随桌面应用分发。普通用户直接创建实例，无需安装离线驱动包；构建与发布约束见 [内置驱动开发](../architecture/driver-development.md) 和 [发行说明](../release/alpha-release.md)。

驱动以 Rust 库链接进桌面主进程，每种驱动由一个独立异步任务串行处理控制请求，同一驱动管理多个状态隔离的实例。任务退出影响该驱动承载的全部实例；普通 panic 可按任务收敛，原生崩溃、abort 与阻塞底层调用不具备进程级隔离。

## 后端组织

core、共享协议类型与终端工具收拢到 `coterm-core`；具体驱动收拢到单个 `coterm-driver`，内部使用 `ssh/serial/local/tcp/telnet` 模块；MCP 保持独立。桌面注册驱动，driver 与 MCP 只依赖 core。固定声明在注册时校验，控制请求直接传 Rust 枚举，诊断 JSON-RPC 不进入产品调用链。

串口与网络复用 `stream` 的有界接收和提示符观察；Raw TCP 与 Telnet 复用 `network` 的连接生命周期，Telnet 协商独立处理。前端共享 xterm 生命周期，串口与 Raw TCP 共享终端／字节工作台；协议参数、控制线、SSH exec 等能力留在各驱动。具体接口见 [网络驱动](../architecture/network-drivers.md) 和 [共享终端](../../apps/shared/docs/terminal-surfaces.md)。

## 主要数据链路

```text
人工界面 / MCP
      ↓
基座：来源识别 → 实例路由 → 每实例审批 → 操作记录
      ↓ 类型化请求 / 有界队列
SSH / Serial / Local / Telnet / TCP 后端任务 → 远端、设备或本地 Shell
      ↓
后端有界记录 → MCP 快照读取
      ↓ 事件与内部回放
前端显示副本 → xterm / UART / 命令块
```

- 基座负责实例、路由、审批、公共交互、操作记录与 MCP Bridge；驱动负责通信、命令边界和领域数据。前端只呈现数据，不是 Agent 查询的唯一数据源。
- 每个运行实例创建独立审批对象，外部传入已验证的 context。规则组独立保存，每个实例绑定一个组；组保存后对所有绑定实例的新提交生效；已保存实例同时记住 Bypass，恢复时加载；临时实例的 Bypass 仅内存有效。见 [命令审批](../architecture/command-approval.md)。
- SSH、Local 使用统一 `command`，默认在前台 PTY 执行；预计长期运行或用户明确要求时显式选择 `mode=exec`。等待超时不迁移执行通道、不自动重发。
- 后端 JSON 记录保留 `stdin/stdout/stderr`，空流也保留键。能分流时分别保存，PTY 已合并时在 stdout 保存合并输出，不从文本猜测 stderr；未知退出码保留 null。
- Agent 使用 `read_lines`、`read_selected_lines`、`read_dialogue_pairs` 非消费快照，不维护游标。Serial／Raw TCP 的 stream 模式只提供 `read_lines`；Shell 模式通过配置的完整字面提示符观察对话边界，不把提示符当成功证明。Telnet 无 SSH exec 或独立 stderr。
- 接收与读取不经过写操作审批；缓存、事件投递和慢消费者仍需同步、有界及缺口处理。前端内部回放可以使用游标。

具体字段、容量、错误和取消语义分别见 [SSH](../architecture/ssh-driver.md)、[Serial](../architecture/serial-driver.md)、[Local](../architecture/local-driver.md)、[运行时通信](../architecture/runtime-communication.md) 和 [MCP](../architecture/mcp-integration.md)。

## 界面与实施入口

实例分组与标签生命周期见 [桌面界面](../architecture/desktop-appearance.md)，共享终端交互见 [终端快捷键](../architecture/terminal-shortcuts.md)，组件接口见 [共享前端模块](../../apps/shared/docs/README.md)。当前能力、验证范围和未完成项统一记录在 [项目进度](../progress/project.md)。
