## 结论

基座**当前代码**（HEAD `73a782c`，分支 `agent/deepseek/ws-9`）可编译、31 个 Rust 测试全绿，没有发现产品代码回归。但历史报告所称的"传输层统一、串口插件实现、plugin-local 迁移已完成"**不在任何基线分支上**，只存在于 `agent/planned-task-executor/ws-8`（`49a4e08`、`018d19b`）。我把该分支在仓库外的临时 worktree 里实际编译并跑了测试：**能编译，但 `plugin-local` 测试 3/4 失败**，与历史声称的"45 passed, 0 failed"不符。

本轮最高价值的三个结论：

1. **插件加载一个坏包会拖垮整个列表**（`apps/desktop/src-tauri/src/main.rs:181`、`:206`），而当前 `plugin-local`、`plugin-serial` 的 manifest 就是旧格式、必然失败，且 `plugin-serial` 没有 `dist/`。这不是理论风险，两种情况叠加后会直接导致"装了两个插件但界面一个都不显示"。
2. **宿主审批队列最多只展示 1 条**（`HostApprovals.tsx:21-29`），其余待审批操作静默挂起直到 60 秒读超时；每次轮询串行发起 `1 + N + M` 个 IPC，且要在前端读完所有实例的 100 条历史。
3. **秘密交互依赖 500ms 轮询**，而后端交互 60 秒就过期。弹出的提示框在服务端已被判定超时后仍会显示，用户输入后拿到 `interaction_not_found`。

我没有修改任何产品代码，没有启动 UI，没有测串口。所有"事实"都注明了取证方式。

---

## 1. 基线确认

| 项目 | 值 | 取证方式 |
|---|---|---|
| 当前 HEAD | `73a782c chore(agent): baseline — the task worktree started here` | `git rev-parse HEAD` |
| 分支 | `agent/deepseek/ws-9` | `git branch --show-current` |
| 工作树状态 | 仅 3 个未跟踪项：`.claude/`、`.multica/`、`CLAUDE.md`，无产品文件改动 | `git status --short` |
| 父提交 | `757cbdd`（亦即 `feature/edge-attached-approval`、主仓库当前 checkout） | `git log --oneline -5` |
| `cargo check --workspace` | 通过 | 实跑，3m08s |
| `cargo test --workspace` | **31 passed, 0 failed** | 实跑 |

### 历史声称 vs 当前代码

| 历史声称 | 当前 HEAD 实际情况 | 结论 |
|---|---|---|
| `crates/coterm-proto/src/runtime/transport.rs` | **不存在**。`runtime/` 下只有 `backend/interaction/lifecycle/manifest/message/mod/operation/stream` | 未在当前基线 |
| 串口插件后端/前端 | `plugins/plugin-serial/` **只有一个 `manifest.json`**，无 `src/`、无 `frontend/`、无 `dist/` | 未在当前基线 |
| `plugin-local` 已迁移到 `PluginBackend` | `plugins/plugin-local/` 只有 `src/main.rs`（旧入口）和旧格式 manifest | **未迁移** |
| 真实硬件 115200 验证 | 无法验证（设备已移除，且实现不在基线） | 未确认 |
| 45 tests passed | 当前基线 31；声称分支实测 **28 passed / 3 failed** | 与历史不符 |

### 声称实现的真实位置与可复现性

两个提交在 `agent/planned-task-executor/ws-8` 上，**不在任何基线分支，也没有合入 main**：

- `49a4e08`：新增 `transport.rs`(430L)、`plugin-serial` 全套（backend 655L / session 419L / frontend 428L / tests）、`main.rs` 容错改动、`docs/architecture/serial-plugin.md`
- `018d19b`：`plugin-local` 迁移（backend 355L / tests 321L / 新 manifest）

我把 `018d19b` 检出到仓库外临时 worktree（`/tmp/coterm-verify`，用 `git worktree add --detach`，未触碰任何现有工作树）实测：

```
cargo check --workspace          → Finished（通过）
cargo test --workspace           → 28 passed, 3 failed
```

三个失败全部在 `plugins/plugin-local/tests/local.rs`：

| 测试 | 失败原因 | 性质 |
|---|---|---|
| `process_host_loads_the_local_package_and_serves_plugin_info` | `plugin_platform_unsupported` | **测试缺陷**：`local.rs:55` 硬编码 `"test"` 作为平台名，而该文件 `platform()` 函数（:20-28）已正确算出 `windows-x64`，返回值在 `:39` 被 `let _ = platform;` 丢弃 |
| `health_check_within_the_handshake_deadline`、`backend_rejects_requests_that_break_the_runtime_contract` | 同上连锁 | 同一根因 |

`cargo build --workspace` 后再跑仍失败，排除构建顺序因素。根因已定位到单一常量，**不是产品代码缺陷**。验证结束后 worktree 已用 `git worktree remove --force` 删除，`/d/Vscode/code/Coterm` 保持 `757cbdd` 未动。

