# 桌面界面与主题

更新：2026-10-30。

## 界面约定

桌面基座继续使用「插件栏 → 实例栏 → 插件工作区」布局，窄于 900px 时默认收起实例栏。顶部只保留应用标识、语言和外观入口，不重复显示当前实例名称，也不提供 MCP 按钮。实例栏标题提供收起按钮；收起后在插件栏出现展开按钮，任何时刻只有一个可见的面板切换入口。插件栏不再显示「已安装插件」文字标题，保留导航的可访问名称。底栏展示真实的插件、实例数量及审批状态。

实例栏以具名工作区分组，标题旁的加号创建分组；组内支持运行实例和待恢复实例拖放，实例工具弹窗中的下拉框提供键盘操作替代。删除工作区需确认，组内实例移至剩余第一个工作区，不断开连接；至少保留一个工作区。实例与恢复记录均为圆角卡片，图标与插件栏保持一致；运行实例右侧横向排列更多和删除按钮，仅悬停或键盘焦点进入卡片时显示。元数据与恢复规则见[运行时通信](runtime-communication.md#工作区与生命周期)。

点击插件时，在主工作区内联展示该插件的连接配置表单；不弹出新建实例窗口，不显示插件操作调用器、参数 JSON 或高级 JSON 配置。无配置前端时显示说明。基座只负责挂载和提交，不了解通信协议字段；SSH/串口配置控件在插件自身 Shadow DOM 中使用主题圆角。

主区域上方的实例标签页负责切换已打开实例。左侧工作区保留分组、恢复、移动和销毁等配置/生命周期管理入口；“更多”仅显示名称、插件、状态、分组移动和删除，不显示内部 UUID、会话 ID 或操作调试器。新建、恢复和从侧栏打开实例会选择相应标签。首次加载打开现有运行实例，后续轮询不自动打开后台新增或已关闭视图。

切换标签或进入配置页只隐藏实例面板，保留终端输出、任务状态、前端及订阅。关闭标签仅卸载视图、回收订阅，不关闭连接或删除保存项；侧栏可再次打开。销毁仍需确认，实例失败/移除同步清理挂载。标签支持方向键、Home/End、Delete 和关闭按钮；关闭当前项选中相邻项，关闭最后一个进入空状态。窄屏横向滚动且选中项随窗口/实例栏尺寸变化保持可见。标签打开状态暂不跨启动保存；原有活跃实例与分组布局持久化不变。实施方案见[实例标签页设计](../design/instance-tabs.md)。

主窗口配置 `dragDropEnabled: false`，允许 Windows WebView2 使用 HTML5 内部拖放；此前没有依赖原生文件拖放的产品入口。外部文件拖入窗口只阻止默认导航，插件安装仍由专用文件选择器完成。该配置适用于 Windows/macOS/Linux；本次浏览器拖放验证不代替其他平台原生 WebView 验收。

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
