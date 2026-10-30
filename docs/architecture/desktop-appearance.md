# 桌面界面与主题

更新：2026-10-30。

## 界面约定

桌面基座继续使用「插件栏 → 实例栏 → 插件工作区」布局，窄于 900px 时默认收起实例栏。工具栏提供 MCP、语言和外观入口；底栏展示真实的插件、实例数量及审批状态。

宿主引入 shadcn/ui 源码组件（Radix primitives、`new-york` 样式），配置位于 `apps/desktop/components.json`。沿用 React 18、Vite 5 与 Tailwind CSS 3，不升级框架、不新增 Tauri capability 或 IPC 协议。新增的 Radix、`class-variance-authority` 依赖仅用于浏览器 UI，平台无关；Windows 浏览器验证通过，macOS/Linux 原生外观仍需真机验证。

工具栏、操作按钮、计数、加载占位、空工作区和提示使用 Button、ToggleGroup、Badge、Skeleton、Empty、Alert、Separator。现有原生 `<dialog>` 保留焦点约束、Escape 关闭和关闭后焦点恢复，避免改变认证、审批与离线安装的生命周期。状态刷新、异步操作和订阅仍经宿主 API，主题切换不调用插件后端。

## 用户选择与启动

右上角依次提供：

- **太阳：浅色模式**。
- **月亮：深色模式**。
- **显示器：跟随系统**（首次启动默认）。

按钮有中英文可访问名称，使用 Radix 单选组的键盘导航。选择以 `light | dark | system` 保存在 `localStorage["coterm.theme"]`；只保存 UI 偏好，不保存会话、连接配置或秘密。存储不可用时，当前会话仍可切换。

`public/theme-init.js` 在应用模块和样式之前执行，读取偏好并设置根节点 `.dark`、`data-theme` 与 `color-scheme`，减少启动时错误配色闪烁。运行时的 `src/theme.ts` 延续同样的规则：显式浅色/深色覆盖系统；跟随系统时响应 `prefers-color-scheme`；其他窗口的偏好变化通过 `storage` 事件同步。

## 语义颜色与插件

`apps/desktop/src/index.css` 是颜色的唯一来源。shadcn 的 `background/foreground/card/primary/muted/accent/destructive/border/input/ring` 等语义颜色映射到 Tailwind；原有 `--coterm-*` 变量继续保留并引用这些颜色，供插件的 Shadow DOM 继承。正文与强调按钮前景分开：正文的 `--coterm-text-bright` 随主题变化，强调按钮使用 `--coterm-accent-foreground`。

终端渲染器不直接使用 CSS 变量字符串。SDK 的纯浏览器模块 `@coterm/plugin-sdk/components/terminal-theme` 提供：

```ts
const terminal = new Terminal({ theme: readTerminalTheme(container) });
const stopTheme = attachTerminalTheme(container, (theme) => {
  terminal.options.theme = theme;
});
// unmount: stopTheme(); terminal.dispose();
```

`readTerminalTheme` 读取继承的 `--terminal-*` 颜色，覆盖背景、前景、光标、选区和 ANSI 调色板。`attachTerminalTheme` 观察所属文档根节点的 `class/data-theme/style`，只有调色板变化才更新渲染器；卸载时必须注销。没有新颜色变量的旧宿主回退至深色。此模块不依赖 xterm 包，不改变 `PluginAPI` 或后端协议。

SSH、串口 Shell 和宿主终端组件均原地设置 xterm `options.theme`，不重建终端、实例、订阅或连接。搜索高亮同步主题；串口 UART 接收区、发送框与配置表单通过 `--coterm-*` 自动继承。人工 Shell 按键直发、UART 发送框、MCP 命令列表的边界保持原样。

## 验证

- `pnpm build`：宿主 TypeScript 与生产构建。
- `pnpm --filter coterm-plugin-ssh-ui build`、`pnpm --filter coterm-plugin-serial-ui build`：实际插件前端 bundle。
- `pnpm test`：主题偏好解析、持久化、不可用存储、实际浏览器渲染、系统主题变化、1280/1024/768 布局，以及语言、安装、审批、串口发送与终端快捷键回归。
- 主题浏览器用例使用实际 SSH/串口前端和宿主契约替身，验证画布颜色、终端身份、订阅次数、输出保留及无额外连接/发送。它不代替真实串口、远程 SSH 或各操作系统原生 WebView 验收。

浏览器截图写入忽略的 `target/desktop-theme-*.png`，用于人工核对两种配色和窄屏布局。
