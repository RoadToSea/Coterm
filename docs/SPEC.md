# Coterm (Co-Terminal) — 架构与规格说明书

> **CLI**: `coterm` | **Slogan**: *The Collaborative Terminal for Humans and AI Agents.*  
> 本文档定义了 Coterm 的系统架构、交互标准、插件协议与安全基线。

---

## 一、 产品定位与核心设计哲学

**Coterm (Co-Terminal)** 对标 MobaXterm，打造一款**轻量、安全、可插件化拼接**的跨平台通信客户端，并为 AI Agent 赋予第一视角的设备与环境操作能力。

* **Everything is a Plugin（万物皆插件）**：基座保持极简，通信协议（SSH、串口等）均以插件形式按需安装与加载，冷启动秒开。
* **Human-Agent Co-Terminal（人机共生终端）**：人与 Agent 共享同一个交互窗口，所有操作对人类完全透明，人类拥有随时接管、逐条审核、直接纠偏的终极控制权。
* **MCP-Native（原生大模型上下文协议）**：基座统一充当 MCP 网关与安全拦截器，将底层设备通信能力无缝暴露给上层 AI。

---

## 二、 基础技术栈与架构决策 (Decisions 1~3)

| 架构维度 | 最终选型 | 决策依据与细节 |
|---|---|---|
| **客户端宿主** | **Tauri + Rust + 前端 (xterm.js)** | 相比 Electron 极大幅度降低内存开销与安装包体积；Rust 负责高并发底层 I/O、安全凭据与进程管理；前端 xterm.js 呈现工业级终端仿真与多标签分屏。 |
| **插件进程模型** | **独立子进程隔离 (Out-of-Process)** | 插件独立于主应用运行，避免单点崩溃导致主窗口闪退；插件开发者可自由选择 Rust、Python、Go、Node.js 等任意语言实现。 |
| **第一期 MVP 协议** | **SSH + 串口 (Serial) + 本地终端 (Local PTY)** | 完整闭环覆盖云服务器、嵌入式硬件调试与本机工作流。 |

---

## 三、 人机共生交互机制 (Decisions 4~7, 9)

### 1. 输入仲裁与竞态处理 (Input Arbiter)
* **默认模式：人类抢占优先 (Preemptive Interruption)**
  * 当 Agent 正在连续执行命令序列时，**人类只要在终端键入任意按键**，基座立即暂停 Agent 的后续输入序列，触发中断事件并通知 Agent 当前已被人类接管。
* **Bypass 模式（全自动运行）**：
  * 支持一键切换为 Bypass 模式，允许 Agent 自主连续无审查输入，适用于成熟脚本或长时间自动化批处理。

### 2. 命令执行完成感知 (Prompt & Completion Detection)
* **现代 Shell 环境（SSH / 本地终端）**：采用 **OSC 133 (Shell Integration)** 终端协议，由底层精确获知命令启动、结束时间及精确的退出码（Exit Code）。
* **嵌入式/串口环境（无确定退出码场景）**：基座提供通用能力，**允许 Agent 通过 MCP 动态配置本次调用的匹配规则**（如配置 `idle_timeout_ms` 静默超时毫秒数、或配置特定的 `prompt_regex` 正则匹配，如 `root@device:#`）。

### 3. 密码与敏感凭据防泄漏 (Password Cloaking)
* **底层自动脱敏**：
  * 终端核心驱动自动侦测 **无回显模式（no-echo，如 `sudo` 密码阶段）** 或特征密码提示符。
  * 检测触发时，前端输入以圆点遮蔽，**基座彻底阻断该段原始字符向 Agent/LLM 上下文的传输**，会话记录中该片段被统一脱敏置换为 `[REDACTED_CREDENTIAL]`。

### 4. 审批与纠错的 UI 交互 (Inline Ghost Text)
* **拟真行内预览**：在逐条审核模式下，Agent 准备发送的命令直接以**灰色拟真预览文字（Ghost Text）**显示在当前终端的 Prompt 光标后。
* **无缝键盘纠偏**：
  * 按 **Enter**：直接放行该命令；
  * 按 **Tab**：接受该命令到输入框；
  * **直接打字/退格**：用户可就地修改 Agent 拟写的命令，改完后回车，实际发送人类纠正后的版本。

### 5. 屏幕历史视觉归属 (Visual Attribution)
* **视觉区分**：Agent 敲入的命令在屏幕历史中带有专属 **🤖 图标标识** 与柔和背景微高亮（如浅紫色底纹），人类亲自键入的命令保持原生终端样式，责任边界一目了然。