> 这意味着该分支不是"已完成的交付"，但也不是"不可用的废墟"——产品代码可编译，缺陷集中在一个测试常量上。Mika 可以低成本回收，但**不能直接当已验证结果合并**。

---

## 2. 宿主审批与交互

### 2.1 插件加载：单包失败使整个列表失败（P0）

**位置**：`apps/desktop/src-tauri/src/main.rs:181`、`:206`

```rust
// 开发目录加载（:181）
.map_err(|error| format!("{}: {}", error.code, error.message))?;   // ← 任一包失败，整个 list_plugins 返回 Err
// 安装目录加载（:206）
.map_err(|error| format!("{}: {}", error.code, error.message))?;   // ← 同上
```

**触发场景**（当前仓库即满足）：
1. `plugins/plugin-local/manifest.json` 与 `plugins/plugin-serial/manifest.json` 都是旧格式（`protocol`/`entry_point`/`config_schema`），缺 `manifestVersion`/`runtime`/`backend`。
2. `plugin-serial` 没有 `dist/`，`:176` 的 `is_file()` 让它被跳过；`plugin-local` 的 `dist/` 若存在但 manifest 是旧的，则 `:181` 直接让整个 `list_plugins` 失败。
3. 安装目录同理——`create_dir_all` 后是空的，目前不触发，但用户放一个坏包进去就触发。

**影响**：一个坏插件让 SSH 也不显示。用户看到的是空插件栏，没有任何指向具体包的提示。

**建议改动**：把两处 `?` 换成"记录并跳过"，失败项收集进 `PluginSummary` 或单独的 `errors: Vec<{path, code, message}>` 字段返回。历史声称分支 `49a4e08` 已做过这个改动，可直接参考其实现。

**验证方式**：在开发 `plugins/` 下放一个 manifest 损坏的目录，断言 `list_plugins` 仍返回健康的 SSH，且错误列表含该目录。

**跨进程影响**：无。仅改 `list_plugins` 命令的返回结构，需同步 `host-api.ts` 的 `PluginSummary` 类型。

---

### 2.2 宿主审批队列：一次只显示一条（P0）

**位置**：`apps/desktop/src/components/host/HostApprovals.tsx:8`、`:21-29`、`:33`

```tsx
const [pending, setPending] = useState<{ ... } | null>(null);   // :8 ← 单槽位
...
const record = records.find((r) => r.status === "awaiting_approval" && r.proposalId);
if (record) { next = { instance, record }; break; }              // :22 ← 命中即 break
...
if (record) { next = { plugin, record }; break; }                // :29 ← 同上
```

**触发场景**：Agent 在实例 A 与实例 B 上各发起一个需要审批的 commit 操作。

**影响**：B 的审批卡片永远不出现，直到 A 被决策或 B 的 `wait_timeout_ms` 到期。默认等待若为 60s，用户只看到一条、点了批准，B 可能已经在服务端超时——用户完全不知道还有第二条。这是审批界面**漏报**，而非仅仅是显示不佳。

**证据**：`:22`/`:29` 的 `break` 与 `:8` 的单对象 state 类型，共同保证"至多一条"。

**建议改动**：`pending` 改为 `OperationRecord[]`（按 `operationId` 稳定排序，先进先出），渲染为纵向卡片栈。单卡片的 `deciding` 锁在 `ApprovalCard.tsx:32-36` 是组件级的，多卡片并存天然安全。

**验证方式**：用两个实例同时触发审批（或桩一个返回两条 `awaiting_approval` 的 `recent_operations`），断言两张卡片同时可见、可独立决策。

---

### 2.3 轮询成本与竞态（P1）

**位置**：`HostApprovals.tsx:17-30`、`:38`

每 1000ms，串行执行：

- `listInstances()` → `list_plugins` 的 Rust 端会**重新扫描并加载全部插件目录**（`:168-207`），不只是读缓存
- 对每个实例 `recentOperations(id, 100)` → 每次克隆最多 100 条记录（`plugin_manager.rs:151-157` → `operation_history.rs:111-127`）
- 若前两步无结果，再 `listPlugins()` + 每个插件 `recentPluginOperations(id, 100)`

即 `1 + N + M` 个串行 IPC，每个都要复制到 100 条 `OperationRecord`，**全在前端 JS 里做筛选**。而"是否还有待审批"在 Rust 侧是 `state.pending_approvals` 一个 HashMap（`plugin_manager.rs:46`），成本接近零。

`revision` ref（`:10`、`:15`、`:31`）只防了"决策后重启旧提案"，没防并发 `poll`——`poll` 结尾无条件调度下一次（`:38`），若某次超过 1s，会叠加。

**建议改动（最小契约，见 2.5）**：后端提供一个快照命令，前端只负责渲染与事件驱动刷新。

