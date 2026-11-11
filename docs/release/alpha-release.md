# Alpha 交付说明

发行应用包含 SSH、Serial、Local 三个内置驱动。用户安装桌面应用后直接创建实例，不再安装离线驱动包。首发目标为 Windows x64；版本号与标签由发布时确定。

## 构建

```bash
pnpm install --frozen-lockfile
pnpm drivers:build
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm build
pnpm test
pnpm tauri:build --bundles nsis
```

Tauri 的生产构建钩子先构建三个驱动前端，再将前端与三个 Rust 后端编入桌面可执行文件；安装包不包含独立驱动 EXE 或驱动资源目录。用户运行时不需要仓库、Rust 或 Node。CI 上传 NSIS，标签发布不再附带 .coterm 文件。

## 发布前验收

- 干净 Windows 用户环境安装、启动与卸载；直接列出 SSH、Serial、Local，设置中无驱动安装管理入口。嵌入模块加载不依赖仓库、旁路驱动 EXE 或 app-data 中的旧驱动目录。
- SSH 指纹确认、认证、前台命令与后台分流；Local PowerShell/cmd 前台环境延续与后台输出；Serial stream/Shell 能力边界、提示符和拔插恢复。
- MCP 使用 coterm-desktop mcp stdio，验证真实客户端、审批面板、实例隔离和后台结果读取。连接参数与端口持久化见[MCP](../architecture/mcp-integration.md)。
- 工作区旧格式迁移、新 ID 与独立审批配置保留；恢复分配新运行 ID、不自动连接、不重放命令；密码与秘密不进入普通日志或 MCP。
- 明暗主题、语言切换、窄窗口、标签恢复、终端事件缺口和退出回收。

## 已知限制

非 Windows 的安装、真实 Local Shell 和系统密钥库仍需独立验收。Windows 工作区使用 DPAPI，其他平台暂不持久化。后台取消不能保证回收完整后代进程树。原生 WebView2/MCP/COM6 验收仅覆盖实际测试路径，不等于 NSIS 安装验收。实施与验收记录见[项目进度](../progress/project.md)。
