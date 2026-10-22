# WS-13 集成与回归验收记录

本文档记录 WS-13 对 WS-10（宿主契约与坏包隔离）与 WS-11（UI 视觉 token 与响应式窄窗布局）的集成收敛、缺口修复与回归验收结果。

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
| **第三轮审阅基线** | - | `30e717f` | `30e717fc44f9e9c7115049fd1207b97b0b748cd5` | 上一轮交付，本轮在此基础上返工 |
| **WS-13 本轮 START** | - | `agent/deepseek/ws-13` | `9808d1266eb7b1a75ef2a5ee0569a2c06917582f` | 平台本次运行分配的起始 HEAD（必须保留为祖先） |
| **平台注入段清理** | - | `fe019e8` | `fe019e88d07c26a249d9753a2881d326b205b812` | 仅删除注入的 AGENTS 段，本轮已并入交付分支，不当作产品改动 |

> **SHA 更正说明**：WS-11 最终交付报告中笔误书写了一个不存在的 SHA，已核验远端真实追踪提交为 `6fbb17c1aac2e4d34dee946f148504e88406eaeb`，无需重派改写。

---

## 2. 本轮返工项

### 2.1 P1：真实 HostInteractions 的旧 finally 干扰新交互

- **缺陷**：`apps/desktop/src/components/host/HostInteractions.tsx` 里，`observe` 发现 A 已消失且 B 待处理时会 `cancelSecretInput(A)`、清空 `active`、再弹 B；随后 A 的 Promise `finally` 无条件把 `active` 置空并补取状态，于是又弹一次 B。而 `promptForSecret` 按同一个 key 取消同名弹窗，重复 B 会取消第一次 B，形成收敛重入。
- **复现**：新用例 `tests/host-interactions-component.test.mjs` 挂载真实组件，依次观察 `[A]`、`[A消失,B]`。对旧实现，收敛这一步本身就会把渲染进程拖死（`Runtime.evaluate` 20s 超时），断言以 `page must stay responsive across A departing and B arriving` 报出。
- **修复**（接口不变）：
  1. 每次弹窗分配递增的 `ticket`，`active` 记录自己的 `ticket`；
  2. `then` / `catch` / `finally` 一律先核对 `active?.ticket === mine`，旧完成回调只能清理自己；
  3. 已结算的请求记入 `settled`，宿主状态还没跟上时不再重开同一项，宿主不再列出后自动清除；
  4. 补取的 `hostAttention()` 返回后同样核对 `disposed` 与 `active`，卸载后不弹窗；
  5. 取消/过期后队列继续推进下一条，没有改回轮询，也没有丢弃新请求。

### 2.1b P1：补取快照仍能重开已取消的请求（本轮新修）

上一轮只给「已分配的弹窗」加了 `ticket`，`finally` 里的补取 `hostAttention()` 本身没有身份。补取出发后，共享订阅可能先交付一份更旧的快照，把 A 从 `settled` 里清掉，并且把 `active` 置空；补取随后带着旧版本 `[A]` 回来时两个判断（`!disposed`、`!active`）都成立，A 就被重新打开了。

- **复现**（`tests/host-interactions-component.test.mjs`，全部用可控闸门排定顺序，不靠 sleep）：
  1. `v1=[A]` 打开 A；用户取消 A，闸门挂住 `finally` 发出的补取；
  2. 较新的空快照 `v2=[]` 到达，共享订阅收敛，`settled` 里的 A 被清掉；
  3. 放行那次早先的补取（返回旧 `v1=[A]`）——修复前实际重新出现 `inst-a` 的密码对话框。
- **修复**（接口与依赖均未变）：
  1. `observe` 维护单调递增的 `observedVersion`，更旧或重复的快照直接丢弃，不倒退状态；
  2. 每次分配弹窗递增 `epoch`；补取出发时记录 `epoch` 与 `observedVersion`，回来时两者任一已变即为过期，直接丢弃；
  3. 补取的 `catch` 用同一组判断，旧失败不再覆盖更新的状态或错误提示；
  4. 仍有 `disposed` 保护，卸载后补取与失败回调都不动界面。
