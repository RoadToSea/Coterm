# 插件前端 SDK

## 当前能力

- SDK v1.0 的实例绑定、操作调用、事件与公共交互入口见 [插件开发](../architecture/plugin-development.md)。
- 公共 UI 文档见 [SDK 文档入口](../../packages/plugin-sdk/docs/README.md)，组件保持无框架依赖，协议数据由插件提供。

## 2026-11-01：命令消息块与选择

- 将原先仅单行命令的 `components.blocks()` 完善为编辑器式布局：左侧块号按钮，右侧命令和回复共同组成有边框的块。
- 点击块号多选 / 取消并高亮；正文可原生选中复制。支持 Tab、Enter / Space、上 / 下方向键和 Home / End。
- `CommandBlock` 新增可选 `output`；props 可定制列表可访问名称与空文案。按稳定 ID 更新，保留未显式覆盖的本地选择、现有焦点和未变化文本的选区。
- 复用宿主浅色 / 深色 token、支持 Shadow DOM 和窄窗口；DOM 使用纯文本，不解释远端 HTML。
- 提供 [可运行示例](../../packages/plugin-sdk/examples/command-blocks.html) 与 [使用说明](../../packages/plugin-sdk/docs/command-blocks.md)。

验收入口：`pnpm build`、`pnpm test:build` 和 `node --import ./scripts/test-ts-loader.mjs --test tests/command-blocks-browser.test.mjs tests/visual-tokens-layout.test.mjs`；已有宿主布局回归同时更新块号选择断言。

本次验收：`pnpm build` 通过；完整 `pnpm test` 为 56 项通过、0 失败、0 跳过。真实浏览器检查命令 / 回复边框、块号多选高亮、键盘、复制、增量更新、滚动位置与卸载，并人工复核浅色 / 深色及 320px 窄窗口截图。

## 2026-11-01：插件直接引用与后端职责

- 将 SDK 文档与插件开发指南的首选接入入口统一为插件前端直接导入 `mountBlockList`，随插件自己的 bundle 构建；宿主 `api.components.blocks` 保留兼容。
- 明确插件后端生成命令 / 回复归属、块 ID、编号及执行结果，前端通过基座获取并适配数据；组件只承担渲染和选择，选择状态由前端保存。
- 静态示例分开模拟后端块快照和前端已选 ID，刷新后端回复不携带或重置选择。
- 使用方式为插件后端提供结构化块数据、插件前端直接引用组件。`CommandBlock` 是前端显示类型；静态示例使用模拟后端快照，SSH 已接入真实后端日志和实例事件，见 [SSH 进度](ssh-plugin.md)。

本次验收：`pnpm build`、`pnpm test:build` 通过；块组件与主题布局定向测试 7 项通过、0 失败、0 跳过，含真实浏览器中的后端快照回复刷新与前端选择保持。

- SSH 首次实际接入：增加可选 `statusText`，由插件传入已本地化的通道 / 状态文案；公共组件不解释插件执行状态。现有字段与宿主挂载方式继续兼容。

## 2026-11-01：公共非模态提示

- 新增 `@coterm/plugin-sdk/components/toasts`，插件前端可直接引用 `mountToasts`，支持普通 DOM 与 Shadow DOM，无框架或新依赖。
- 圆角矩形，error 红色、warning 黄色、info 主题强调色；默认 4 秒自动消失，不抢焦点、不遮挡背景操作。支持手动关闭、悬停 / 键盘聚焦暂停、相同消息合并和最多 5 条的有界提示。
- 文案按纯文本渲染，可访问名称可随语言更新，卸载移除提示、样式和计时器；串口插件已用于配置锁定、HEX 校验、队列与操作失败提示。
- 接入、选项和示例见 [公共提示组件](../../packages/plugin-sdk/docs/toasts.md)。首个实际接入见 [Serial 进度](../../plugins/plugin-serial/docs/progress.md)。

验收：`pnpm build`、`pnpm test:build` 通过；完整 `pnpm test` 为 58 项通过、0 失败、0 跳过。公共组件真实浏览器用例检查颜色、圆角、非模态、焦点、自动消失、暂停、去重、容量、纯文本、本地化及卸载；浅色错误 / 警告截图完成复核。

## 2026-11-01：公共显示偏好

`terminal-theme` 模块增加可选 `readTerminalPreferences / attachTerminalPreferences`，读取并观察宿主终端字号、行高和光标闪烁变量；旧宿主有默认回退，卸载须注销。命令块正文继承终端字号，SSH / 串口和宿主终端已接入原地更新；接口与边界见 [桌面外观](../architecture/desktop-appearance.md#语义颜色与插件)。完整前端回归 62 项通过，含观察器去重 / 注销、活跃实例实时更新和无额外连接 / 写入验证。

## 2026-11-02：xterm 命令行号栏

- 新增可选 `@coterm/plugin-sdk/components/terminal-blocks`，结构化适配 xterm marker / decoration，不让 SDK 依赖终端包。插件提供可信块边界并同步当前有界块 ID；组件绘制 Gutter、选中行背景、边框与「取消全选」，调用回调同步实例上下文。
- 支持 buffer 视口滚动 / resize / render、Alternate Screen 隐藏、32 块上限、marker 淘汰清理和区域局部 Esc。SSH 已应用于同一主终端，没有互斥 Terminal / Command blocks Tab。
- 示例 SDK browser bundle 加入 terminal-blocks；接口说明见 [组件文档](../../packages/plugin-sdk/docs/command-blocks.md#终端行号栏)，真实集成测试为 `tests/ssh-blocks-browser.test.mjs`。
- SSH 终端来源标签与 Agent 上下文选择见 [SSH 交互设计](../design/ssh-terminal-interaction.md) 及 [SSH 进度](ssh-plugin.md)。

验收入口：`pnpm build`，`pnpm --filter coterm-plugin-ssh-ui build`，`pnpm test:build`，以及 SSH blocks / jobs / terminal shortcuts 浏览器用例。

- 不在基座猜测 SSH、串口或本地终端的命令边界，不自动将已有终端改成块视图；各插件后端仍需按协议提供可信块数据。
- 每种插件自行处理截断、缺口和秘密脱敏；选中上下文由各插件自己的实例绑定操作提供。
- 该组件展示纯文本，不是 xterm 替代品；已有终端仿真、认证与审批流程保持各自职责。
- SDK Terminal Gutter 继续入口：`packages/plugin-sdk/src/components/terminal-blocks.ts`；结构化块列表仍见 `packages/plugin-sdk/src/components/block-list.ts`。