---

### 2.4 秘密交互：过期后仍会弹出（P0）

**位置**：`HostInteractions.tsx:12-32`、`:34`；`plugin_manager.rs:340-341`、`:505-520`

后端行为：
- 插入交互时按 `timeout_ms.unwrap_or(60_000)` 计算截止（`plugin_manager.rs:335-341`）
- `main.rs:655-662` 起一个**每秒**跑 `expire_interactions()` 的后台任务，到期即从 `interactions` 移除并给插件发 `TimedOut`

前端行为：每 **500ms** 轮询一次 `pending_interactions()`。

**竞态窗口**：假设交互在服务端还剩 300ms 过期。前端在 T 时刻拿到它，弹出对话框，用户输入密码耗时 3 秒，提交时后端已移除该 ID → `respond_secret` 返回 `interaction_not_found`（`plugin_manager.rs:413-415`）。用户白白输入了一次密码，看到的是一条英文错误。

反过来同样糟：`HostInteractions.tsx:16` 用 `active` 本地状态判断"请求是否消失"，若一次轮询失败（`:33` 吞掉所有异常），`active` 保持，对话框会一直挂着。

**建议改动**：
1. 后端增加交互的 `updated_at`/`deadline` 快照字段，前端在**剩余时间 < 5s 时不再弹出新请求**，直接视为过期。
2. 对话框标题区显示倒计时，对齐服务端 `deadline` 而非前端本地计时。
3. 轮询异常不应静默；至少累计失败次数后在 `HostInteractions` 的 `error` 区域提示。

**验证方式**：把 `timeout_ms` 设为 1500，断言 1000ms 后不再弹出新对话框；提交一个已过期 ID，断言前端给出可读的"已过期"而非原始错误码。

---

### 2.5 宿主级快照 + changed 通知的最小契约（建议）

**现有约束（必须先承认）**：`subscribe_plugin_events` 是**按实例**订阅的（`main.rs:55-125`，`plugin_manager.rs:107-121` 要求实例存在且插件已启动）。它**不能**用来承接全局通知——无实例的插件级操作、未启动插件的待审批项、以及实例销毁后的残留，都收不到。轮询换事件不能只是"监听某个现有事件"。

**最小契约**：

```rust
// 快照：一次 IPC 覆盖全部来源，替代 1+N+M
#[tauri::command]
async fn host_attention(state) -> Result<HostAttention, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HostAttention {
    version: u64,                       // 单调递增，任何进入/离开变化都 +1
    approvals: Vec<ApprovalEntry>,      // 全部待审批，不再只取第一条
    interactions: Vec<InteractionEntry>,
}

struct ApprovalEntry {
    operation_id: String,
    proposal_id: String,
    plugin_id: String,
    plugin_instance_id: Option<String>, // None = 插件级
    operation: String,
    target_label: String,
    created_at: String,
}

struct InteractionEntry {
    interaction_id: String,
    plugin_instance_id: String,
    title: String,
    prompt: String,
    remaining_ms: Option<u64>,          // 由后端算，前端不自己计时
}
```

**changed 通知**：新增全局事件 `host://attention-changed`，payload 只有 `{ "version": u64 }`。**不含**操作参数、不含任何秘密值——与 `AGENTS.md` 的"秘密不得进入普通事件"一致。前端收到事件后重新拉一次快照。

**订阅与快照竞态的标准解法**（顺序写死）：

```
1. listen("host://attention-changed") 并开始缓冲 version
2. invoke("host_attention") 取快照 S
3. 丢弃缓冲中 version <= S.version 的事件
4. 用 S 渲染；此后每个 version > 当前 的事件触发一次重取
```

**去重**：以 `version` 为主做整体替换，而不是逐条 diff。`approvals`/`interactions` 内部按 ID 稳定排序；渲染层用 `operationId`/`interactionId` 作 React key（`ApprovalCard.tsx` 已用 `key={pending.record.operationId}`，可直接延用）。

**错误恢复**：快照命令失败时保持上一次成功快照 + 显示一条警示（现有 `pollingError` 的文案可复用），**不**清空界面——清空会让用户误以为待审批已消失。

**卸载注销**：`unlisten` 必须在组件卸载时调用；Tauri 的 `listen` 返回 `Promise<UnlistenFn>`，注意与现有 `createPluginAPI` 中 `subscribe` 的 `disposed` 守卫同构（`host-api.ts:192-199` 已有可参考的写法）。

**为什么用 version 而不是增量事件**：待审批是**小而全集**的状态（实测上限受 100 条历史 + 16KB 单值限制约束），整体替换天然免疫"丢事件"和"乱序"。增量事件设计需要处理补发、断线重放、越界回滚，成本远高于收益。