---

## 四、 基座与插件的职责边界 (Decisions 8, 10, 11)

```
┌────────────────────────────────────────────────────────────────┐
│                           AI Agent                             │
└───────────────────────────────┬────────────────────────────────┘
                                │ 标准 MCP (JSON-RPC)
┌───────────────────────────────▼────────────────────────────────┐
│                      Tauri 宿主基座 (Host)                     │
│  - 插件生命周期与配置装配 (manifest.json, 按需拉起)             │
│  - 统一 MCP 路由网关: comm__<plugin_id>__<tool_name>           │
│  - 安全与审批拦截器 (Inline Ghost 审批 / 熔断黑名单)           │
│  - 敏感凭据底层脱敏过滤器                                      │
└───────────────────────────────┬────────────────────────────────┘
                                │ JSON-RPC over Stdio (标准输入输出)
┌───────────────────────────────▼────────────────────────────────┐
│                     独立进程通信插件 (Plugin)                  │
│  - 领域特定的语义化 MCP Tools (由插件自主定义并上报)           │
│  - 硬件/协议物理驱动连接 (libssh, serialport, libusb 等)       │
└────────────────────────────────────────────────────────────────┘
```

### 1. 职责划分
* **基座负责（基础设施）**：
  1. 插件进程管理（拉起、心跳、崩溃重启、安全释放）。
  2. 统一 MCP Server 网关路由，把不同插件暴露的工具动态映射至 `comm__<plugin_id>__<tool_name>`。
  3. 统一审批与输入仲裁管道。
  4. xterm.js 窗口渲染与双向数据流分发。
* **插件负责（领域逻辑）**：
  1. 负责具体硬件/网络的连接维护（如处理串口波特率、SSH 握手）。
  2. **自主定义并暴露高阶语义化 MCP 工具**（例如 SSH 插件暴露 `ssh_exec_command`, `ssh_send_keys`；未来的 Modbus 插件自主暴露 `modbus_read_holding_registers`），不强求协议间统一。

### 2. 插件 IPC 通信协议
* 基座与独立插件进程之间统一采用 **JSON-RPC 2.0 over Stdio**。任何语言编写的独立可执行文件，只要能够读写标准输入输出（stdin/stdout），即可轻松挂载进基座。

### 3. 插件包与分发标准
* 插件以**目录包 (Directory Bundle)** 格式发布，支持客户端应用内下载解压或加载本地开发目录，核心包含两部分：
  * `manifest.json`：定义插件 ID、版本、展示图标、连接所需的 UI 表单 Schema（如端口输入框、下拉框）、启动可执行文件路径。
  * `bin/`：各平台预编译的二进制可执行程序或启动脚本。

---

## 五、 安全与高危熔断机制 (Decision 12)

* **高危规则熔断器（可配置开关）**：
  * 在设置中提供“**高危命令熔断保护 (Safety Guardrails)**”总开关，由用户决定是否启用。
  * **生效机制**：当开启熔断时，即使当前处于“Bypass / 自动审批模式”，一旦 Agent 拟发送的命令命中系统预置或自定义的高危黑名单规则（如 `rm -rf /*`、`mkfs`、`reboot`、扇区直接擦写等），基座**强制剥夺自动放行权限**，必须弹窗经用户二次确认后方可送达底层通道。

---

## 六、 块级命令架构 (Command Block) 与上下文精控 (Decision 13)

### 1. 核心问题与设计动机
传统终端流是一整段连续的字符流，如果 Agent 为了排查一个错误而读取全部屏幕上下文，会产生大量与故障无关的输出（如 `ls`、中间目录切换、重复清屏），造成严重的 **Token 浪费与上下文污染**。

本系统引入类似现代终端（Warp / VS Code Shell Integration）的 **“块级命令 (Command Item)”** 架构，将无序的终端流切分为具有完整生命周期的离散原子块。

### 2. 自动分块原理 (OSC 133 自动切块)
借助 OSC 133 标准终端协议，基座能够精准捕获每个命令的四个生命节点：
* `OSC 133;A` (Prompt 出现)
* `OSC 133;B` (Command 输入结束，回车敲下)
* `OSC 133;C` (Output 输出流开始)
* `OSC 133;D;exit_code` (命令结束，获取最终退出码)

对于无 OSC 133 的嵌入式/串口环境，基座自动降级为“Prompt 正则匹配 + 空闲静默超时”进行启发式切块。

