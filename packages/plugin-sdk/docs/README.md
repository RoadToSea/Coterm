# 插件公共 UI

SDK 提供无框架依赖的可选组件，由插件前端直接导入并随插件打包。插件后端生成命令与回复的块数据，通过基座的操作结果或实例事件交给插件前端；前端引用组件负责展示与选择。

| 组件 | 入口 | 说明 |
|---|---|---|
| [命令消息块与选择](command-blocks.md) | `@coterm/plugin-sdk/components/block-list` → `mountBlockList(container, props)` | 后端提供块数据；前端显示命令及回复、左侧块号选择、高亮与边框 |

通用挂载、实例绑定与卸载契约见 [插件开发](../../../docs/architecture/plugin-development.md)。
