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
- 当前 `CommandBlock` 是前端显示类型；尚未新增通用后端块码、事件或操作，实际后端返回格式与具体插件接入仍需确定。

本次验收：`pnpm build`、`pnpm test:build` 通过；块组件与主题布局定向测试 7 项通过、0 失败、0 跳过，含真实浏览器中的后端快照回复刷新与前端选择保持。

## 限制与后续入口

- 本次交付插件可复用组件及示例；不在基座猜测 SSH、串口或本地终端的命令边界，不自动将已有终端改成块视图。
- 每种插件的后端需从自己的协议 / 命令记录生成有界块数据，处理截断、缺口和秘密脱敏；插件前端适配为 blocks 快照，并决定选中上下文的用途。
- 该组件展示纯文本，不是 xterm 替代品；已有终端仿真、认证与审批流程保持各自职责。
- 继续开发从 `packages/plugin-sdk/src/components/block-list.ts`、对应浏览器测试及插件侧实际块数据适配开始。
