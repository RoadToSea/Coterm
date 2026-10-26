# Coterm (Co-Terminal)

> **The Collaborative Terminal for Humans and AI Agents.**  
> 人类与 AI Agent 共生的模块化通信工具。

---

## 当前进度

Coterm 当前处于基础闭环收尾、插件运行时完善阶段。完整评估、优先缺口与验证范围见 [项目进度](docs/PROGRESS.md)，架构边界见 [SPEC](docs/SPEC.md)。

- **插件基座**：插件发现与坏包诊断、独立进程、多实例、可选插件前端和有界事件转发。
- **人机协作**：桌面和 Agent 关联同一实例；Agent 提交默认需要人工审批，密码通过受控交互交给插件。
- **SSH 与串口**：SSH 已接通共享 PTY、命令结果和 MCP；串口已有字节读写及界面初版，跨平台与关闭回收仍需完善。
- **MCP 接入**：固定本机 Streamable HTTP 入口与插件原生 Tools；Resources、Prompts、订阅尚未实现。
- **内部 SDK**：提供运行时类型、共享后端行协议和可选命令块列表组件；接口仍未冻结。本地 PTY、命令块上下文闭环和工业协议仍是后续工作。

---

## 目录结构

```text
Coterm/
├── apps/
│   └── desktop/                # Tauri 桌面端 GUI (Rust + React + xterm.js)
├── crates/
│   ├── coterm-proto/           # 统一 JSON-RPC 2.0 协议契约与数据模型
│   ├── coterm-core/            # 插件生命周期、实例路由、审批与操作账本
│   └── coterm-mcp/             # MCP HTTP Bridge 与开发用 stdio 适配器
├── packages/plugin-sdk/        # 插件前端契约与可选通用 UI
├── docs/                       # 架构、进度与阶段验收
└── plugins/                    # 独立进程插件
    ├── plugin-local/           # 当前运行时骨架，尚无 PTY
    ├── plugin-ssh/             # SSH MVP
    └── plugin-serial/          # 串口初版
```

---

## 快速开发与编译

### 环境要求
- **Rust / Cargo**：使用与当前 Tauri 依赖兼容的工具链；尚未建立 MSRV 验证矩阵。
- **Node.js**：本次使用 22.22.2；测试脚本要求内置 TypeScript 类型剥离和 `registerHooks` 支持。
- **pnpm**：使用 workspace 支持；当前验证版本见进度报告。

### 安装依赖
```bash
pnpm install
```

### 检查 Rust 工作区编译
```bash
cargo check --workspace
```

### 构建插件、桌面与回归测试

```bash
pnpm plugin:ssh:build
pnpm plugin:serial:build
pnpm build
pnpm test
cargo test --workspace
cargo clippy --workspace -- -D warnings
pnpm tauri:dev
```

浏览器用例需要本机 Chrome/Edge/Chromium，可用 `CHROME_BIN` 或 `EDGE_BIN` 指定。串口实机测试需显式配置设备；未配置不代表硬件验收通过。插件包只构建当前平台，发布与跨平台验收尚未完成。
