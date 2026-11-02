# 插件公共 UI

SDK 提供无框架依赖的可选组件，由插件前端直接导入并随插件打包。插件后端生成命令与回复的块数据，通过基座的操作结果或实例事件交给插件前端；前端引用组件负责展示与选择。

| 组件 | 入口 | 说明 |
|---|---|---|
| [命令消息块与选择](command-blocks.md) | `@coterm/plugin-sdk/components/block-list` → `mountBlockList(container, props)` | 后端提供块数据；前端显示命令及回复、左侧块号选择、高亮与边框 |
| [xterm 命令行号栏](command-blocks.md#终端行号栏) | `@coterm/plugin-sdk/components/terminal-blocks` → `mountTerminalBlocks(gutter, overlay, terminal, options)` | 终端插件提供可信命令边界，SDK 在现有 xterm 上显示块号及选择高亮 |
| [非模态提示](toasts.md) | `@coterm/plugin-sdk/components/toasts` → `mountToasts(container, options)` | 圆角提示；错误红色、警告黄色，自动消失、不抢焦点；支持关闭、去重和有界展示 |

通用挂载、实例绑定与卸载契约见 [插件开发](../../../docs/architecture/plugin-development.md)。