- **证明边界**：这是请求顺序缺陷，已证明的是「旧补取不再重开已取消的请求」；不声称曾发生过秘密泄露或跨实例提交。

### 2.2 P1：用真实组件和可控 IPC 补足交互测试

新增 `tests/host-interactions-component.test.mjs`（9 个用例，真实浏览器）：

| 用例 | 覆盖 |
| :--- | :--- |
| a departed request's late finally… | A 消失、B 到达；A 的迟到 finally 不得重开 A 或清空 B；`host_attention` 不得被额外补取 |
| late IPC completion… | `respond_secret` 用可控闸门挂起；旧投递完成后不得清空新 active、不得重开旧项 |
| cancel, expiry, instance destroy, plugin stop and unmount | 用户取消、宿主过期、实例销毁、插件停止、组件卸载；卸载后监听器归零、不再弹窗 |
| duplicate and out-of-order… | 重复版本、更旧版本、内容不变的更新版本都不得二次弹窗；已结算请求不被重复版本翻出 |
| a stale prefetch cannot reopen… | 取消 A 后较新空快照先收敛，旧补取返回 `[A]`：零对话框、只有取消那一条 `respond_secret` |
| a stale prefetch cannot displace… | 较新快照含 B 时旧补取不影响 B；B 提交归零后 A 仍不回来 |
| cancel, newer B, B cancelled… | B 也取消、队列再归零后，旧 A 补取仍不重开 |
| a prefetch or a failure landing after unmount… | 补取在卸载后才返回：不弹窗、不写错误 |
| an older prefetch failure… | 旧补取失败不覆盖较新状态，B 的弹窗不受影响 |

实现要点：
- 页面只挂 `HostInteractions`（`tests/support/host-interactions-entry.tsx` → Vite 打成 `apps/desktop/dist/host-interactions/entry.js`），其余全部走真实路径（`host-api`、`secret-input`、React、`@tauri-apps/api`）。不挂真实 `App` 是因为它同时挂 `HostApprovals`，两份订阅记录会互相干扰，反而看不出是谁开的弹窗。
- 宿主替身新增 `gate()` / `release()` / `clearGates()` 与 `invocations` 记录，用可控 Promise 推进时序；不用固定 sleep 模拟竞态。
- 补取回归用例按**闸门队列下标**放行（index 0 是 `finally` 补取、index 1 是共享订阅为较新快照发的请求），不靠时间猜测先后；放行后先排空微任务再断言。
- `respond_secret` 的 `interactionId` 与 `value` 逐条核对，取消后迟到 submit 不提交秘密，也不串到其他实例。
- 对照证据：把三处保护（版本判断、`epoch` 判断、补取的 `catch` 判断）逐一移除后，分别有 1 个用例转红（第 5、7、9 个），说明这些断言确实盯着本缺陷而不是巧合通过。

### 2.3 P2：移除测试替身冒充真实 SDK 的覆盖声明

- **删除**：`tests/support/host-mock.js` 里手写的 `__cotermTestMountBlockList`（自写 CSS + `div` 循环）已整体移除，`tests/` 下不再有任何引用。
- **改为真实组件**：`scripts/build-sdk-browser.mjs` 用 Node 自带类型剥离，把 `packages/plugin-sdk/src/components/block-list.ts` 生成 `packages/plugin-sdk/dist/browser/block-list.js`，页面通过 `import("/__sdk/block-list.js")` 加载同一份源文件。没有复制实现，也没有打包替换。
- **新增断言**（`tests/browser-layout.test.mjs`）：
  - 行是 `BUTTON` 而非 `div`；根节点 `role="list"`、`aria-label="Command blocks"`；
  - token 继承：文字 `rgb(204, 204, 204)`、背景 `rgb(30, 30, 30)`、robot 标签 `rgb(117, 190, 255)`；
  - 真实渲染内容：actor 显示 `Robot`/`Human` 而非枚举值，非零退出显示 `!` 且 `title="Exit code 1"`，零退出无标记；
  - 选择状态：点击后仅该行 `aria-pressed="true"`，回调收到 `["b2"]`；
  - `update()` 重建行并按传入的 `selected` 重置选择；
  - 销毁：`unmount()` 清空容器，之后 `update()` 不再渲染。