**验证方式**：
- 单测：`host_attention` 在"2 个实例各 1 条 + 1 个插件级 1 条"下返回 3 条
- 竞态：在 `listen` 与 `invoke` 之间插入一次状态变化，断言最终渲染与真实状态一致
- 停机：卸载组件后再触发一次变化，断言无 setState 警告

**注意**：本条涉及 Tauri 事件与跨进程契约（新增 command + 全局事件名），按 `AGENTS.md` 的"修改边界"需同步更新 `docs/architecture/runtime-communication.md`。

---

### 2.6 审批状态一致性核对表

| 场景 | 后端行为 | 前端可观测性 | 结论 |
|---|---|---|---|
| 审批过期 | `pending_approvals` **没有**超时清理逻辑 | 仅靠 `wait_timeout_ms` 让记录离开 `awaiting_approval` | **不一致**：审批本身无独立过期，与交互（有 60s 过期）不对称。建议至少明确这是有意设计并写进文档 |
| 插件停止 | `stop()` 取消该插件全部 pending，置 `Cancelled`（`plugin_manager.rs:864-876`） | 下次轮询消失 | 一致 |
| 实例销毁 | `cancel_instance_interactions` 取消实例级审批与交互（`:522-563`） | 同上 | 一致 |
| 审批被 `cancel_operation` 取消 | 走 reject 分支并置 `Cancelled`（`:1350-1370`） | 同上 | 一致 |
| Bypass 开启 | `requires_proposal` 仅对 `Agent + Commit + !bypass` 生成提案（`:1585-1587`） | 底栏显示 "Bypass on" | 一致；但**Bypass 是进程级全局开关，无持久化**，重启即复位。文档中未说明这一点 |
| 同实例并发多审批 | 后端支持（HashMap 多条） | **前端只显示 1 条**（见 2.2） | **不一致，P0** |

审批枚举完整性：`OperationStatus` 有 9 个变体（`operation.rs:49-61`），`HostApprovals` 只关心 `awaiting_approval`，其余 8 个由 `App.tsx` 的历史列表呈现——这部分是完整的，不需要改。

---

## 3. 插件契约与基座边界

### 3.1 Manifest 三种状态并存（P1）

| 插件 | manifest 格式 | 实体文件 | 能否被当前基座加载 |
|---|---|---|---|
| `plugin-ssh` | 新格式（`manifestVersion`/`runtime`/`backend`/`frontend`） | `src/` + `frontend/` 完整 | 能（前提是有 `dist/`） |
| `plugin-local` | **旧格式**（`protocol`/`entry_point`/`config_schema`） | 仅 `src/main.rs` | **不能** |
| `plugin-serial` | **旧格式** | **仅 manifest.json** | **不能**（且无 `dist/`） |

另外两个插件都引用了 `"$schema": "../../schemas/plugin-manifest.schema.json"`，但**仓库根没有 `schemas/` 目录**（`ls schemas` → 不存在）。这是一个悬空引用，任何 IDE 校验都会报错，也会误导后续开发者以为存在官方 schema。

**建议**：二选一——补上 `schemas/plugin-manifest.schema.json`（从 `plugin_process.rs` 的 `PluginManifest` 反推），或删掉 `$schema` 行。前者更好，可让插件作者在写 manifest 时就发现错误。

**`plugin-local` 的处置**：`018d19b` 已给出完整迁移（355L backend + 321L tests + 新 manifest），修掉 2.1 中那个 `"test"` 常量后即可作为干净的工作包起点。**不建议在本轮直接删除**——它是"无前端插件"这一架构说法的唯一具体例证。

### 3.2 旧代码调用路径核实（不按名字删代码）

| 类型 | 定义 | 实际引用方 | 结论 |
|---|---|---|---|
| `SessionManager` | `crates/coterm-core/src/session.rs:31` | 仅 `main.rs:37` 存为 `_session_mgr` 字段，**从未被读取或调用**（下划线前缀即其自身声明） | **死代码**，可删 |
| `Session` | `session.rs:8` | 仅 `SessionManager` 内部 | 随 `SessionManager` 一起死 |
| `Osc133Parser` | `block_parser.rs:12` | 仅 `session.rs:12` 的字段 | **链路死**，但能力有迁移价值（见下） |
| `SafetyGuardrail` | `guardrail.rs:3` | 仅 `session.rs:14` 的字段 | **链路死**。注意：它是一条**正则黑名单**，不是安全边界（`AGENTS.md` 与 SPEC 都强调协议语义归插件），不建议原样迁移 |
| `InputArbiter` | `arbiter.rs:14` | 仅 `session.rs:13` 的字段 | **链路死**。但"人类抢占 Agent 输入"是 SPEC 原则 5 的诉求，属**设计未落地**而非设计错误 |
| `AppState._session_mgr` | `main.rs:37`、`:667` | 无 | 可整条移除 |

