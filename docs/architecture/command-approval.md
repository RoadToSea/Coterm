# Agent 审批规则组与实例绑定

规则组由宿主独立保存，不依赖实例存活，可在没有任何实例时创建，也可同时被不同驱动的实例引用。每个实例最多绑定一个组；未分配组且 Bypass 关闭时，所有 Agent commit 需要审批。删除实例不删除组，未绑定组是有效配置。

规则组字段为 `{id, name, mode, patterns}`：id 是稳定标识，改名不改变引用；名称去除首尾空格后非空、最多 64 个字符、不含控制字符且不重名（忽略大小写）。实例保存 `approvalRuleGroupId: string|null`，不保存规则正文副本。最多 128 个组；每组最多 128 条规则，单条最多 2048 个 UTF-8 字节，总量最多 16 KiB。使用 Rust `regex` 语法，不支持环视、反向引用；编译结果和匹配缓存有内存上限，无效规则拒绝保存。

每个运行实例创建独立 `InstanceApproval`，拥有当前编译策略及 Bypass；共享组更新时批量替换编译策略，不共享可变的实例开关。Bypass 仍只属于实例，与组的复制、编辑或改绑无关。

| 模式 | 命中任一规则 | 未命中规则 |
|---|---|---|
| 正向放行 `allow_matches` | 免审批 | 人工审批 |
| 反向拦截 `approve_matches` | 人工审批 | 免审批 |

“拦截”表示暂停并等待人工批准或拒绝。新建实例默认正向模式、空规则，因此所有 Agent commit 仍需审批。反向模式的空规则会放行所有参与匹配的命令；模式选择在保存前始终可见。

应用设置的「审批」提供规则组管理入口：搜索、创建、复制、改名、正反向模式、规则编辑 / 追加 / 删除、宿主试匹配和显式保存。宽窗口采用列表与编辑器两列，窄窗口在列表与详情间切换。组列表显示已分配 / 未分配；保存前展示受影响实例名称。复制创建独立组，不复制绑定。已绑定组禁止删除，须先改绑或取消所有实例分配。

实例审批面板选择一个组或「未分配」，只读查看组规则；编辑通过同一对话框切换至规则组管理页。待恢复实例也可通过侧栏审批入口改绑，无需启动驱动或建立连接。面板单独展示 Bypass；开启时明确说明当前跳过审批，不能把选中组的摘要当成实际审批结果。

规则及绑定编辑均显式保存，失败保留草稿。离开脏组或关闭有未保存分配的面板时提供保存、放弃、继续编辑。命令测试使用草稿中的 mode/patterns，不执行命令，也不把组规则匹配结果当作 Bypass 开启时的实际审批结论。

正则默认匹配任意位置；用 `^`、`$` 限制整条单行命令，或 `\A`、`\z` 限制整个原始文本。`^ls$` 仅匹配 `ls`，`\brm\b` 可用于反向模式匹配含 `rm` 的命令。需要简单参数时可以使用 `\Als(?:[ \t]+[^;&|\x60$<>\r\n]+)?\z`。正则只匹配文本，不解析 Shell 语法、别名或被调用脚本的内容；用户决定规则覆盖范围。

命令数组按每个原始列表项匹配，不裁剪、不拆行、不拼接：正向模式要求每项都命中至少一条规则，反向模式有一项命中就让整个提交进入审批。多行脚本始终作为一个完整项匹配。空数组、错误类型、空文本、超过 128 项或合计超过 1 MiB 的输入保留审批，不以宽松匹配回退。

## 宿主与驱动契约

驱动在 operation 的 `inputSchema` 顶层声明可选扩展 `x-coterm-approval-text`，值为指向实际命令文本或字符串数组的 JSON Pointer，例如 `/text`、`/commands`。声明来自注册目录，调用参数不能覆盖它。宿主在 schema 校验后读取该路径，使用已编译规则判断；不扫描其他参数、配置或秘密，不猜测具体协议。

SSH 内部 command 声明 `/text`，MCP command 声明 `/commands`；terminal/exec 执行模式与命令同属审批参数快照；串口文本 command 及其 MCP 镜像声明 `/commands`。串口 HEX、底层 write、原始按键、文件写入、连接配置等不声明命令文本，继续使用原审批规则。新驱动可以通过同一 schema 扩展接入，不需要新增宿主协议适配器。

只有 Agent 来源的 `effect=commit` 受此门控；Human/System、read/draft 沿用已有行为。未声明、无效指针、缺失或类型错误的字段不能获得规则豁免。`sensitiveInput` 继续控制账本脱敏；例如串口文本命令同时声明命令路径和敏感标记，只在内存匹配，原文不进入普通日志或 MCP 账本。用户主动开启的 Bypass 只跳过当前目标实例的宿主审批；规则与 Bypass 均不跳过驱动参数校验、来源限制、协议认证或秘密交互。已等待审批的提案不因修改规则或 Bypass 自动执行；后续新提交才使用目标实例的新规则。驱动级 Agent commit 不借用任意实例的 Bypass 或规则，仍需审批。

