# SSH / MCP 实际使用记录

日期：2026-10-26。入口：Coterm 桌面本机 Streamable HTTP MCP。

## 任务与边界

通过 Coterm SSH，在用户指定的服务器创建 `liu`，加入 `root` 组，并通过 Docker 部署 Clash Verge 使用的 Mihomo 内核。记录真实操作中的问题，不把源码中的已知限制当作已经复现的问题。

加入 `root` 组不等于 UID 0 或 sudo 权限。部署前检查服务器、现有用户、容器、端口和 Docker 状态；不覆盖现有配置。无订阅或节点信息时，只能验证内核运行和直连代理，不能声称已验证节点代理。

记录不得包含密码、订阅地址、Token、服务器公网地址或主机密钥指纹。

## 实测问题

### MCP 会话满时，新客户端无法开始任务

- 操作：向 `/mcp` 发送标准 `initialize`，声明协议版本 `2025-03-26`，Accept 同时包含 JSON 与 SSE。
- 结果：HTTP 429，JSON-RPC 错误 `-32000`，消息为 `Too many MCP sessions; close an existing session`。尚未取得 session ID，无法调用实例发现或 SSH。
- 使用影响：错误要求关闭已有会话，但新客户端不知道已有会话 ID，也没有可用的会话管理工具；需要人工清理客户端、重启桌面或等待会话过期。
- 源码核对：`crates/coterm-mcp/src/http.rs` 设置最多 8 个会话，空闲 30 分钟过期；初始化时清理过期会话。此处只能确认名额已满，不能仅凭 429 判断是会话泄漏。
- 建议：桌面提供会话数量、最近活动时间、客户端名称和关闭入口；429 返回可操作的恢复提示与重试信息；客户端在任务结束时发送 DELETE 释放自身会话。

### 只读 Shell 检查也必须审批

- 操作：执行 `id`、系统版本、用户和 Docker 状态检查。
- 结果：`ssh.command` 返回 `awaiting_approval`，需要桌面审批后才能取得检查结果。
- 使用影响：Shell 命令统一为 commit；检查与修改使用同一入口，逐条部署会产生审批切换成本。后续调用没有再次停留在审批，不据此判断审批策略或 Bypass 的变更原因；本次未尝试通过 Agent 开启 Bypass。
- 建议：提供插件原生的系统信息、文件元信息、容器状态等有限只读工具；不要依靠命令字符串猜测安全性，也不要让 Agent 自批。

### 结果嵌套和 base64 增加客户端处理成本

- 操作：读取直接完成的命令结果，以及 pending 后的 `coterm.get_operation`。
- 结果：命令数据位于 `content[].text` 中的 JSON 字符串，终端字节在 `data` 内 base64 编码；操作查询还包含 `structuredContent` 与文本副本。直接结果的操作元信息也可能再次携带输出。
- 使用影响：客户端需要多层 JSON 解析、base64 解码和去重，才能提取退出码与实际输出；原始响应明显大于实际命令输出。本次临时客户端增加了解码和展示去重才能顺畅检查。
- 建议：插件提供原生 `structuredContent` 和 schema，保持二进制数据编码但补有界可读文本；收据尽量只包含操作元信息，避免重复大块输出。

### 共享 PTY 的后台作业通知污染下一条输出

- 操作：后台安装 Docker，再通过另一条命令查询日志；镜像拉取也使用相同方式。
- 结果：下一条命令捕获到 `[1]+ Done ...` 作业通知，其中重复整段后台启动命令。
- 使用影响：输出同时包含当前检查与前一次后台任务信息，增加解析负担；本次退出码和完成标记正常，没有据此认定执行失败。
- 建议：增加明确的后台作业 ID、状态、日志游标和退出码；若仍用共享 PTY，明确标记作业通知与命令输出的关系。

### 长任务缺少直接的作业进度入口

- 操作：安装 Docker、拉取镜像。
- 结果：为适应 SSH 命令最长 120 秒的已知限制，主动将任务放入后台，写日志与退出码文件，再查询进度。本次没有实际触发 120 秒超时。短 MCP 等待窗口返回 pending 后，也需要另调操作查询取得完成结果。
- 使用影响：部署流程需要自行实现任务状态和日志读取；`ssh.read` 可以读取终端回放，但不等于独立后台作业管理。
- 建议：插件增加有界、可取消的长任务接口，提供进度、退出码和日志读取；保持客户端等待时间与远端作业生命周期的明确区别。