**关键证据**：`grep -rn "Osc133Parser\|SafetyGuardrail\|InputArbiter" --include=*.rs` 的全部命中都在 `coterm-core` 自身内部（定义 + `session.rs` 的字段 + `lib.rs` 的 re-export）。`plugins/plugin-ssh/src/` 里**没有任何 OSC 133、guardrail 或 arbiter 的引用**——SSH 插件是自己实现协议语义的。

**建议分层处置**（而不是"名字像旧代码就删"）：

- **可删**：`SessionManager`、`Session`、`main.rs` 的 `_session_mgr`。零风险，纯减负。
- **可归档（保留在 crate 但标注 deprecated）**：`Osc133Parser`——SSH 插件将来若要做命令块，需要 OSC 133 解析；但**它应该搬进 `plugin-ssh`**，而不是留在基座。现状是"能力正确、归属错误"。
- **先澄清再动**：`InputArbiter`。它的 UI 接线依赖一个前提：基座要知道"当前输入属于人类还是 Agent"。当前插件的输入路径（`plugin-ssh/frontend` 直接操作 xterm → `invoke` 操作）**没有这个概念**。所以 `InputArbiter` 不是"接个 UI 就完事"，而是需要先决定输入归属语义放基座还是插件。**本轮不应动它**，Mika 的建议（先厘清关系再展示状态）是对的。
- **不建议迁移**：`SafetyGuardrail` 的正则黑名单。任何基于命令字符串黑名单的安全控制都可绕过；若要恢复"高危操作需审批"，应走现有的 `OperationEffect::Commit` + `proposal` 机制，那是结构化的、插件声明的，而非正则猜测。

### 3.3 终端原型（`apps/desktop/src/components/terminal/`）

**核实结果**：5 个文件（`TerminalView.tsx`、`TerminalGutter.tsx`、`useTerminal.ts`、`terminalTheme.ts`、`types.ts`、`index.ts`），**仅被自身的 `index.ts` 互相引用**，`App.tsx` 从未导入。

**与 Gemini 报告的分歧**：该报告称它"带有特定于命令块和 Ghost Text 的审批逻辑...与正式架构冲突"。实际核对：它是一套独立原型，**不参与构建**（`index.ts` 无人导入，Vite 不会打包）。所谓"冲突"是概念层面的（它是基座内的终端实现，而架构要求终端归插件），不是运行时的。

**结论**：可归档删除，但**优先级 LOW**——它不影响生产代码，也不影响构建产物。把它排在第一优先级是过度反应。

### 3.4 MCP 端口诊断

`crates/coterm-mcp/src/http.rs:40` `DEFAULT_PORT = 23517`，`:58-77` 绑定 `127.0.0.1:23517`，失败即整体 `Err`。`main.rs:652-654` 的映射信息是：

> `MCP unavailable at http://127.0.0.1:23517/mcp: {error}. Close the application using this port and restart Coterm.`

**核实**：硬编码属实；诊断**已经存在且文案具体**（指出端口、要求关闭占用进程）。Gemini 报告称"若端口被占用则无优雅回退或提示"——**提示这一半不成立**。缺的只是"可配置端口"。

**结论**：降级为 LOW。真要改，最小改动是读环境变量（如 `COTERM_MCP_PORT`），不改默认行为。

---

## 4. UI 与 SDK

### 4.1 视觉 token：无共享机制，且 Shadow DOM 天然隔离（P1）

**实测数据**：

- `apps/desktop/src/` 下 **127** 处 Tailwind 任意值颜色类（`bg-[#...]`/`text-[#...]`/`border-[#...]`）
- 跨 12 个文件共 **181** 处十六进制字面量
- `apps/desktop/tailwind.config.js` 的 `theme.extend.colors` **只定义了一个 `agent.500/900` 紫色**，没有任何语义色

**关键结构性事实**：`plugins/plugin-ssh/frontend/src/index.ts:31-41` 用 `element.attachShadow({ mode: "open" })` 挂载。**Shadow DOM 不继承外部 CSS 变量以外的任何样式规则**——Tailwind 的 utility class 进不去。所以：

- 想共享视觉，**唯一可行机制是 CSS Custom Properties**（它们能穿透 shadow boundary）
- `block-list.ts:3-15` 在 DOM 里插入 `<style>` 并写死颜色（`#1e1e1e`、`#094771`、`#3794ff`…），说明这个组件也无法跟随主题

**建议的最小改动**（三步，互不阻塞）：

1. 在 `apps/desktop/src/index.css` 的 `:root` 定义一组语义变量：`--ct-surface`、`--ct-surface-raised`、`--ct-border`、`--ct-text`、`--ct-text-muted`、`--ct-accent`、`--ct-danger`、`--ct-font-mono`。
2. `tailwind.config.js` 的 `colors` 改指向 `var(--ct-*)`，让 127 处任意值逐步替换为语义类名（**可以分批做，不需要一次改完**）。
3. `plugin-ssh` 的 shadow style 与 `block-list` 改用 `var(--ct-*, <fallback>)`。带 fallback 是关键——这样即使宿主没定义变量，插件也不会裸奔。

