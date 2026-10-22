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
- **问题**：在无头浏览器中，以 `for (const key of ["keyDown", "keyUp"])` 发送事件无法触发普通 `<button>` 的原生 click/activate 逻辑，导致 768px 键盘展开断言超时；原有再折叠用例使用了 `.click()` 冒充键盘行为。
- **修复**：按 CDP 规范补齐完整的 Enter 键事件三部曲（`rawKeyDown` + `char` + `keyUp`，并携带 `\r` 字符序列）。展开与再折叠均通过焦点定位与原生按键事件激活，彻底移除 `.click()` 冒充。

### 2.2 Tauri mock IPC 事件注册契约
- **问题**：
  1. `@tauri-apps/api/event` 的 `listen` 首先调用 `transformCallback` 获取一个数字 ID，再调用 `plugin:event|listen` 传入该 ID。原 mock 在 `plugin:event|listen` 里再次分配自增 ID，导致 `listeners` 中存入数字而非回调函数，`emitAttention` 触发时抛出 `TypeError: callback is not a function`。
  2. `setAttention` 内部直接调用未挂载的全局 `emitAttention`，导致 `ReferenceError: emitAttention is not defined`。
- **修复**：
  1. `plugin:event|listen` 直接将传入的 callback ID 存入对应事件集合并返回，与 `@tauri-apps/api` 契约严格对齐，支持正常取消注册与准确计数。
  2. `setAttention` 修正为通过 `window.__cotermTest.emitAttention` 调用，且 `emitAttention` 保持与 `attention.version` 严格同步。

### 2.3 宿主状态订阅多路复用与并发健壮性（subscribeHostAttention）
- **问题**：`HostApprovals` 与 `HostInteractions` 分别调用 `subscribeHostAttention`。原有逻辑在并发注册、提早注销、快照在途时存在竞态；此外，快照拉取异常时若未清空 `buffered`，会导致后续相同版本的重新通知被去重死锁，同时若在在途快照未完成前所有订阅者注销，迟到的旧快照会污染新生命周期。
- **修复**：在 `apps/desktop/src/host-api.ts` 中以单例多路复用（Multiplexed Shared State）模式实现完整防护：
  1. **共享 `listenPromise` 与提早中止回滚**：首个订阅者创建底层 Tauri 事件监听，若在 listen 完成前订阅者注销或抛出异常，立即触发 unlisten 回滚，避免孤立监听；
  2. **生命周期代次计数器（Generation Counter）**：每次所有订阅者归零清空状态时递增 `generation`。所有在途快照返回、事件回调与错误广播必须核验 `gen === sharedAttention.generation`，彻底杜绝迟到响应污染新代次；
  3. **订阅者归零时废弃在途快照**：当 `subscribers.size === 0` 时，在途 `host_attention` 快照返回直接丢弃，不更新全局缓存或唤醒已注销处理函数；
  4. **失败恢复与去重死锁防范**：快照拉取失败时清空 `buffered` 版本缓存，使得外部下一次重新通知同一版本号时能正常触发拉取，同时完整保留 `lastSnapshot` 供 UI 界面维持当前展示，不闪烁、不假收敛。

### 2.4 审批提示与逐包诊断
- **审批错误前缀**：`HostApprovals.tsx` 补充规范的 `Unable to follow Agent approvals: ` 前缀。
- **逐包诊断展示**：`App.tsx` 坏包条目显示优化为 `issue.pluginId ? `${issue.pluginId} (${issue.directory})` : issue.directory`，同时显示插件标识与文件目录，精准定位坏包。

### 2.5 真实 SSH 前端 Shadow DOM 与 SDK Block-list 挂载
- **挂载与样式穿透**：通过真实编译构建产物 `plugins/plugin-ssh/dist/frontend/index.js`，在工作区中直接挂载 ShadowRoot 并验证 `.layout`、`.bar`、`.terminal` 真实节点；使用 Plugin SDK `mountBlockList` 在真实页面中渲染并验证其在真实 token 主题下的继承与 fallback 计算样式。

---

## 3. 证明范围与实测证据

### 3.1 证明范围明确划分（Proof Scope Statement）
- **Rust 后端协议核心（Core Protocol & Security Boundaries）**：
  由 `cargo test --workspace`（44 个单元与集成测试）提供不可替代的安全证明：
  - 插件进程生命周期隔离（崩溃检测与重启）；
  - Stdio JSON-RPC 帧编解码与传输契约；
  - SSH 协议底层网络与会话状态机；
  - 未批准 Proposal 绝不下发底层协议执行；
  - Secret 凭据与 `pluginInstanceId` 严格隔离，严禁明文持久化或跨实例泄露。
- **前端状态契约与 UI 交互（Frontend State Contracts & UI Convergence）**：
  由 `tests/*.test.mjs`（17 个 Node/CDP 自动化测试）提供前端与用户界面证明：
  - `subscribeHostAttention` 在单线程事件循环下的代次管理、并发初始化互斥、失败重试与版本防锁死；
  - 多审批卡片在屏幕上同时呈现、实例级与插件级操作独立审批/拒绝、决定后剩余卡片保留直至最终收敛；
  - 凭据对话框在生命周期内的正常提交、用户/宿主取消、超时过期清理以及实例销毁时收敛，杜绝迟到输入提交；
  - 语义 CSS token 在 `:root`、Tailwind、插件 Shadow DOM 以及 SDK 外部容器中的层叠继承与 fallback 计算样式；
  - 窄窗 768px 断点自适应折叠与真实 CDP 键盘 Enter/Escape 双向折叠展开。

