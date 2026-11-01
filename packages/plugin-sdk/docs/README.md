# 插件公共 UI

SDK 提供无框架依赖的可选组件。插件可以通过宿主 `api.components` 使用，也可以直接导入组件模块随插件打包；数据与协议语义仍由插件负责。

| 组件 | 入口 | 说明 |
|---|---|---|
| [命令消息块与选择](command-blocks.md) | `api.components.blocks(container, props)` | 左侧块号选择，右侧命令及回复组成有边框的消息块 |

通用挂载、实例绑定与卸载契约见 [插件开发](../../../docs/architecture/plugin-development.md)。
