# 内置驱动开发

SSH、Serial、Local 位于 `crates/coterm-driver/src/{ssh,serial,local}`，共用一个 Cargo 包，随应用一起构建与分发。后端以 Rust 库直接链接进桌面进程，每种驱动有一个独立异步任务，管理多个状态隔离的实例。基座只负责路由、审批、操作记录和公共交互，不解释领域命令或字节流。

## 共享 Rust 模块

| 包 | 职责 |
|---|---|
| `coterm-core` | 共享数据模型、驱动接口、实例运行时、审批、秘密交互、操作记录及终端工具 |
| `coterm-driver` | SSH、Serial、Local 三个内部模块，分别管理协议连接、命令和接收数据 |
| `coterm-mcp` | MCP 工具发现与调用、本机 Bridge 和 Stdio 适配 |

桌面组合入口注册具体驱动，core 不依赖 driver 或 MCP。SSH、Serial 保留各自的领域缓冲；Local 使用 `coterm_core::terminal`。

## 后端

实现 `coterm_core::driver::DriverBackend`，由桌面的 `builtin_drivers` 将后端工厂、声明和嵌入的前端源码注册到 `Runtime`。`DriverTask` 使用有界 mpsc 请求队列与 oneshot 应答，不进行 stdio 编解码。控制数据契约为 v2.0，通过 definition() 提供固定目录，注册时校验 manifest ID、版本、能力和 schema；不执行运行时握手。字段、边界和错误契约见[运行时通信](runtime-communication.md)。

操作通过显式 schema 声明 scope、effect、敏感输入及 MCP 工具。Agent 提交由宿主完成来源识别与每实例审批；准备提案不能执行写入。批准后重新校验参数，返回实际执行事实，不能确认时报告 unknown。命令文本审批声明见[命令审批](command-approval.md)。

领域接收数据和命令结果在驱动后端有界保留，前端另有显示副本；MCP 查询后端。Agent 使用 read_lines、read_selected_lines、read_dialogue_pairs，不维护游标。内部事件回放允许使用游标，过载必须报告缺口。各驱动的具体能力见[SSH](ssh-driver.md)、[串口](serial-driver.md)、[本地终端](local-driver.md)。

## 前端

桌面与内置驱动通过相对路径引用 `apps/shared/` 中的类型和共享 UI，驱动前端仍构建为单文件 ES 模块。该目录是应用内部源码，不注册独立工作区包，也不依赖桌面组件或具体驱动实现。导出 `mount(container: HTMLElement, api: DriverAPI): DriverFrontend`，返回 `unmount(): void`，负责释放事件、计时器和终端。可选 `configure(container, api: DriverConfigurationAPI)` 提供创建前配置界面，提交配置后由基座创建实例。

前端契约 v2.0 将 driverId、driverInstanceId、sessionId 绑定到当前实例；invoke 不允许修改目标、source 或 effect。结果为结构化收据，失败包含 RuntimeError；等待超时不等于取消。可选 instanceConfigRef 是跨启动身份；saveConfiguration(config) 保存完整已验证配置，失败保留旧值，不能保存运行期秘密句柄。非 Windows 当前仅会话内保存。

subscribeEvents 在订阅安装后返回注销函数；处理器完成后确认投递。慢消费者和断线按驱动契约恢复，界面卸载不销毁实例。requestSecret 返回短期一次性句柄，秘密不进入日志、MCP、普通账本或明文配置。正式流 API 当前返回不支持。

可选组件的参数、返回值和卸载边界见[共享前端组件文档](../../apps/shared/docs/README.md)。前端与宿主共享 WebView，是可信应用代码，不构成第三方安全沙箱。

## 构建与验证

先运行 `pnpm drivers:build` 生成三个前端的单文件 ES 模块，再执行 Cargo / Tauri 构建。桌面通过 coterm-driver 库链接三个内部模块，通过 `include_str!` 嵌入声明与前端；manifest 不声明平台可执行入口，Tauri 不打包驱动资源目录。普通 `cargo build --workspace` 不生成三个独立驱动 EXE。前端源码修改后须重新运行 `pnpm drivers:build` 并重编桌面。

驱动停止、请求超时或任务 panic 后必须释放后端持有的会话与作业；不能依赖杀进程回收资源。阻塞设备读写放到专用线程或 `spawn_blocking`。普通 panic 的收敛依赖 unwind；native fault、abort、内存损坏与不可取消的底层阻塞可能影响整个宿主。Local 的真实 Shell 和 MCP stdio 适配器仍可使用子进程，不属于驱动 Worker。

`coterm_core::diagnostic::transport` 仅供显式诊断适配和线缆契约测试，不参与桌面驱动链路。串口测速先运行 `cargo build -p coterm-driver --example serial-diagnostic`；该示例不随桌面打包，测速结果不代表桌面派发性能。

修改后执行 cargo check --workspace、相关 Rust 测试、pnpm build 和对应浏览器测试。原生全链路入口为 pnpm test:native；串口硬件参数按串口验收记录提供。Windows 已有实测不替代 macOS/Linux 和安装包验证。