### 顶层 Shell 语法错误会关闭 SSH 会话

- 操作：在已连接的交互 Shell 中执行 `printf 'BEFORE'; if`，只制造未闭合的 `if` 语法错误。
- 结果：操作返回 `unknown`，错误为 `ssh_connection_lost_before_completion`；随后 `ssh.status` 为 disconnected，回放尾部包含 `-bash: eval: line 2: syntax error: unexpected end of file` 和 `logout`。新的命令在重连前均失败。
- 对照：把同样错误放入子 Shell，即 `(eval 'if'); printf 'SUBSHELL_RECOVERED'`，连接保持可用并返回退出码 0。
- 使用影响：普通命令的语法错误可以杀死共享 PTY，且操作结果无法说明是命令错误还是网络断线；Agent 需要额外读取回放并重建连接。
- 建议：不要在登录 Shell 顶层直接 `eval` 用户命令；为每条命令使用可恢复的子 Shell 或明确隔离的执行上下文。错误结果应保留有界 stderr 摘要，并区分 shell 退出、远端断线和主机密钥失败。

### 取消未确认会永久阻塞当前连接

- 操作：执行 `sleep 20; printf 'SHOULD_NOT_COMPLETE'`，等待 pending 后调用 `coterm.cancel_operation`。
- 结果：取消返回 `operation_cancellation_unconfirmed`、状态 `unknown`。立即执行的新命令返回 `A command is still running or its completion is unknown`；等待超过 20 秒后仍返回相同错误。只有显式 `ssh.disconnect`，再重新连接，才能恢复命令执行。
- 使用影响：取消操作不是可用的终止动作；连接进入需要人工恢复的阻塞状态。操作查询一直保持 unknown，没有提供底层进程是否结束的最终状态。
- 建议：取消失败时应停止跟踪并关闭或重建 PTY，或者提供明确的“连接被隔离，需重连”状态与自动恢复入口；不要让同一连接永久拒绝后续命令。

### 语法错误和连接失败的错误分类过于宽泛

- 操作：在断线后调用 `ssh.probe`，以及在语法错误导致退出后查询操作。
- 结果：探测失败与“主机密钥不匹配”共用 `SSH connection failed or host key did not match`；命令退出则使用 `ssh_connection_lost_before_completion`。网络层在 TCP 建立后无 SSH banner 时也没有暴露更具体的阶段或原因。
- 使用影响：无法仅凭结构化错误决定重试探测、请求用户确认指纹、输入密码，还是重连已经退出的 Shell。
- 建议：至少区分 TCP 连接、SSH 握手、主机密钥校验、认证、PTY 建立和远端 Shell 退出；保留安全的阶段码，不返回凭据或完整网络错误。

### 失败命令的终端回放包含完整包装命令

- 操作：读取 `ssh.read` 游标 0 的回放。
- 结果：回放包含登录横幅、带随机 marker 的完整 `printf ...; eval ...; __coterm_rc=...` 包装命令，以及命令输出。输出中出现的源命令会被重复展示，且包装细节混入用户终端。
- 使用影响：回放不适合作为干净的审计记录或模型上下文；命令参数可能出现在普通终端事件中，尽管本次没有传入秘密。
- 建议：将协议 marker 和包装命令从用户可见回放中隔离；审计记录使用结构化、脱敏后的源命令，并明确是否包含原始输入。

### 输出边界在 8192 字节处截断但没有摘要字段

- 操作：分别输出 8191、8192、8193 字节，并输出包含 NUL、控制字符及非 UTF-8 字节的数据。
- 结果：8191 和 8192 字节完整返回；8193 字节返回 8192 字节并标记 `outputTruncated=true`。二进制字节保留在 base64 中，控制字符不导致协议失败。
- 使用影响：调用方只能知道发生截断，无法知道丢弃字节数或是否存在更多可回放数据；直接把结果当文本会遇到控制字符。
- 建议：结果补充原始长度、丢弃字节数或明确的输出游标；继续保持 base64，避免把二进制强制转换为文本。