**为什么不建议"统一做一套设计系统组件库"**：`@coterm/plugin-sdk` 目前只有 2 个导出（`index.ts:1` 的 type-only 导出 + `components/block-list`）。在此之上铺组件库，等于在契约尚未冻结（`plugin-development.md` 自述未冻结）时先冻结 UI 实现。token 是低风险的，组件库不是。

### 4.2 固定导航宽度与折叠（P2，需先测量）

**位置**：`App.tsx:246`（插件栏 `w-[72px]` 固定）、`:289`（实例栏 `w-[240px]`，仅 `max-[900px]:w-[200px]` 一档收缩）

合计 **312px**（<900px 时 272px），全程无折叠能力。主工作区是 `min-w-0 flex-1`（`:344`），所以不会溢出，只会被压窄。

**核实**：`index.css` 中除 `prefers-reduced-motion` 外**没有任何 `@media` 规则**，也没有折叠状态或触发器。

**结论**：报"固定 312px"是**事实**；但"挤压主视图到不可用"是**推断**，我没有在真实窗口尺寸下验证过（本轮未启动 UI）。这个改动应该由一个实测数据驱动——比如"在 1024×768 下 xterm 可视列数从 X 降到 Y"。**不要仅凭示意图开工。**

### 4.3 焦点管理（P2）

- `ApprovalCard.tsx:56-65`：Esc 绑定在表单上且 `stopPropagation()`，Enter 用 `nativeEvent.isComposing` 守卫（兼顾中文输入法）——**这块做得对**，不需要改。
- **缺失**：卡片出现时不抢焦点。这是刻意的"非模态"设计，但对键盘用户意味着必须 Tab 过去才能响应。配合 `aria-label` 与 `aria-busy`（:51-53）已具备，缺的是"新卡片出现时把焦点移到卡片但**不**移到输入框"（避免吞掉用户的命令输入）。
- `HostInteractions.tsx` 弹的是 `<dialog>`（`index.css:37-54`），焦点由浏览器管理，这部分没问题。

### 4.4 加载/空/错误/无前端状态（P2）

| 状态 | 覆盖情况 | 位置 |
|---|---|---|
| 插件列表加载中 | 有骨架屏 | `App.tsx:253-255` |
| 插件列表为空 | 有 "No plugins" | `:274` |
| 实例列表加载中 | 有骨架屏 | `:302-304` |
| 实例列表为空 | 有引导文案 | `:329-334` |
| 全局错误 | 有 `errorNotice` 组件 | `:227-231` |
| **插件无前端** | **缺失**——`PluginWorkspace.tsx:16` 拿到 `null` module 时直接 `return`，主区一片空白 | `:15-17` |
| 插件前端加载失败 | 有 `error` 提示 | `:18-20` |

**`PluginWorkspace.tsx:15-17` 是明确的缺口**：

```tsx
void loadPluginFrontend(instance.pluginId).then((module) => {
  if (disposed || !module) return;   // ← 无前端 = 静默空白
```

对 `plugin-local` 这类"无前端插件"（架构明确支持的形态），用户选中实例后看到纯黑区域，没有任何解释。**这是本轮性价比最高的 UI 修补**：只需一个"此插件没有提供界面，可通过 MCP 或插件操作调用"的空状态。

### 4.5 其他已核实的小问题

| 问题 | 位置 | 说明 |
|---|---|---|
| 顶栏只有一个 MCP 按钮，`h-8` 高度大部分为空 | `App.tsx:234-240` | 属实，但不构成缺陷——留白是布局选择，不是遗漏 |
| 底栏只有插件数/实例数 | `:346-352` | 属实。补 Tx/Rx 属于串口范围，本轮不做；但"当前活动实例名"这类**基座通用**信息是可加的 |
| 插件状态变化不推送 | `App.tsx` 的 `refresh()` 仅在显式操作后调用 | 插件后端崩溃后界面仍显示 Running，需手动刷新。属 P2 |
| 字体栈 7 处重复，且 `JetBrains Mono` **未安装** | `index.css:17`、`plugin-ssh/frontend:7`、`block-list.ts:4` 等 | 写的是 `"Segoe UI"`/`Consolas`/`monospace`，`grep "JetBrains"` 无命中——所以**没有**引用不存在的字体，只是没有统一变量 |

---

## 5. 对 Gemini 旧报告的逐项判定

