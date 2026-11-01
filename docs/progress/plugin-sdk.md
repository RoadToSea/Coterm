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

## 限制与后续入口

- 本次交付插件可复用组件及示例；不在基座猜测 SSH、串口或本地终端的命令边界，不自动将已有终端改成块视图。
- 每种插件需从自己的协议 / 命令记录构造有界 blocks 快照，并处理截断、缺口、秘密脱敏与选中上下文用途。
- 该组件展示纯文本，不是 xterm 替代品；已有终端仿真、认证与审批流程保持各自职责。
- 继续开发从 `packages/plugin-sdk/src/components/block-list.ts`、对应浏览器测试及插件侧实际块数据适配开始。