## 部署结果与验证

用户清理会话后，重新初始化成功。通过已有、已认证的 root SSH 实例完成以下操作；未另建 SSH 连接，因此本次未覆盖指纹确认或密码交互。

| 项目 | 结果 |
|---|---|
| 服务器环境 | Ubuntu 20.04.4 LTS，Linux x86_64 |
| 用户 | `liu`，UID/GID 1000，家目录 `/home/liu`，Shell `/bin/bash` |
| 附加组 | `root`（GID 0）；未授予 sudo，未加入 docker 组 |
| 认证 | 未设置密码，`passwd -S liu` 为锁定状态；未配置 SSH 密钥 |
| Docker | Ubuntu 软件源 `docker.io` 26.1.3；服务 active / enabled |
| 内核 | `metacubex/mihomo:v1.19.33`；实际运行固定 digest，见下文 |
| 容器 | `mihomo`，running，`unless-stopped` |
| 配置 | 服务器 `/opt/mihomo/config.yaml`，目录 0700、文件 0600，root 所有 |
| 代理 | 宿主 `127.0.0.1:7890/tcp`；容器内 mixed HTTP/SOCKS 端口 |
| 规则 | `MATCH,DIRECT`，无订阅、无节点；未启用管理 API、TUN 或 UDP 端口映射 |
| 资源与日志 | 256 MiB 内存、1 CPU、日志单文件 10 MiB、最多 3 个；移除 Linux capabilities，开启 no-new-privileges |

镜像 digest：`sha256:9211361a3ed85305383288d38c05e85862d6d5a590c10007c37fc059cca82e9c`。上游版本依据：[Mihomo v1.19.33](https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.33)，发布于 2026-10-21。

验证结果：

- 内核 `-t` 配置校验通过；`/mihomo -v` 确认为 v1.19.33 linux amd64。
- 通过 HTTP 代理访问 `http://example.com`：200。
- 通过 HTTP CONNECT 访问 `https://example.com`：200。
- 通过 SOCKS5 hostname 访问 `https://example.com`：200。
- `docker restart mihomo` 后再次通过 HTTPS 代理访问：200。
- Docker inspect 和宿主 `ss` 均确认端口发布在 `127.0.0.1:7890`；内核日志确认请求匹配 DIRECT。未进行公网外部连接测试。
- Docker 提示服务器内核不支持 swap limit，内存限制不包含 swap；未修改宿主内核配置。

## 第二轮边界测试

本轮使用短命令和自动结束的进程，没有修改用户、Docker 或 Mihomo 配置。已显式取消未完成的重连操作并释放 MCP HTTP 会话；远端因语法错误退出和取消测试产生的 PTY 均已断开，容器仍在运行。

## 问题修复记录

日期：2026-10-27。修复分支 `fix/ssh-field-test-findings`，提交 `760aa82`。本节只记录代码与测试层面的处理，不代表已在真实服务器复测；现场复测项见文末清单。

### 逐条对应

