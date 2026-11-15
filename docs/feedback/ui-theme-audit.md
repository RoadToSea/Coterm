# 前端主题与工作台的待验收范围

组件、圆角、主题及工作台约束见[前端实现](../implementation/frontend.md)，配置元数据见[内部契约](../implementation/contracts.md)，软件验收见[实现与验收状态](../implementation/verification.md)。本文件只保留尚未完成的原生验证范围。

| 待验收范围 | 当前证据边界 | 验收要求 |
|---|---|---|
| Windows 原生 WebView2 的完整外观矩阵与安装包 | 隔离 smoke 构建已验证 Local、TCP、Telnet、MCP HTTP 审批、保存恢复、HTTP 换端口／启停及关窗释放连接与端口；尚未覆盖完整外观矩阵或安装包 | 补验五种驱动配置、切换标签、主题／字号与认证；检查安装包资源和安装后关闭清理 |
| macOS／Linux 原生界面 | 未在对应系统运行 | 检查原生选择器、dialog 焦点与 Escape、系统主题跟随、终端字体和窄窗口布局 |
| 真实设备与远端 | 本次 UI 验收不覆盖真实串口、远端 SSH、Telnet 设备或代理 | 按[开发验收](../implementation/development.md)的已授权现场流程复测，不以模拟状态代替实际连接成功 |

原生验收前不宣称上述范围已经通过。后端协议、认证、审批规则与保存格式仍按原契约执行。