| # | Gemini 结论 | 判定 | 依据 |
|---|---|---|---|
| 1 | 顶栏/底栏信息利用不足 | **成立（但优先级低）** | `App.tsx:234-240`、`:346-352` |
| 2 | 实例配置层级过深，须弹窗创建 | **成立（P2）** | 创建走 `Modal` → `PluginConfiguration`/JSON 表单 |
| 3 | `components/terminal/` 是"未引入的原型，与架构冲突" | **需修正** | 确未被引用（仅自引用），但"冲突"应为"概念归属错误 + 不参与构建"，故优先级 LOW 而非高 |
| 4 | 串口热插拔/就地改波特率缺失 | **超出本轮范围** | 设备已移除 |
| 5 | `InputArbiter` 已实现但前端未打通 | **部分成立，需修正** | 类型存在但**整条调用链是死的**（仅 `session.rs` 引用）。不是"实现了没接线"，而是"未接线且无人使用"。不建议先做 UI |
| 6 | `HostApprovals` 1s / `HostInteractions` 500ms 轮询低效 | **成立（P1）** | `HostApprovals.tsx:38`、`HostInteractions.tsx:34` |
| 7 | 多重设计语言割裂（基座/原型/SSH/block-list 各写各的） | **成立（P1）** | 181 处 hex，12 个文件；Shadow DOM 见 4.1 |
| 8 | 固定 72+240px 挤压主视图 | **成立但为推断** | 宽度属实（`App.tsx:246/289`），"挤压到不可用"未实测 |
| 9 | `plugin-serial` manifest 旧 schema，基座无法加载 | **成立（P0 组合项）** | 已核实；叠加 2.1 会拖垮整个列表 |
| 10 | `main.rs` 硬编码 dev 目录 `../../../plugins/*/dist/manifest.json` | **成立** | `main.rs:172` |
| 11 | MCP 端口硬编码且**无提示** | **需修正** | 硬编码属实（`http.rs:40`），但**已有提示**（`main.rs:654`），仅为不可配置 |
| 12 | `Osc133Parser`/`SafetyGuardrail` 在核心库硬编码 bash 语义 | **成立** | 且整条链是死的（见 3.2） |
| 13 | 插件 SDK 只提供拼字符串 CSS，缺 token/基类 | **成立** | `block-list.ts:3-15` |
| 14 | 串口双模式终端完整规格矩阵 | **超出本轮范围** | 设备已移除；且属于插件而非基座 |
| 15 | 阶段 1「清理技术债 + 事件化改造」 | **方向成立，但顺序需调整** | 事件化必须**先建宿主级契约**（2.5），不能直接监听现有插件事件 |

**Gemini 报告遗漏、本轮新增的发现**：
- 审批队列只显示一条（2.2，P0）
- 秘密交互过期竞态（2.4，P0）
- `schemas/plugin-manifest.schema.json` 悬空引用（3.1）
- 无前端插件静默空白（4.4，P2）
- 历史声称实现的分支存在失败测试（1，基线事实）

---

## 6. 建议工作包（供 Mika 决策，我不执行）

四个包按依赖排序，**1 → 2 无依赖，3 依赖 1 的容错修改，4 独立**。每个包可单独验收、单独回滚。

### 工作包 A：插件发现容错 + manifest 契约收敛

| 项 | 内容 |
|---|---|
| **文件所有权** | `apps/desktop/src-tauri/src/main.rs`（仅 `list_plugins`）、`apps/desktop/src/host-api.ts`（`PluginSummary` 类型）、可选新增 `schemas/plugin-manifest.schema.json` |
| **前置依赖** | 无 |
| **跨进程/权限/依赖** | 改 Tauri command 返回结构（需同步前端类型）；**不新增依赖、不改 capability** |
| **范围** | ①`main.rs:181`/`:206` 单包失败改为记录并跳过 ②返回 `errors` 列表供 UI 展示 ③消除 `$schema` 悬空引用 ④（可选）参考 `49a4e08` 的容错实现 |
| **验收** | 放一个坏 manifest 的包到 dev 目录 → `list_plugins` 仍返回健康插件且错误列表含该包；`cargo test` 全绿；`pnpm build` 通过 |

### 工作包 B：宿主级 attention 快照 + changed 通知

| 项 | 内容 |
|---|---|
| **文件所有权** | `crates/coterm-core/src/plugin_manager.rs`（新增只读快照方法）、`apps/desktop/src-tauri/src/main.rs`（新增 command + 事件发射）、`apps/desktop/src/host-api.ts`、`HostApprovals.tsx`、`HostInteractions.tsx` |
| **前置依赖** | **A 的容错修改**（快照要能遍历插件而不因坏包失败） |
| **跨进程/权限/依赖** | **涉及跨进程契约**（新 Tauri command + 全局事件名 `host://attention-changed`）→ 必须同步 `docs/architecture/runtime-communication.md`；**不新增 crate 依赖**；不改 capability |
| **范围** | 按 2.5 的契约实现快照命令；审批改多卡片（2.2）；交互加 `remaining_ms` 与过期保护（2.4）；移除两个轮询定时器 |
| **验收** | 两条待审批同时可见且可独立决策；接管 `listen`↔`invoke` 之间的竞态用例；`timeout_ms=1500` 时不弹出已过期交互；卸载无 setState 警告；事件 payload 中搜不到任何秘密字段 |