| 实测问题 | 修复方式 | 验证 |
|---|---|---|
| MCP 会话满时无法开始任务 | 429 返回 `mcp_sessions_full`、上限、各会话客户端名与空闲时长、`Retry-After: 30` 和两条恢复路径；桌面 MCP 面板列出会话并按 8 位前缀关闭 | `crates/coterm-mcp/tests/http.rs` 新增会话限制用例 |
| 只读 Shell 检查也必须审批 | 新增 `ssh.system_info`、`ssh.file_info`，固定脚本、effect=read、不执行调用方命令 | SSH 集成测试 `read_only_queries_use_fixed_scripts_outside_the_pty` |
| 结果嵌套和 base64 增加处理成本 | 机器副本改为 `structuredContent`，文本块为有界摘要；可读 UTF-8 返回 `text`，二进制/控制字符保留 base64；`_meta` 收据只报输出大小 | 插件 MCP 测试、`crates/coterm-mcp/src/server.rs` 收据与摘要单测 |
| 共享 PTY 后台作业通知污染输出 | 新增 `ssh.exec` 独立 exec 通道，stdout/stderr 分离，不经过共享 PTY | `exec_is_isolated_from_the_shared_pty_and_streams_output` |
| 长任务缺少作业进度入口 | `ssh.exec` 最长 30 分钟、最多 8 个并行作业；`ssh.output` 按游标读取 `done/exitCode/gap/more` | 同上，并通过 `ssh.output` 校验完整输出 |
| 顶层 Shell 语法错误关闭会话 | 执行前用同一 Shell 的 `-n` 静态检查；语法错误返回 `syntaxError=true`、退出码 2，不进入 `eval` | `syntax_errors_are_rejected_without_killing_the_shared_shell`；另在真实 bash PTY 上验证 |
| 取消未确认永久阻塞连接 | 取消后等待 2 秒结束标记；确认则为 `stopped`，否则 `unknown` 并自动重建 PTY；新增 `ssh.reset` | `unconfirmed_cancel_resets_the_terminal_instead_of_blocking_it`、`explicit_reset_abandons_an_unknown_command_and_keeps_exec_available` |
| 错误分类过于宽泛 | 连接错误带阶段码 `tcp/handshake/host_key/auth/channel/pty/shell`；区分 `ssh_remote_shell_exited` 与 `ssh_connection_lost_before_completion` | 代码审查；真实断线场景未复测 |
| 回放包含完整包装命令 | 回显区在进入回放前丢弃；用短提示串识别，兼容 readline 对长行的 CRLF 重绘 | `readline_redraws_are_dropped_and_partial_markers_survive`；真实 PTY 确认回显区含提示串 |
| 8192 字节截断无摘要字段 | 结果新增 `bytes`、`originalBytes`、`droppedBytes`、`limit`，保留 `truncated`/`outputTruncated` | `binary_and_control_bytes_stay_base64_and_report_truncation` |

### 新增接口

- `ssh.exec`：独立 exec 通道，不保留 `cd`/`export`；取消即关闭通道。
- `ssh.output`：读取 exec 作业的有界输出，参数 `operationId`、`stdoutCursor`、`stderrCursor`。
- `ssh.reset`：重建共享 PTY Shell，未完成命令记为 unknown。
- `ssh.system_info`、`ssh.file_info`：固定只读脚本，无审批。
- MCP 事件名 `ssh.output` 改为 `ssh.terminal.output`，避免与作业读取操作同名；前端已同步。

### 真实 PTY 验证

在 WSL 交互式 bash 5.1 上直接写入插件 `Command::new` 生成的真实包装字节，确认：

- 语法错误被拦截，输出 `bash: ... syntax error: unexpected end of file`；
- 结束标记为 `end;2;syntax`；
- 随后 `echo STILL-ALIVE-$?` 正常执行，Shell 未退出；
- 回显区包含 `__coterm` 提示串，可被回放过滤识别。

### 尚未复测

以下项目本轮只有代码与本地测试，没有重新连接真实服务器：

- 真实指纹确认、密码交互与 MFA；
- 真实网络中断后的错误阶段码；
- 跨公网的长连接、keepalive 与 exec 通道并发；
- `ssh.exec` 在 Ubuntu 20.04 目标服务器上的 `$SHELL -c` 行为；
- Docker/Mihomo 部署本身未改动，容器仍按原记录运行。

## 使用与维护

以下命令在服务器执行。配置只提供直连能力；不包含 Clash Verge 桌面 GUI。

```sh
curl --noproxy '' -x http://127.0.0.1:7890 https://example.com
curl --noproxy '' --socks5-hostname 127.0.0.1:7890 https://example.com
docker ps --filter name=^/mihomo$
docker logs --tail 50 mihomo
docker restart mihomo
```

如修改配置，先校验再重启：

```sh
docker exec mihomo /mihomo -t -f /root/.config/mihomo/config.yaml
docker restart mihomo
```

安装与镜像拉取的临时日志保留在服务器 `/var/tmp/coterm-docker-install.log`、`/var/tmp/coterm-mihomo-pull.log`，对应 `.exit` 文件均为 0。未验证宿主重启、订阅节点、UDP、TUN 或服务器外部访问。
