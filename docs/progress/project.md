# 项目进度

## 当前能力

- 三个内置驱动随桌面资源分发，保留独立 Worker 和多实例隔离，普通用户安装管理已移除。
- 每实例审批对象和独立保存规则；Bypass 仅会话有效。
- SSH、Local command 默认前台，显式 exec 后台；后端保留 stdin/stdout/stderr，PTY 合并输出。
- Serial 按 stream/Shell 分离读取能力，Shell 支持字面提示符配置与识别，配置修改先保存再连接。
- Agent 三类读取不使用游标；原始 stream 仅支持 read_lines。

## 2026-11-08：数据链路与内置驱动收敛

- 串口配置保存、Local 前台命令和后端记录、内置资源构建已实施。Local PowerShell/cmd 和 COM6 开发板已完成原生 WebView2 → MCP Stdio → 审批界面 → Worker → 设备链路验收，配置恢复不自动连接。
- 命名统一为 drivers/driver-*、@coterm/driver-sdk、Driver* 和 driverId/driverInstanceId。控制协议与 SDK 升为 v2.0；工作区写 v3，兼容 v1/v2 旧标识，保留配置引用、分组和独立审批策略。
- 重命名后的独立提交候选：Rust 254 项通过、3 项硬件用例默认忽略；前端与浏览器 90 项通过，Clippy 无警告。桌面与三个驱动构建、原生 MCP/审批/COM6 链路通过。文档链接和 diff 空白检查通过。

## 2026-11-08：过期文件与文档收敛

- 删除已落地或被替代的设置、标签、工作区、串口界面、Local、MCP 与 SSH 方案；有效约束归入架构文档，终端架构保留定位、取舍和主要数据链路。
- 删除重复进度与已解决反馈，串口硬件和早期 SSH 现场验收摘要归入对应模块；保留 SSH 空输出和串口客户端 Schema 导出两个待复测问题。
- 清理重命名后旧目录与无引用反馈截图，修复文档入口及相对链接。本次仅清理文件与文档，不修改运行时行为；验证范围为本地文档链接、目录和差异空白检查。

## 限制与续作入口

- 非 Windows 的实机与发行安装验收尚未完成；NSIS 资源与干净环境安装需发行前验证。
- Local 原始人工按键不生成对话块；后台取消不能保证回收完整进程树。
- 各模块入口：[Local](local-driver.md)、[SSH](ssh-driver.md)、[Serial](../../drivers/driver-serial/docs/progress.md)、[MCP](mcp.md)、[SDK](driver-sdk.md)、[工作区与桌面](workspace.md)。
