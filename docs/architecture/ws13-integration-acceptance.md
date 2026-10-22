# WS-13 集成与回归验收记录

本文档记录 WS-13 对 WS-10（宿主契约与坏包隔离）与 WS-11（UI 视觉 token 与响应式窄窗布局）的集成收敛、缺口修复与全量回归验收结果。

---

## 1. 来源提交与真实完整 SHA

| 任务 / 来源 | 提交作者 | 分支 / 引用 | 真实完整 SHA | 说明 |
| :--- | :--- | :--- | :--- | :--- |
| **WS-10 交付** | deepseek | `origin/agent/deepseek/ws-10` | `75b3fcd66c18bcae8ca58d49523dbe7e047464a8` | PluginCatalog 逐包诊断、坏包隔离、宿主 attention 契约 |
| **WS-10 宿主修复** | deepseek | - | `8505474478f77348e3cfbb83cf4b163ce4802c5c` | 宿主交互状态与审批逻辑修复 |
| **WS-11 交付** | gemini | `origin/agent/gemini/ws-11` | `6fbb17c1aac2e4d34dee946f148504e88406eaeb` | 语义 UI token、768px 响应式折叠、Shadow DOM fallback |
| **WS-11 前序提交** | gemini | - | `9e97b1ba2eb2e86b24bb6b8206d213ee8c934271` | shared visual tokens 与 responsive layout 实现 |
| **共同祖先基线** | - | - | `757cbdd41f057b38046bf6e2a646d5cdbcb10c31` | 两任务分叉起点 |
| **平台原起点** | - | - | `7b0a94b6b96c6da7d541ca853a2998ba27a36a0c` | 平台 WS-13 初始起点 |
| **中间恢复点** | deepseek | `0ac643c2` | `0ac643c234afef0e99cbd65637fd2a20bc30b0ac` | WS-10/11 合并（3ec69c2）及测试 harness 变更 |
| **恢复合并点** | deepseek | `b174bcd3` | `b174bcd3aa0b8ea0e28f57e22ed50915a5a74e4f` | 恢复点 merge 到分配分支 |
| **WS-13 本轮 START** | RoadToSea | `agent/gemini/ws-13` | `9808d1266eb7b1a75ef2a5ee0569a2c06917582f` | 平台本次运行分配的起始 HEAD（必须保留为祖先） |

> **SHA 更正说明**：WS-11 最终交付报告中笔误书写了一个不存在的 SHA，已核验远端真实追踪提交为 `6fbb17c1aac2e4d34dee946f148504e88406eaeb`，无需重派改写。

---

## 2. 缺口定位与修复说明

### 2.1 Harness 键盘事件激活（CDP Input.dispatchKeyEvent）
- **问题**：在无头浏览器中，以 `for (const key of ["keyDown", "keyUp"])` 发送事件无法触发普通 `<button>` 的原生 click/activate 逻辑，导致 768px 键盘展开断言超时。
- **修复**：按 CDP 规范补齐完整的 Enter 键事件三部曲（`rawKeyDown` + `char` + `keyUp`，并携带 `\r` 字符序列）。真实按钮原生激活，`expanded=true` 状态顺利切换，无任何 mock 冒充。

### 2.2 Tauri mock IPC 事件注册契约
- **问题**：
  1. `@tauri-apps/api/event` 的 `listen` 首先调用 `transformCallback` 获取一个数字 ID，再调用 `plugin:event|listen` 传入该 ID。原 mock 在 `plugin:event|listen` 里再次分配自增 ID，导致 `listeners` 中存入数字而非回调函数，`emitAttention` 触发时抛出 `TypeError: callback is not a function`。
  2. `setAttention` 内部直接调用未挂载的全局 `emitAttention`，导致 `ReferenceError: emitAttention is not defined`。
- **修复**：
  1. `plugin:event|listen` 直接将传入的 callback ID 存入对应事件集合并返回，与 `@tauri-apps/api` 契约严格对齐，支持正常取消注册与准确计数。
  2. `setAttention` 修正为通过 `window.__cotermTest.emitAttention` 调用。

### 2.3 宿主状态订阅多路复用（subscribeHostAttention）
- **问题**：`HostApprovals` 与 `HostInteractions` 分别调用 `subscribeHostAttention`，导致每次宿主事件到达时同时触发两条独立的 `host_attention` IPC 请求；当模拟注入单次失败时，其中一条请求消费了失败而另一条成功，破坏了全局状态一致性并导致状态错误漏报。
- **修复**：在 `apps/desktop/src/host-api.ts` 中重构为单例多路复用（Multiplexed Shared State）模式：
  - 多个订阅者共享单一底层 Tauri 事件监听与快照请求；
  - 自动丢弃乱序/旧版本（`version <= current`）；
  - 在途拉取时到达的高版本缓冲待完成后自动补取；
  - 快照异常时广播至所有活跃订阅者；快照恢复时自动清除错误；
  - 最后一个订阅者注销时彻底清理事件监听与内部状态。

