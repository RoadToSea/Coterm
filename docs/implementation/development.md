# 开发与验收

本文件集中构建、测试、硬件与发布入口。模块行为见[需求](../requirements/)，代码关系见[代码架构](architecture.md)，已验证能力见[实现与验收状态](verification.md)，未解决现场问题见[feedback](../feedback/known-issues.md)。

## 环境与构建

在仓库根目录执行。使用兼容 Tauri v2 的 Rust 工具链，Windows CI 当前固定 Rust 1.97.0、Node 22.22.2，pnpm 版本由根 packageManager 固定为 11.0.8。浏览器测试依赖 Node 的类型剥离与 registerHooks。

```powershell
pnpm install --frozen-lockfile
pnpm drivers:build
pnpm tauri:dev
```

仅前端开发用 pnpm dev；桌面前端生产构建用 pnpm build。修改驱动前端后重新构建对应驱动或运行 pnpm drivers:build，再重编桌面。后端由 coterm-driver 库链接，五个前端和声明嵌入桌面 EXE，运行用户不需要仓库、Node、Rust 或独立 driver.exe。

## 软件检查

```powershell
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm drivers:build
pnpm build
pnpm test
git diff --check
```

修改 Rust 至少执行工作区 check 与相关测试；修改前端执行生产构建和对应浏览器回归。文档变更检查本地链接、锚点、源码定位、命令名称和差异空白；不把历史软件验收写成这次重新执行。

浏览器测试需要 Chrome／Edge／Chromium，可用 CHROME_BIN 或 EDGE_BIN 指定。跳过浏览器或真实硬件用例不等于通过。pnpm test 会构建共享测试入口，但不能替代五个驱动前端和桌面前端构建。

| 需求范围 | 定向入口 |
|---|---|
| 实例、控制与故障 | [core 控制测试](../../crates/coterm-core/tests/control_contract.rs)、[运行时夹具](../../tests/fixtures/runtime-driver/tests/lifecycle.rs) |
| 工作区、保存、迁移、Bypass | [workspace_tests.rs](../../apps/desktop/src-tauri/src/workspace_tests.rs)、[工作区浏览器](../../tests/workspace-browser.test.mjs) |
| 审批组与界面 | [规则实现与单元测试](../../crates/coterm-core/src/approval_policy.rs)、[审批浏览器](../../tests/approval-settings-browser.test.mjs) |
| SSH、Local、Serial | [driver/tests](../../crates/coterm-driver/tests)、[tests](../../tests) 中对应驱动浏览器用例 |
| Telnet／Raw TCP | [网络测试](../../crates/coterm-driver/src/network/tests.rs)、[codec](../../crates/coterm-driver/src/telnet/codec.rs)、[网络浏览器](../../tests/network-drivers-browser.test.mjs) |
| MCP Stdio 与目录 | [coterm-mcp/tests](../../crates/coterm-mcp/tests)、各驱动目录与 MCP 集成测试 |
| 前端公共交互 | tests 下 terminal-shortcuts、command-blocks、toasts、theme、serial-display-preferences 等用例 |

## 原生与发行验收

pnpm test:native 使用 Windows WebView2、隔离 smoke 配置、本地 Shell、网络回环与真实 MCP Stdio；脚本拒绝覆盖已有 smoke profile。先构建带测试配置的专用版本，再指定该产物运行：

```powershell
pnpm tauri:build --config ../../tests/fixtures/tauri-smoke.conf.json --bundles nsis
pnpm test:native target/release/coterm-desktop.exe
```

脚本不传路径时默认 debug 产物，但仍要求 smoke 配置，不能用普通桌面 EXE 代替。细节见[原生脚本](../../scripts/test-native.mjs)和[Windows CI](../../.github/workflows/windows.yml)。

Windows 安装包：

```powershell
pnpm tauri:build --bundles nsis
```

发布前执行：

1. 干净 Windows 用户安装、启动和卸载；五个驱动直接可用，不依赖旧 app-data 驱动文件或旁路 EXE。
2. SSH 指纹、密码、前后台命令、独立错误流与取消；Local 已支持 Shell 的前台状态延续和后台结果。
3. Serial 端口枚举、拔出状态、Shell／stream、提示符与断线不重放。
4. Telnet 真实协商和人工登录、Raw TCP 文本／HEX、实际代理与重连不重放。
5. 真实 MCP 客户端、审批编辑、实例隔离、配置恢复和无秘密泄露。
6. 语言主题、窄窗口、标签、终端底行、慢消费者缺口、退出和资源回收。

CI 在 main／develop 推送、PR 或手动触发时检查并产出 NSIS，v* 标签发布安装产物。测试安装器见[scripts/test-installer.ps1](../../scripts/test-installer.ps1)，实际分发前仍需确认[未解决问题](../feedback/known-issues.md)。macOS／Linux 安装和原生行为独立验收。

## 真实串口