### 3.2 响应式布局实测数据（无重叠与横向溢出）
通过 CDP 测量当前构建（`apps/desktop/dist`）的实际渲染指标：

| 视口宽度 | 插件侧栏 (w) | 实例侧栏 (w / left / right) | 工作区 (w / left) | scrollWidth / clientWidth | 状态与断言 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **1280px** | 72px | 240px / 72px / 312px | 968px / 312px | 1280px / 1280px | 展开，无重叠，无横向溢出 |
| **1024px** | 72px | 200px / 72px / 272px | 752px / 272px | 1024px / 1024px | 实例栏窄化为 200px，无横向溢出 |
| **768px (默认折叠)** | 72px | 0px (隐藏) | 696px / 72px | 768px / 768px | `<900px` 自动折叠，工作区优先 |
| **768px (键盘展开)** | 72px | 200px / 72px / 272px | 496px / 272px | 768px / 768px | 纯 Enter 键激活展开，无溢出与重叠 |
| **768px (键盘再折叠)** | 72px | 0px (隐藏) | 696px / 72px | 768px / 768px | 纯 Enter 键再次折叠，无溢出与重叠 |

- 交付截图见附件（工作区非空，包含真实 SSH 终端布局）：
  - `attachments/layout-1280px.png`
  - `attachments/layout-1024px.png`
  - `attachments/layout-768px-collapsed.png`
  - `attachments/layout-768px-expanded.png`

### 3.3 计算样式与 Shadow DOM 继承实测
- **真实 SSH 前端 Shadow DOM 计算样式**：
  - `:host` 颜色：`rgb(204, 204, 204)`（继承 `--coterm-text-primary`）
  - `.layout` 背景：`rgb(30, 30, 30)`（继承 `--coterm-surface`）
  - `.bar` 背景：`rgb(37, 37, 38)`（继承 `--coterm-surface-raised`）
  - `.bar strong` 颜色：`rgb(204, 204, 204)`（继承 `--coterm-text-primary`）
- **SDK Block-list 组件计算样式**：
  - 容器文字颜色：`rgb(204, 204, 204)`（继承 `--coterm-text-primary`）
  - 容器背景：`rgb(30, 30, 30)`（继承 `--coterm-surface`）
  - Robot Actor 标签：`rgb(117, 190, 255)`（继承 `--coterm-accent-glow`）
  - 错误状态标签：`rgb(244, 135, 113)`（继承 `--coterm-danger`）
  - 缺失 token fallback 计算值：`rgb(250, 204, 21)`（准确回退至声明 fallback）

---

## 4. 测试套件与构建运行计数

| 测试套件 / 命令 | 耗时 | 测试数 | 结果 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| `cargo check --workspace` | ~7s | 全部 crate | 0 errors | **Pass** |
| `cargo test --workspace` | ~2.5s | 44 tests | 44 pass / 0 fail | **Pass** |
| `cargo clippy --workspace -- -D warnings` | ~8s | 全部 crate | 0 warnings | **Pass** |
| `pnpm build` (`coterm-desktop`) | ~3.2s | 前端生产构建 | 0 errors | **Pass** |
| `pnpm --filter coterm-plugin-ssh-ui build` | ~1.2s | SSH 插件前端 | 0 errors | **Pass** |
| `pnpm test` (`node --test tests/*.test.mjs`) | ~3.5s | 17 tests | 17 pass / 0 fail | **Pass** |

- **Node 测试用例细目（17 pass）**：
  - `tests/browser-layout.test.mjs` (3 tests):
    1. Browser layout, catalog diagnostics, and keyboard collapse on the current build
    2. Snapshot failure keeps the last valid attention state on screen
    3. Multi-card independent approvals and UI convergence in the browser
  - `tests/host-attention-state.test.mjs` (8 tests):
    4. subscribeHostAttention: concurrent registration creates only one listen and unlistens cleanly (order 1)
    5. subscribeHostAttention: different unlisten order (order 2: stop2 then stop1)
    6. subscribeHostAttention: listen failure rolls back subscription and allows retry
    7. subscribeHostAttention: late in-flight snapshot does not pollute new generation or resurrect cache
    8. subscribeHostAttention: in-flight snapshot catches up on events arrived during request
    9. subscribeHostAttention: snapshot failure keeps last valid state and recovers on re-notification
    10. Approvals UI behavior: multiple cards displayed simultaneously, independent approve/reject, and convergence
    11. Interactions UI behavior: secret dialog lifecycle, cancellation, expiry, and instance isolation
  - `tests/visual-tokens-layout.test.mjs` (6 tests):
    12. Desktop CSS defines required semantic tokens in :root
    13. Tailwind configuration maps coterm semantic tokens to CSS custom properties
    14. SSH Plugin Shadow DOM styles use semantic tokens with readable fallbacks
    15. Plugin SDK block-list component styles use semantic tokens with fallbacks
    16. App.tsx implements responsive navigation collapse and accessible controls
    17. Layout width simulation at 1280px, 1024px, and 768px

---

## 5. 结论

WS-10 与 WS-11 的功能点、契约与 UI 全面收敛并通过全量自动化测试。所有返工评审要求（多路复用代次管理、未决快照隔离、清空 `buffered` 防死锁、多卡片独立审批收敛、凭据隔离、真实 SSH Shadow DOM 挂载、纯键盘无 `.click()` 折叠以及精准 SHA 说明）均已严格闭环，随时可供合并交付。