### 2.4 审批提示与逐包诊断
- **审批错误前缀**：`HostApprovals.tsx` 补充规范的 `Unable to follow Agent approvals: ` 前缀。
- **逐包诊断展示**：`App.tsx` 坏包条目显示优化为 `issue.pluginId ? `${issue.pluginId} (${issue.directory})` : issue.directory`，同时显示插件标识与文件目录，精准定位坏包。

---

## 3. 验收场景与实测证据

### 3.1 响应式布局实测数据（无重叠与横向溢出）
通过 CDP 测量当前构建（`apps/desktop/dist`）的实际渲染指标：

| 视口宽度 | 插件侧栏 (w) | 实例侧栏 (w / left / right) | 工作区 (w / left) | scrollWidth / clientWidth | 状态与断言 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **1280px** | 72px | 240px / 72px / 312px | 968px / 312px | 1280px / 1280px | 展开，无重叠，无横向溢出 |
| **1024px** | 72px | 200px / 72px / 272px | 752px / 272px | 1024px / 1024px | 实例栏窄化为 200px，无横向溢出 |
| **768px (默认折叠)** | 72px | 0px (隐藏) | 696px / 72px | 768px / 768px | `<900px` 自动折叠，工作区优先 |
| **768px (键盘展开)** | 72px | 200px / 72px / 272px | 496px / 272px | 768px / 768px | Enter 键激活展开，无溢出与重叠 |

- 交付截图见附件：
  - `layout-1280px.png`
  - `layout-1024px.png`
  - `layout-768px-collapsed.png`
  - `layout-768px-expanded.png`

### 3.2 诊断与 Shadow DOM 样式隔离
- **有效与损坏插件并存**：SSH 插件正常显示并可操作；损坏包（`broken-plugin`）与旧格式包（`legacy-plugin`）在 `ul[aria-label="Plugin package issues"]` 中以 `role="status"` 诊断卡片清晰展示。
- **Shadow DOM Token 继承**：继承 `--coterm-text-primary`（`rgb(204, 204, 204)`）与 `--coterm-surface`（`rgb(30, 30, 30)`）；缺失 token 准确回退至声明 fallback（`rgb(250, 204, 21)`）。

### 3.3 状态管理与契约用例
- **初始化与并发版本更新**：验证在途初始快照期间新事件到达时的排队重拉；
- **乱序与重复事件**：旧版本号（<= current）与重复通知完全静默丢弃，无冗余 IPC；
- **错误恢复**：快照失败时屏幕保留上次有效审批卡片并显示告警；下一次快照成功告警自动消失并更新状态；
- **生命周期注销**：各订阅者独立注销，归零后底层 listener 彻底 unlisten；
- **独立审批**：实例级（`inst-1`）与插件级（`ssh` 无实例）操作分别路由对应后端命令，独立响应与审批；
- **交互收敛与隔离**：宿主交互取消/过期/实例销毁时弹窗立即收敛，不会向非对应实例泄漏或发送凭据。

---

## 4. 测试套件与构建运行计数

| 测试套件 / 命令 | 耗时 | 测试数 | 结果 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| `cargo check --workspace` | ~7s | 全部 crate | 0 errors | **Pass** |
| `cargo test --workspace` | ~2.5s | 44 tests | 44 pass / 0 fail | **Pass** |
| `cargo clippy --workspace -- -D warnings` | ~8s | 全部 crate | 0 warnings | **Pass** |
| `pnpm build` (`coterm-desktop`) | ~4.5s | 前端生产构建 | 0 errors | **Pass** |
| `pnpm --filter coterm-plugin-ssh-ui build` | ~1.2s | SSH 插件前端 | 0 errors | **Pass** |
| `pnpm test` (`node --test tests/*.test.mjs`) | ~3.8s | 12 tests | 12 pass / 0 fail | **Pass** |

---

## 5. 剩余问题与后续建议

1. **当前状态**：WS-10 与 WS-11 的功能点、契约与 UI 全面收敛并通过自动化测试，所有验收指标均达成，无未解决阻塞项。
2. **后续建议**：WS-8 父任务后续若引入串口、MQTT 等新型独立插件，建议直接复用本次完善的 `PluginCatalog` 逐包隔离与 `subscribeHostAttention` 多路复用机制。
