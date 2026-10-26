# Coterm 分阶段详细技术方案

本目录存放按阶段拆分的详细技术方案。每个阶段独立描述接口、状态机、数据流、异常处理、测试策略和验收标准，并遵守 [`docs/SPEC.md`](../SPEC.md) 及 [`docs/architecture/`](../architecture/) 中定义的全局架构边界。

当前已实现基础闭环，第一阶段仍需补齐实机与异常路径验收。代码核对、优先缺口和验证结果见[项目进度评估](../PROGRESS.md)。第一阶段入口已列实际交付与验收矩阵，后续专题方案仍需细化。

## 阶段目录

- [`phase-01-foundation/`](phase-01-foundation/)：基础闭环，已实现主要路径、待完成验收
- `phase-02-multi-protocol/`：多协议与插件运行时，后续建立
- `phase-03-ecosystem/`：插件生态与高级工作台，后续建立
- `phase-04-industrial/`：工业协议与多 Agent 策略，后续建立