- **fallback 改为真实回退**：不再给额外 span 设置不存在的变量。真实组件被挂进同源空白 iframe（文档内确实没有 `--coterm-text-primary`，先断言为空），读组件自身 CSS 的 fallback 计算值。SDK 与 SSH 前端各测一遍，分别断言：

| 目标 | 断言值 | 来源 |
| :--- | :--- | :--- |
| SDK `.coterm-blocks` 文字 | `rgb(204, 204, 204)` | 组件自身 `#cccccc` |
| SDK `.coterm-blocks` 背景 | `rgb(30, 30, 30)` | 组件自身 `#1e1e1e` |
| SDK robot 标签 | `rgb(117, 190, 255)` | 组件自身 `#75beff` |
| SDK 行号槽 | `rgb(133, 133, 133)` | 组件自身 `#858585` |
| SDK 失败标记 | `rgb(244, 135, 113)` | 组件自身 `#f48771` |
| SSH `:host` | `rgb(204, 204, 204)` | 前端自身 `#cccccc` |
| SSH `.layout` | `rgb(30, 30, 30)` | 前端自身 `#1e1e1e` |
| SSH `.bar` | `rgb(37, 37, 38)` | 前端自身 `#252526` |

- **多审批断言补强**：不再只数卡片。按 `targetLabel` 定位卡片，断言批准/拒绝打到真实的 command 名与参数：`approve_proposal`（`inst-1` / `prop-1` / `{command:"systemctl restart nginx"}`）、`reject_plugin_proposal`（`ssh` / `prop-3`）、最后一次 `approve_proposal`（`inst-2` / `prop-2` / `{command:"journalctl -u nginx"}`）；每一步都核对保留下来的是未决的那两张/那一张。

### 2.4 测试命名与证明范围更正

- 原 `Approvals UI behavior: …` 实际是手写 `map`/`filter`，改名为 `hostAPI routes instance-level and plugin-level approvals to distinct commands`，只声明 hostAPI 路由；卡片渲染与身份断言移到浏览器用例。
- 原 `Interactions UI behavior: …` 只调用 `promptForSecret`/`cancelSecretInput`，且用 `timeoutMs=10` 冒充过期（真实实现下限 1000ms）。现改名为 `promptForSecret: resolves submitted once, ignores later submits, and binds the cancellation key`，只测 `secret-input` 自身；过期场景改由浏览器用例走宿主收敛路径验证。**本轮没有任何用例声称用 10ms 超时验证过期。**
- 订阅代次用例里原来的 `await new Promise((r) => setTimeout(r, 20))` 改为等待真实调用发生的可控 Promise。

---

## 3. 证明范围与实测证据

### 3.1 证明范围划分

- **Rust 协议核心与基座安全边界**：`cargo test --workspace`（44 个）。覆盖插件进程生命周期、Stdio JSON-RPC 帧、SSH 会话状态机、未批准 Proposal 不下发、Secret 与 `pluginInstanceId` 绑定。
- **host-api 订阅状态机**（`tests/host-attention-state.test.mjs`，8 个，无 DOM）：并发注册只建一个监听、两种注销顺序、listen 失败回滚、迟到快照不污染新代次、在途事件补取、失败恢复与版本不锁死、审批路由命令。
- **`secret-input` 自身行为**（同文件 1 个）：单次投递、结算后不再投递、`cancellationKey` 生效。
- **真实组件交互生命周期**（`tests/host-interactions-component.test.mjs`，9 个，真实浏览器 + 真实构建产物 + 可控 IPC）：见 2.2。其中 5 个是补取顺序的回归用例。
- **真实构建产物的布局与组件**（`tests/browser-layout.test.mjs`，3 个，真实浏览器）：布局断点、逐包诊断、键盘折叠、真实 SSH Shadow DOM、真实 SDK `mountBlockList`、真实 fallback、多卡片审批。
- **不覆盖**：真实 SSH 连接、真实 Tauri 运行时、硬件/串口。IPC 层是契约替身，命令名与参数按 `apps/desktop/src-tauri/src/main.rs` 对齐，不等于 Rust 端到端验证。
- **宿主过期/销毁/插件停止的证明边界**：这些用例通过 `setAttention` 推送「宿主已不再列出该交互」的快照来模拟收敛，验证的是前端收到收敛后关闭弹窗；不是真的执行 Rust 端的销毁命令或 `stop_plugin`。