运行时先验证来源、实例归属、会话、就绪状态和参数，再向目标实例对象传入 `ApprovalContext {source, descriptor, args}`。descriptor 来自已注册目录，source 来自可信入口；调用参数不能覆盖二者。对象只返回是否需要审批，不负责协议连接或执行。

桌面专用命令：

| 命令 | 请求 | 响应 |
|---|---|---|
| `approval_snapshot` | `{}` | `{groups:[{id,name,mode,patterns}], instances:[{configRef,label,driverId,instanceId,ruleGroupId,bypass}]}`，含离线保存项，不含连接配置 |
| `save_approval_group` | `{group:{id:string|null,name,mode,patterns}}` | 保存后的规则组；null id 新建，其余更新已有组 |
| `assign_approval_group` | `{configRef,groupId:string|null}` | 无返回数据；null 取消分配，支持离线实例 |
| `delete_approval_group` | `{groupId}` | 无返回数据；组正在使用时返回 `approval_group_in_use` |
| `get_bypass` | `{driverInstanceId}` | 当前运行实例的布尔状态 |
| `set_bypass` | `{driverInstanceId,enabled}` | 无返回数据；未知实例或保存失败时报错 |
| `preview_approval_policy` | `{policy:{mode,patterns},text}` | `true` 需审批，`false` 免审批；使用同一 Rust matcher，不执行任何操作 |

这些命令只由可信桌面使用，不导出给驱动前端或 MCP。组名称重复、格式无效、容量限制、引用丢失分别返回 `approval_group_name_exists`、`approval_group_name_invalid` / `approval_group_invalid`、`approval_group_capacity`、`approval_group_not_found`。无效正则返回 `approval_rule_invalid`。所有失败保持旧组、绑定和已生效策略。未知 configRef 返回 `workspace_entry_not_found`，不能回退到别的实例。

配置变更与实例生命周期共用宿主工作区锁。保存组先校验并编译、原子持久化，再在运行时单次加锁发布到所有仍存活的绑定实例；保存成功后，后续新提交统一使用新规则。发布跳过已停止实例，保存项下次恢复时解析当前组。分配一个组也先编译和落盘，再更新目标运行实例；没有运行实例时只保存绑定。明确编辑的离线恢复项会被保留到下次会话快照，组集合始终独立保存，不受实例会话过滤影响。

工作区文档写入版本 5，沿用 `workspace.v1.dpapi`、Windows 当前用户 DPAPI 和原子替换；兼容读取 v1–v4。组保存在顶层 `approvalRuleGroups`；实例引用通过 configRef 关联，Bypass 仍保存在实例 `bypass` 字段。恢复新实例时在绑定前加载引用组和自身 Bypass，不重放任何操作。非 Windows 工作区仍仅内存保存，无明文回退。

旧独立策略迁移成逐实例独立组，不因内容相同自动合并。迁移组使用旧 configRef 作为组命名空间中的稳定 ID，保证首次写 v5 前重复读取一致；名称从实例名派生并消歧。默认正向空规则迁移为未分配；反向空规则保留其放行语义。v1 全局策略先分别复制至旧实例再迁移，旧 allowedOperations 仅兼容读取、不产生豁免。v5 不再写每项 approvalPolicy 或顶层全局策略。旧数据缺少 Bypass 时默认 false。

Bypass 由用户通过当前实例入口主动开启，保存成功再更新运行时；临时实例没有保存项时仅保留内存开关，销毁后丢弃。组更新、分配、解除绑定均不改变 Bypass。已等待审批的提案不因任何规则或开关修改而自动执行。未知运行实例返回 `driver_instance_not_found`，过期查询或保存响应不能覆盖另一个实例。

接口没有新增依赖、系统权限或进程启动方式；schema 扩展可被旧宿主忽略，新宿主对未声明的旧驱动继续要求正常审批。

## 验证入口

- [策略模块](../../crates/coterm-core/src/approval_policy.rs)：正反向、列表、多行、组合文本、无效/超限规则、脱敏操作、缺失/错误字段。
- [真实进程测试](../../tests/fixtures/runtime-driver/tests/lifecycle.rs)与 [SSH 回归](../../drivers/driver-ssh/tests/ssh.rs)：实际派发、等待审批、来源约束与旧提案保持。
- [工作区测试](../../apps/desktop/src-tauri/src/workspace_tests.rs)：同驱动规则隔离、旧记录逐项迁移、加密恢复、稳定身份绑定、Bypass 恢复、临时实例不落盘和写盘失败回滚。
- [浏览器测试](../../tests/approval-settings-browser.test.mjs)：规则追加/删除、两种模式、失败保留、宿主试匹配、双实例规则与 Bypass、切换后的过期响应隔离和中文界面。
