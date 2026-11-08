# 内置驱动开发

SSH、Serial、Local 位于 `drivers/driver-*`，随应用一起构建与分发。每个驱动有一个独立 Worker 进程，同一进程管理多个相互隔离的实例。基座只负责路由、审批、操作记录和公共交互，不解释领域命令或字节流。

## 后端

实现 `coterm_proto::runtime::backend::DriverBackend`，由共享 `serve` 入口处理 stdio JSON-RPC。stdout 只输出协议，日志写 stderr。控制协议为 v2.0，先握手再创建实例；manifest ID、版本和运行时目录必须一致。字段、边界和错误契约见[运行时通信](runtime-communication.md)。

操作通过显式 schema 声明 scope、effect、敏感输入及 MCP 工具。Agent 提交由宿主完成来源识别与每实例审批；准备提案不能执行写入。批准后重新校验参数，返回实际执行事实，不能确认时报告 unknown。命令文本审批声明见[命令审批](command-approval.md)。

领域接收数据和命令结果在驱动后端有界保留，前端另有显示副本；MCP 查询后端。Agent 使用 read_lines、read_selected_lines、read_dialogue_pairs，不维护游标。内部事件回放允许使用游标，过载必须报告缺口。各驱动的具体能力见[SSH](ssh-driver.md)、[串口](serial-driver.md)、[本地终端](local-driver.md)。

## 前端

从 `@coterm/driver-sdk` 导入类型和可选 UI，构建单文件 ES 模块。导出 `mount(container: HTMLElement, api: DriverAPI): DriverFrontend`，返回 `unmount(): void`，负责释放事件、计时器和终端。可选 `configure(container, api: DriverConfigurationAPI)` 提供创建前配置界面，提交配置后由基座创建实例。

SDK v2.0 将 driverId、driverInstanceId、sessionId 绑定到当前实例；invoke 不允许修改目标、source 或 effect。结果为结构化收据，失败包含 RuntimeError；等待超时不等于取消。可选 instanceConfigRef 是跨启动身份；saveConfiguration(config) 保存完整已验证配置，失败保留旧值，不能保存运行期秘密句柄。非 Windows 当前仅会话内保存。

subscribeEvents 在订阅安装后返回注销函数；处理器完成后确认投递。慢消费者和断线按驱动契约恢复，界面卸载不销毁实例。requestSecret 返回短期一次性句柄，秘密不进入日志、MCP、普通账本或明文配置。正式流 API 当前返回不支持。

可选组件的参数、返回值和卸载边界见[SDK UI 文档](../../packages/driver-sdk/docs/README.md)。前端与宿主共享 WebView，是可信应用代码，不构成第三方安全沙箱。

## 构建与验证

运行 `pnpm drivers:build` 构建三个驱动；`--release` 选择发行 Worker。暂存目录是 target/bundled-drivers，Tauri 将其映射到发行资源 drivers。manifest 仅声明本次构建实际支持的平台，入口必须限制在驱动目录内。

修改后执行 cargo check --workspace、相关 Rust 测试、pnpm build 和对应浏览器测试。原生全链路入口为 pnpm test:native；串口硬件参数按串口验收记录提供。Windows 已有实测不替代 macOS/Linux 和安装包验证。