### 3.2 响应式布局实测数据

通过 CDP 测量当前构建（`apps/desktop/dist`）：

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

- **真实 SSH 前端 Shadow DOM（继承主题）**：`:host` `rgb(204, 204, 204)`、`.layout` `rgb(30, 30, 30)`、`.bar` `rgb(37, 37, 38)`、`.bar strong` `rgb(204, 204, 204)`，`.terminal` 存在。
- **真实 SDK block-list（继承主题）**：文字 `rgb(204, 204, 204)`、背景 `rgb(30, 30, 30)`、robot `rgb(117, 190, 255)`。
- **无主题文档中的自身 fallback**：见 2.3 表格。

---

## 4. 测试与构建运行结果（本轮实测）

| 命令 | 结果 |
| :--- | :--- |
| `pnpm install --no-frozen-lockfile` | 通过（锁文件未入库，见 `pnpm-lock.yaml` 在 `.gitignore`） |
| `pnpm build` | 通过，0 errors |
| `pnpm --filter coterm-plugin-ssh-ui build` | 通过，0 errors |
| `pnpm test` | **26 tests / 26 pass / 0 fail / 0 skipped** |
| `cargo check --workspace` / `cargo test --workspace` / `cargo clippy` | **本轮未复跑**；Rust 无改动，引用上一轮 44/44 结果 |

- **Node 用例细目（26 pass）**：
  - `tests/browser-layout.test.mjs` (3)：布局/诊断/键盘折叠、快照失败保留、多卡片审批与身份断言。
  - `tests/host-attention-state.test.mjs` (8)：订阅状态机 6 个 + 审批路由 1 个 + `secret-input` 1 个。
  - `tests/host-interactions-component.test.mjs` (9)：真实组件交互生命周期 4 个 + 补取顺序回归 5 个。
  - `tests/visual-tokens-layout.test.mjs` (6)：CSS token 定义、Tailwind 映射、SSH/SDK 样式源检查、响应式结构、宽度模拟。
- **本轮对照实验**：逐一移除三处保护后重跑同一文件，失败集合分别是 `{5}`、`{5,7}`、`{5,7,9}`；恢复后 9/9 通过。补取回归用例在修复前确实转红。
- **浏览器可用性**：本机存在 `C:\Program Files\Google\Chrome\Application\chrome.exe`，浏览器用例真实执行，未 skip。
- **测试产物构建**：`pnpm test` 会先跑 `pnpm test:build`，生成 SDK 浏览器模块与交互挂载入口；`assertFreshTestArtifacts()` 会在产物缺失或比源码旧时直接失败，不允许静默测旧实现。

---

## 5. 结论

本轮针对第四次审阅的 P1 完成返工，改动的产品代码只有 `HostInteractions.tsx`：

- 补取快照重开已取消请求：修复前用真实组件与可控闸门稳定复现（`inst-a` 对话框重新出现），修复后零对话框；5 个回归用例覆盖空快照、较新 B、B 取消后归零、卸载、旧失败五种情形；
- 旧实现只保护「已分配的弹窗」，本轮把保护扩展到补取请求本身（观察版本 + 分配代次），没有改接口、没有引入依赖、没有改回轮询、没有无限保存 `settled`；
- 既有 21 项用例全部保留并继续通过，总数 26 项、0 skip；
- 交付中移除了意外入库的 `.multica/thread2.json`，并加局部 `.gitignore` 规则防止再次入库；
- 仍未覆盖：真实 SSH 连接、真实 Tauri 运行时、硬件/串口；PR 标题与平台 PR 关联见交付评论。

仍未覆盖的部分（真实 SSH 连接、真实 Tauri 运行时、平台 PR 关联）见 3.1 与交付评论。
