# Coterm 文档

Coterm 面向人类与 Agent 协同操作终端和连接设备，支持 SSH、串口、本地终端、Telnet 与 Raw TCP。文档按内容性质分为四个目录。

| 目录 | 内容 | 入口 |
|---|---|---|
| requirements/ — 需求 | 模块划分、详细需求、公共需求、职责和信息交互；不规定代码实现 | [模块架构](requirements/modules.md) |
| implementation/ — 技术实现 | 整体代码关系、公共复用、模块内部方案、内部契约、开发验收与实现状态 | [代码架构](implementation/architecture.md) |
| feedback/ — 反馈 | 未解决问题、现场现象、复现条件、失败证据和继续调查入口 | [未解决反馈](feedback/known-issues.md) |
| api/ — 对外 API | 外部客户端接入、公开工具、参数、返回值、错误与调用限制 | [MCP API](api/mcp.md) |

## 需求阅读顺序

1. [模块架构](requirements/modules.md)：项目分成哪些模块，各自的业务边界是什么。
2. 模块详细需求：[工作区与配置](requirements/workspace.md)、[实例运行](requirements/runtime.md)、[审批与交互](requirements/approval.md)、[通信驱动](requirements/drivers.md)、[数据记录与读取](requirements/data.md)、[桌面交互](requirements/desktop.md)、[Agent 接入](requirements/agent.md)。
3. [公共需求](requirements/common.md)：跨模块共同约束。
4. [职责与信息交互](requirements/interactions.md)：数据归谁管理，交接什么，失败怎样收敛。
5. [代码架构](implementation/architecture.md)：根据整体关系、公共需求、模块内部需求安排实现。

前四步定义业务模块、需求和信息语义，不规定语言、类、目录或消息编码。第五步位于技术实现目录，区分现状与后续建议。逻辑模块不等于独立软件包，也不要求增加转发层。

## 技术实现与验收

- [内部契约](implementation/contracts.md)：宿主、驱动、前端的控制、配置和容量限制。
- [开发与验收](implementation/development.md)：构建、测试、硬件测速、发行与调试清理。
- [实现与验收状态](implementation/verification.md)：已有能力、已通过的检查及实际覆盖范围。

## 维护规则

- 每项稳定要求在所属需求文件定义，公共要求集中维护；实现细节不倒灌需求。
- 内部控制和配置格式放技术实现，外部调用方需要知道的契约放对外 API；不因内部存在操作就将其当作公开工具。
- 未解决现场证据放 feedback，已经实现和验证的能力放技术实现；已解决反馈归入相应需求或实现说明，开发历史交给 Git。
- 长文档集中在 docs/；源码旁只保留必要接口注释和示例，不另建成套需求、架构、用法和进度文档。
- 新增依赖、修改持久化结构或共享基础接口的实现变更仍需单独确认；文档归档不授权改变这些实现。
