# 驱动公共 UI

共享前端模块提供无框架依赖的可选组件，由驱动前端直接导入并随驱动打包。驱动后端生成命令与回复的块数据，通过基座的操作结果或实例事件交给驱动前端；前端引用组件负责展示与选择。

| 组件 | 入口 | 说明 |
|---|---|---|
| [公共终端与字节工作台](terminal-surfaces.md) | `terminal-surface`、`stream-workbench`、`network-connection` | 共享 xterm 生命周期、终端／字节交互和网络表单；驱动保留协议参数与能力 |
| [命令消息块与选择](command-blocks.md) | `apps/shared/components/block-list` → `mountBlockList(container, props)` | 后端提供块数据；前端显示命令及回复、左侧块号选择、高亮与边框 |
| [xterm 命令行号栏](command-blocks.md#终端行号栏) | `apps/shared/components/terminal-blocks` → `mountTerminalBlocks(gutter, overlay, terminal, options)` | 终端驱动提供可信命令边界，共享前端模块在现有 xterm 上显示块号及选择高亮 |
| [非模态提示](toasts.md) | `apps/shared/components/toasts` → `mountToasts(container, options)` | 圆角提示；错误红色、警告黄色，自动消失、不抢焦点；支持关闭、去重和有界展示 |

通用挂载、实例绑定与卸载契约见 [驱动开发](../../../docs/architecture/driver-development.md)。
