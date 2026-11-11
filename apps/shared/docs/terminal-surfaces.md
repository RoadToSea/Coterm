# 公共终端与字节工作台

共享组件是 `apps/shared/` 内部源码，直接随调用驱动打包，不注册独立 SDK 或运行时依赖。通过消费者的 Vite dedupe 和 TypeScript paths 解析既有 xterm 依赖。

| 入口 | 参数／返回 | 边界 |
|---|---|---|
| `components/terminal-surface::mountTerminalSurface` | 容器、工具栏、翻译函数、尺寸回调、复制失败回调 → terminal、searchView、resize()、dispose() | 主题、字体、搜索、快捷键和监听统一管理；尺寸回调只在容器可见时执行；dispose 幂等，非法 DOM 容器由 DOM／xterm 报错 |
| `components/stream-workbench::mountStreamWorkbench<C>` | 容器、绑定实例的 DriverAPI、驱动选项 → DriverFrontend | C 至少包含可选 mode/prompt；驱动提供配置表单、命名空间及实际输入差异；操作错误由有界提示展示，unmount 释放监听、队列后续动作和计时器 |
| `components/network-connection::configureNetwork` | 容器、DriverConfigurationAPI、Telnet 标记 → DriverFrontend | 配置由 submit 创建实例，不打开 socket；无效参数显示错误；提交期间阻止重复提交 |
| `components/network-connection::mountNetwork` | 容器、DriverAPI、Telnet 标记 → DriverFrontend | 绑定网络 connectionId、本地回显、状态补取；不提供 SSH exec／文件面板 |

`mountTerminalSurface` 只管理显示生命周期，不识别协议、推断命令或发起连接。调用方负责输入、输出、resize 协议请求和可见性。SSH 在该组件之上保留自己的命令块、人工观察和后台作业。

`mountStreamWorkbench` 复用串口已有的终端／字节视图、UTF-8／HEX 展示、接收换行、选区、回放缺口处理、字节计数、定时发送、导入导出、发送历史及本地偏好。配置接口只有 update、prepare、locale、setBusy 和可选 sync；串口适配器处理控制线，网络适配器处理主机、代理和终端参数。共享工作台不构造虚假的串口参数。

内部回放游标只用于显示补取，不作为 Agent 接口。网络写入绑定当前连接 ID，切换连接后丢弃旧输入；定时发送断线停止。网络每 2 秒串行补取状态，串口继续使用原事件与缺口回放策略。

Telnet 的密码由人工终端输入，不写入连接配置。任何本地回显仅显示成功提交的人工输入，不回灌后端接收流。Raw TCP 的字节调试发送与终端发送共用同一有界输入队列；关闭页面不销毁后端连接。

串口原有 `coterm.serial.display.v1:*` 和工具栏存储标识保持不变，保留旧换行偏好迁移。网络显示偏好使用各自命名空间，连接参数走现有完整配置保存；保存失败可重试，不在共享组件中另建持久化模型。

回归覆盖实际构建的驱动模块、主题与语言更新、模式切换、搜索与快捷键、选区、保存失败、断线状态和卸载释放。协议与容量见 [网络驱动](../../../docs/architecture/network-drivers.md) 和 [串口驱动](../../../docs/architecture/serial-driver.md)。
