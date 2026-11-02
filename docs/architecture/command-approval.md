# Agent 命令审批规则

规则由用户在桌面「Agent 审批」中管理，针对实际提交的命令文本，不按接口名放行。可追加、删除规则，保存后生效；未保存的修改不影响运行时。规则最多 128 条，单条最多 2048 个 UTF-8 字节，总量最多 16 KiB。使用 Rust `regex` 语法，不支持环视、反向引用；编译结果和匹配缓存有内存上限，无效规则拒绝保存。

| 模式 | 命中任一规则 | 未命中规则 |
|---|---|---|
| 正向放行 `allow_matches` | 免审批 | 人工审批 |
| 反向拦截 `approve_matches` | 人工审批 | 免审批 |

“拦截”表示暂停并等待人工批准或拒绝。全新工作区默认正向模式、空规则，因此所有 Agent commit 仍需审批。反向模式的空规则会放行所有参与匹配的命令；模式及其说明在保存前始终可见。

正则默认匹配任意位置；用 `^`、`$` 限制整条单行命令，或 `\A`、`\z` 限制整个原始文本。`^ls$` 仅匹配 `ls`，`\brm\b` 可用于反向模式匹配含 `rm` 的命令。需要简单参数时可以使用 `\Als(?:[ \t]+[^;&|\x60$<>\r\n]+)?\z`。正则只匹配文本，不解析 Shell 语法、别名或被调用脚本的内容；用户决定规则覆盖范围。

命令数组按每个原始列表项匹配，不裁剪、不拆行、不拼接：正向模式要求每项都命中至少一条规则，反向模式有一项命中就让整个提交进入审批。多行脚本始终作为一个完整项匹配。空数组、错误类型、空文本、超过 128 项或合计超过 1 MiB 的输入保留审批，不以宽松匹配回退。

## 宿主与插件契约

插件在 operation 的 `inputSchema` 顶层声明可选扩展 `x-coterm-approval-text`，值为指向实际命令文本或字符串数组的 JSON Pointer，例如 `/text`、`/commands`。声明来自注册目录，调用参数不能覆盖它。宿主在 schema 校验后读取该路径，使用已编译规则判断；不扫描其他参数、配置或秘密，不猜测具体协议。

目前 SSH 内部 command/exec 声明 `/text`，MCP command/exec 声明 `/commands`；串口文本 command 及其 MCP 镜像声明 `/commands`。串口 HEX、底层 write、原始按键、文件写入、连接配置等不声明命令文本，继续使用原审批规则。新插件可以通过同一 schema 扩展接入，不需要新增宿主协议适配器。

只有 Agent 来源的 `effect=commit` 受此门控；Human/System、read/draft 沿用已有行为。未声明、无效指针、缺失或类型错误的字段不能获得规则豁免。`sensitiveInput` 继续控制账本脱敏；例如串口文本命令同时声明命令路径和敏感标记，只在内存匹配，原文不进入普通日志或 MCP 账本。用户主动开启的 Bypass 仍跳过宿主审批；规则与 Bypass 均不跳过插件参数校验、来源限制、协议认证或秘密交互。已等待审批的提案不因修改规则自动执行；后续新提交才使用新规则。

桌面专用命令：

| 命令 | 请求 | 响应 |
|---|---|---|
| `get_approval_policy` | 无 | `{mode, patterns}` |
| `set_approval_policy` | `{policy: {mode, patterns}}` | 保存后的策略；无效正则返回 `approval_rule_invalid` |
| `preview_approval_policy` | `{policy, text}` | `true` 需审批，`false` 免审批；使用同一 Rust matcher，不执行任何插件操作 |

这些命令不导出给插件前端或 MCP。规则与工作区共同保存在 Windows DPAPI 文件中；其他平台仅会话内保存。保存先校验并原子持久化，再更新运行时；写盘失败保留旧策略和编辑内容。旧 `allowedOperations` 仅用于兼容读取，不再产生放行效果，也不转换成正则；旧工作区从默认空规则开始。

接口没有新增依赖、系统权限或进程启动方式；schema 扩展可被旧宿主忽略，新宿主对未声明的旧插件继续要求正常审批。

## 验证入口

- [策略模块](../../crates/coterm-core/src/approval_policy.rs)：正反向、列表、多行、组合文本、无效/超限规则、脱敏操作、缺失/错误字段。
- [真实进程测试](../../tests/fixtures/runtime-plugin/tests/lifecycle.rs)与 [SSH 回归](../../plugins/plugin-ssh/tests/ssh.rs)：实际派发、等待审批、来源约束与旧提案保持。
- [工作区测试](../../apps/desktop/src-tauri/src/workspace_tests.rs)：旧记录迁移、加密恢复和写盘失败回滚。
- [浏览器测试](../../tests/approval-settings-browser.test.mjs)：规则追加/删除、两种模式、失败保留、宿主试匹配和中文界面。