以下均会访问显式选择的设备，仅在用户授权写入、设备已登录并释放端口后执行。例子中的占位符必须换成实际设备，不能把上次 COM6 记录当成每台机器的默认值。普通回归不自动运行 ignored 硬件用例。

```powershell
$env:COTERM_SERIAL_LIVE = "<实际端口>"
$env:COTERM_SERIAL_LIVE_BAUD = "115200"
cargo test --locked -p coterm-driver --test serial_live -- --ignored --nocapture
```

手工连接入口为 cargo run -p coterm-driver --example serial-live -- <实际端口> 115200。

Shell 专项只使用回车、printf 和 uname 等已授权命令，不自动登录或保存密码：

```powershell
$env:COTERM_SERIAL_SHELL_LIVE = "<实际端口>"
$env:COTERM_SERIAL_SHELL_PROMPT = "<完整可见提示符>"
$env:COTERM_SERIAL_LIVE_BAUD = "115200"
cargo test --locked -p coterm-driver --test serial_shell_live -- --ignored --nocapture
```

虚拟串口需先创建互联且未占用的一对端口：

```powershell
$env:COTERM_SERIAL_PAIR_A = "<端口 A>"
$env:COTERM_SERIAL_PAIR_B = "<端口 B>"
$env:COTERM_SERIAL_PAIR_BAUD = "115200"
cargo test --locked -p coterm-driver --test serial_virtual_pair -- --ignored --nocapture
```

测试范围以[实际用例](../../crates/coterm-driver/tests)为准；物理线路、提示符识别和真实桌面不是同一验收路径。

## 性能与完整性

必须同时报告实际投放、系统接收／发送、消费字节和内容校验；保留失败负例，不只看成功样本均值。明确方向是交替还是同时全双工，说明是否包含 Runtime、桌面和 MCP。

### TCP

```powershell
cargo build --locked -p coterm-driver --release --example tcp-task-throughput --example network-diagnostic
cargo run --locked -p coterm-driver --release --example tcp-task-throughput -- 60 5242880,10485760,52428800,104857600,0
node scripts/benchmark-tcp.mjs --seconds 60 --rates 5242880,10485760,unlimited --output .agent-debug/tcp-throughput/report.json
```

第一个示例使用生产 DriverTask 队列，两个投放任务独立，逐字节验证带序号的 16 KiB ASCII 记录；第三个位置参数为 0–5000 ms 的消费者暂停。Node 路径额外经过诊断 JSON／stdout，另有整流 SHA-256；两者都不包含完整 Runtime 账本、WebView、xterm、MCP 或审批。

0／unlimited 只表示不主动限速，不是已验证无损速率。重复、乱序、缺失、异常帧或广播遗漏使程序非零退出，包括故意慢消费用例。后端计数完整与历史未淘汰是不同条件。

### Serial

设备需要 Python 3、POSIX tty 和脚本可识别的 # 提示符。脚本只使用内存中的临时设备程序，不接收密码；默认交替收发，长测使用显式 duplex。

```powershell
cargo build --locked -p coterm-driver --release --example serial-diagnostic
node scripts/benchmark-serial.mjs --port <实际端口> --baud 115200 --rates 115200,500000,1500000 --bytes 131072 --repeats 1 --seconds 600 --transfer-mode duplex --diagnostic target/release/examples/serial-diagnostic.exe --output .agent-debug/serial-throughput/report.json
```

其他平台去掉示例产物的 .exe。8N1 理论数据速率为波特率除以 10；配置接受波特率不等于实测支持。脚本走真实 Serial Backend 的诊断适配、事件和写入，不包含桌面完整链路。

每轮两个方向使用独立随机标识、方向和轮次的 SHA-256 文本负载；主机接收逐字节核对，设备接收校验和。报告记录事件缺口、UART 计数和失败时后端／通知对照；只有完整当前轮边界才据回放推断缺失数量，回放最多 1024 页。

调速前保存 tty 属性，测试时关闭 IXON／IXOFF／IXANY；设备 30 秒无命令退出并恢复，失败恢复额外等待 32 秒。主机切回原波特率并确认 Shell 后释放串口。restored 是恢复确认，verified 才是该档校验通过。任意完整性失败非零退出，不能将零 UART 错误当作无损证据。

## 调试产物

脚本、日志、截图、测速报告和备份放在相对目录 .agent-debug/<任务名>/，不写入模块源码目录。日志先脱敏，不写密码。构建缓存与编译产物仍留工具链目录。

调试结束将已验证能力归入[实现与验收状态](verification.md)，将未解决现象和证据归入[feedback](../feedback/known-issues.md)，停止自己启动的进程并确认没有其他任务使用该目录，再执行：

```powershell
pnpm debug:clean --dry-run
pnpm debug:clean
Get-ChildItem .agent-debug -Force
```

只通过该脚本清理，范围固定为 .agent-debug 内容并保留目录本身；失败报告残留，不扩大删除范围。浏览器等工具在系统临时目录建立的 profile 由创建者退出时回收。