### 3. 结构化数据模型 (CommandItem)
终端内部维护一个结构化命令项列表：
```typescript
interface CommandItem {
  id: string;              // 块唯一 ID，如 "cmd_42"
  index: number;           // 序号
  timestamp: number;       // 发起时间戳
  cwd: string;             // 执行路径（如 "/var/www"）
  command: string;         // 输入指令文本（如 "npm run build"）
  exitCode: number;        // 返回码：0 为成功，非 0 为异常
  output: string;          // 专属该命令的 stdout + stderr 纯文本
  actor: "human" | "agent";// 执行者身份
  isSelected?: boolean;    // 是否被用户勾选
}
```

### 4. UI 侧边栏交互 (Gutter Checkbox & Quick Actions)
* **左侧边栏勾选区 (Gutter)**：
  * 每条命令左侧支持类似 VS Code 的行号区域，用户可点击勾选框 (Checkbox) 单选或按 `Shift` 多选命令块。
  * 失败命令左侧呈现醒目的 **红色 ❌ 图标** 与退出码。
* **快速求助操作 (Quick Actions)**：
  * 鼠标悬浮失败命令时弹出小悬浮药丸：`[🤖 让 Agent 修复]`，点击直接以此 Block 为上下文呼出 Agent 对话，无需人类手动复制报错。

### 5. MCP 通信范式与精准拉取 (Push + Pull 混合)
* **用户主动投喂 (Push)**：人类点击求助时，前端直接把勾选的 `CommandItem` 结构体装配进 Agent 会话的初始消息中。
* **Agent 按需精准拉取 (Pull)**：基座向 Agent 暴露专用精控查询工具：
  * **工具名称**：`comm__terminal__get_command_blocks`
  * **支持参数**：
    * `target: "selected"`（仅读取人类在界面勾选的项目）
    * `target: "last_failed"`（仅读取最近一条退出码非 0 的失败命令）
    * `target: "by_id"`（按 Block ID 指定获取）
    * `max_output_lines: 100`（防爆保护：自动提取头部 50 行与尾部 50 行，丢弃中间冗余输出）
* **收益**：单次错误排障的 Token 消耗从上万 Token 极限压缩至 **100~300 Token**，且没有无关字符干扰，大模型诊断精准度大幅提升。

---

## 七、 全局主题系统实现架构 (Decision 14: Theme Engine)

### 1. 主题数据结构定义 (Theme Schema)
```typescript
interface CotermTheme {
  id: string;               // 唯一标识，如 "coterm-dark", "dracula"
  name: string;             // 展示名称
  isDark: boolean;
  
  // 终端字符层 (直接映射至 xterm.js ITheme)
  terminal: {
    background: string;
    foreground: string;
    cursor: string;
    cursorAccent?: string;
    selectionBackground: string;
    black: string; red: string; green: string; yellow: string;
    blue: string; magenta: string; cyan: string; white: string;
    brightBlack: string; brightRed: string; brightGreen: string; brightYellow: string;
    brightBlue: string; brightMagenta: string; brightCyan: string; brightWhite: string;
  };

  // 应用外壳 UI 层 (注入 CSS Variables)
  shell: {
    bgApp: string;          // 主背景色
    bgSidebar: string;      // 侧边栏背景
    bgHeader: string;       // 顶部标签栏背景
    bgFooter: string;       // 底部状态栏背景
    border: string;         // 边框描边
    textPrimary: string;
    textMuted: string;
    accent: string;         // 主品牌色
  };

  // 人机协同特色元素
  agent: {
    commandBgTint: string;  // Agent 命令行微弱背景色
    ghostTextColor: string; // Ghost Text 待审批拟真文字色
    badgeColor: string;     // 🤖 胶囊指示灯色
  };
}
```

### 2. 双轨动态热重载机制 (Zero-Reload Hot-Swapping)
1. **外壳层响应**：通过全局 `ThemeContext` 监听到切换事件，直接修改 `:root` 节点的 CSS Custom Properties（如 `--ct-bg-app`, `--ct-accent`），Tailwind 与 UI 依靠原生 CSS 变量瞬间重绘，零闪烁。
2. **终端层响应**：多标签页/多分屏下的每个活跃 `Terminal` 实例，直接响应 `terminal.options.theme = newTheme.terminal`，无需释放/重新挂载 DOM 容器，输入缓冲区与历史屏幕字符完美保留。

### 3. 主题存储与本地发现
* 预设主题内置在编译资产中；
* 用户自定义主题与外部导入主题统一落盘于用户主目录：`~/.coterm/themes/<theme-id>.json`，应用启动时自动扫描挂载。