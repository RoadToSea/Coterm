# Coterm (Co-Terminal)

> **The Collaborative Terminal for Humans and AI Agents.**  
> 人类与 AI Agent 共生的模块化通信工具。

---

## 核心特性

- **万物皆插件 (Everything is a Plugin)**：极简轻量基座，所有通信协议（SSH、串口、本地 PTY、工业总线）均作为独立进程插件按需装配。
- **人机共生终端 (Human-in-the-Loop)**：人与 Agent 共享同一个终端视窗，具备人类抢占优先仲裁与拟真行内纠偏能力。
- **块级命令架构 (Command Blocks)**：基于 OSC 133 Shell Integration，终端输出自动切块，支持 VS Code 风格的 Gutter 勾选，精准拉取上下文，Token 消耗降低 90% 以上。
- **MCP 原生接入**：基座统一充当 MCP 网关与安全拦截器，赋能任意外部大模型。

---

## 目录结构

```text
Coterm/
├── apps/
│   └── desktop/                # Tauri 桌面端 GUI (Rust + React + xterm.js)
├── crates/
│   ├── coterm-proto/           # 统一 JSON-RPC 2.0 协议契约与数据模型
│   ├── coterm-core/            # 终端核心引擎 (输入仲裁、OSC 133 分块、会话管理、高危熔断)
│   └── coterm-mcp/             # MCP 聚合网关定义
└── plugins/                    # 官方首批独立进程插件
    ├── plugin-local/           # 本地 PTY 终端插件
    ├── plugin-ssh/             # SSH 远程连接插件 (待完善)
    └── plugin-serial/          # 串口通信插件 (待完善)
```

---

## 快速开发与编译

### 环境要求
- **Rust / Cargo**: 1.80+
- **Node.js**: v20+
- **pnpm**: v9+

### 安装依赖
```bash
pnpm install
```

### 检查 Rust 工作区编译
```bash
cargo check --workspace
```
