# Alpha 交付说明

首个 Alpha 面向 Windows x64 的可信本地插件场景。桌面与 SSH 插件分开分发，不依赖云端市场；版本与发布标签由发行时统一确定。此文档描述发布准备，不代表已经创建 GitHub Release。

## 用户首次使用

1. 安装 Coterm Windows NSIS 安装包。
2. 顶部 **语言 / Language** 选择简体中文或 English；下次启动保留选择。
3. 下载同一发行版附带的 Windows x64 SSH `.coterm` 插件包。
4. 点击左侧 **安装插件 / Install plugin**，选择包并核对名称、ID、版本和平台；仅确认安装来自可信作者的包。
5. 选择左侧 SSH 插件，填写主机、端口和用户名创建实例，再连接并确认服务器指纹、输入密码。

安装不会执行插件。语言切换不影响现有 SSH 连接、终端内容和审批草稿；已安装插件无需重启应用即可列出。压缩包格式、限额与 SDK 语言 API 见 [插件开发](architecture/plugin-development.md)。

## 构建与检查

```bash
pnpm install --frozen-lockfile
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm plugin:ssh:build
pnpm build
pnpm test
pnpm tauri:build --bundles nsis
pnpm plugin:ssh:build --release
pnpm plugin:pack plugins/plugin-ssh/dist target/release/bundle/plugins/coterm-plugin-ssh-windows-x64.coterm
```

打包器拒绝覆盖已有输出文件；重新生成时先移除自己确认过的旧产物。打包不要求使用者安装 Rust 或 Node，工具链仅供开发与发布使用。CI 的 `v*` 标签任务同时上传 NSIS 和 SSH 离线包；不在本功能分支自动创建发布标签。

## 发布前验收

- Windows 干净用户目录：安装程序启动后不依赖仓库或编译工具，即可通过界面安装 SSH 离线包、创建实例和连接。
- 语言：系统默认语言、手动切换、重启保持；SSH 连接与终端对象不因切换重建，审批草稿不丢失。
- 安装：取消文件选择、坏 ZIP、入口缺失、不支持的平台、同 ID 冲突、暂存取消/清理、重启后发现已安装包。
- 安全：选择和预览阶段不运行包；确认前有可信来源提示；错误不暴露绝对路径；不授予任意文件访问或 shell 代理。
- Windows 实例创建：GUI 宿主启动插件不弹出黑色控制台；SSH 配置提交后显示未连接终端，点击连接后才发起远端连接。
- 已有回归：审批、公共交互、多实例、工作区加密恢复、MCP 和退出回收；远程 SSH 实机验收按 [实测记录](ssh-mihomo-field-test.md) 单独记录。
- MCP Stdio：面板启用后复制配置，用真实 MCP 客户端（`coterm-desktop mcp stdio`）完成初始化与工具调用；停用或重启后旧 Token 失效，未启用时面板不显示配置。

## 本轮边界

- 只增加宿主与 SSH 前端的简体中文/English；第三方插件可渐进接入，远端输出和领域诊断不进行自动翻译。
- 暂不支持覆盖安装、卸载、更新、回滚、签名、权限沙箱或云端市场。
- 首发仅承诺 Windows x64；macOS/Linux 已考虑包路径、执行权限及依赖差异，但仍需独立安装验收。
- 串口、本地 PTY 和 SSH 密钥/MFA/SFTP 等既有缺口继续按 [项目进度](PROGRESS.md) 跟踪。

## 2026-10-28 验收结果

- Rust workspace：`check`、99 个测试和 Clippy 通过；1 个物理串口测试按预期跳过。
- 桌面与 SSH Release 构建、35 个前端/浏览器测试通过。
- 重新构建并安装独立 Smoke NSIS，在真实 WebView2 和 Rust IPC 上验证原生文件选择取消、包预览、确认安装、即时发现及重启后发现；多实例、审批、交互、加密保存和退出回收回归通过。
- 修复 Windows GUI 宿主创建实例时弹出插件控制台的问题；新增控制台分配断言，旧安装包失败、修复后通过。真实 SSH 配置提交、后端握手与未连接终端显示已覆盖，未连接远端服务器。
- 产品 NSIS 重新构建，临时目录安装与卸载通过。产品标识下实际安装 SSH 离线包和连接远端，仍保留为发行前人工验收项。
- 产物位于 `target/release/bundle/nsis/Coterm_0.1.0_x64-setup.exe` 与 `target/release/bundle/plugins/coterm-plugin-ssh-windows-x64.coterm`；尚未创建 Alpha 标签或 GitHub Release。

新增 Rust 文件通过 rustfmt 检查；全仓格式检查存在既有差异，未在此功能中批量格式化。未覆盖远程 SSH 的新增实机回归、物理串口和 macOS/Linux 安装。