> A 和 B 合并可一次性解决三个 P0（坏包拖垮列表、审批漏显示、交互过期）。如果 Mika 想减少来回，**A+B 是一个自然的合并单元**，前提是文档同步跟上。

### 工作包 C：视觉 token 骨架（分批、可回滚）

| 项 | 内容 |
|---|---|
| **文件所有权** | `apps/desktop/src/index.css`（`@layer base` 下的 `:root`）、`apps/desktop/tailwind.config.js`、`packages/plugin-sdk/src/components/block-list.ts`、`plugins/plugin-ssh/frontend/src/index.ts`（仅 css 常量块） |
| **前置依赖** | 无（与 A/B 完全正交） |
| **跨进程/权限/依赖** | 无。**不改任何插件与基座之间的接口**，纯样式 |
| **范围** | 定义 `--ct-*` 语义变量；Tailwind `colors` 指向变量；`block-list` 与 SSH shadow style 改用 `var(--ct-x, fallback)`。**不要求一次替换完 127 处**，先建立机制与 2-3 个示范点 |
| **验收** | 在宿主 `:root` 改一个变量的值，插件 shadow DOM 内的对应颜色随之变化（这是唯一能证明"共享"生效的方法）；`pnpm build` 通过；视觉无回归（逐屏对比） |

### 工作包 D：无前端插件空状态 + 焦点可达性

| 项 | 内容 |
|---|---|
| **文件所有权** | `apps/desktop/src/components/plugins/PluginWorkspace.tsx`、`apps/desktop/src/components/host/ApprovalCard.tsx`（仅焦点逻辑）、`apps/desktop/src/App.tsx`（仅在 `main` 区块内） |
| **前置依赖** | 无；若与 B 并行，注意 `HostApprovals` 的行号冲突 |
| **跨进程/权限/依赖** | 无 |
| **范围** | ①`PluginWorkspace.tsx:16` 的 `!module` 分支渲染空状态文案（说明可经 MCP/操作调用）②新审批卡片出现时移焦点到卡片容器而非输入框 ③（可选）底栏补当前活动实例名 |
| **验收** | 选中一个无前端插件 → 显示说明而非黑屏；键盘 Tab 顺序连续，Esc 仍可拒绝；`pnpm build` 通过 |

**明确不在任何工作包内**（Mika 已定或用户已定）：串口相关一切、`InputArbiter` UI 接线、组件库/设计系统、`terminal/` 原型删除（可另开低优先级清理）、MCP 端口配置化（LOW，除非顺手）。

---

## 7. 未验证项（诚实声明）

以下我**没有**做，不能写为通过：

1. **未启动 UI**。所有前端结论来自源码阅读，不含真实窗口渲染、键盘焦点实测、窄窗口挤压实测。"312px 挤压主视图"是几何计算，不是体验结论。
2. **未运行 `pnpm build` / `pnpm install`**。前端类型检查与打包结果未知；工作包验收条件里的 `pnpm build` 尚未在基线上确认过一次。
3. **未跑 `cargo clippy -- -D warnings`**。`AGENTS.md` 要求，本轮只跑了 `check` 与 `test`。
4. **未测串口**（设备已移除，符合本轮约束）。
5. **`49a4e08`/`018d19b` 的真实硬件验证无法复核**——设备不在，该声称既不能确认也不能证伪。
6. **未验证 Bypass 开关重启后复位**的产品影响；仅从 `plugin_manager.rs:66` 的初始值 `false` 推断无持久化。
7. **未验证** `plugin-ssh` 的 `dist/` 是否存在——若不存在，当前基线在 dev 模式下插件栏同样是空的。

如果 Mika 需要，第 2、3 项是可以在下一个短轮次里补上的低成本验证。

---

## 附：本轮操作足迹

- 只读审阅：`docs/`、`apps/desktop/src-tauri/src/main.rs`、`apps/desktop/src/`、`crates/coterm-core/`、`crates/coterm-proto/`、`crates/coterm-mcp/`、`plugins/*`、`packages/plugin-sdk/`
- 实跑：`cargo check --workspace`（通过）、`cargo test --workspace`（31 通过）
- 隔离验证：`git worktree add --detach /tmp/coterm-verify 018d19b` → `cargo check`（通过）+ `cargo test`（28 通过 / 3 失败）→ `git worktree remove --force`。主仓库 `/d/Vscode/code/Coterm` 保持 `757cbdd`，当前工作树未改动，未合并、未重置、未覆盖任何分支。
- 未修改产品代码、未提交、未创建 PR、未新增依赖、未改权限或协议、未发起其他成员任务。
