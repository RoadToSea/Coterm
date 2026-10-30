# 串口命令列表虚拟配对验收

日期：2026-10-30。环境：Windows x64，HHD Software Bridged Serial Port，COM1 ↔ COM2，115200 8-N-1，DTR/RTS 置高。

## 验收方式

先独立探测两端双向收发，再执行 `plugins/plugin-serial/tests/virtual_pair.rs`。测试从当前构建复制后端到独立临时插件包，启动真实宿主运行时和插件子进程，插件在 OS 驱动上打开两个实例。结束或失败时关闭运行时、释放端口、删除临时包。

```powershell
$env:COTERM_SERIAL_PAIR_A = "COM1"
$env:COTERM_SERIAL_PAIR_B = "COM2"
$env:COTERM_SERIAL_PAIR_BAUD = "115200"
cargo test -p coterm-plugin-serial --test virtual_pair -- --ignored --nocapture
```

需要虚拟端口的测试默认 ignored，并要求显式配置两端；普通测试不会打开或写入端口。

COM1 → COM2 与交换后的 COM2 → COM1 均完整通过，每次约 9 秒。测试退出后再次独立打开两端并发送探测数据，通过，确认没有遗留端口占用。

## 发现并修复的问题

首次列表测试失败：第一条的 14 字节已经到达对端，但发送方被标记为 `unknown` 并关闭连接。加入不含命令内容的 IO 阶段、错误种类与 OS 错误码后，定位到驱动的 `flush` 返回 Windows 错误码 1。

写端现优先使用驱动 flush；仅在明确不支持时，改为在现有写入超时内查询发送队列，确认其为空后完成当前项。真实 flush 错误、队列查询失败、队列排空超时均保留为错误，不重新发送当前项，也不继续列表。该逻辑同时用于原始字节和命令列表，无新增依赖、权限或协议。

## 已通过的链路断言

| 场景 | 检查内容 |
|---|---|
| 顺序与间隔 | 三条命令逐项送达，UTF-8、shell 字符和 CRLF 原样保留；100 ms 间隔生效；发送字节数准确 |
| 双向字节 | 对端通过原始写入返回包含 `00/80/FF` 的二进制，接收方游标推进并逐字节一致 |
| 取消 | 当前列表停止余下项，已排队列表不提交；等待超过原发送间隔后仍无迟到字节 |
| 输入冲突 | 列表在途时，原始字节与 DTR/RTS 修改被拒绝 |
| MCP 领域审批 | Agent 发起 `mcp.serial.command`，未批准时对端无数据；人工可检查草稿，Agent 查询不含敏感草稿；批准编辑后的快照才发送，MCP 格式结果和完成通知正确 |
| 列表边界 | 64 项可发送；含结束符恰好 16384 字节可跨多个回放块完整读取；后续非法项导致整批拒绝 |
| 队列容量 | 16 个列表在途，第 17 个拒绝；断开使当前及排队操作收敛，不重放剩余命令 |
| 重连回收 | 连续三轮重连只发送新请求；正常断开、进程退出后释放端口 |

## 回归检查

- `cargo test -p coterm-plugin-serial`：26 条单元测试、2 条普通集成测试通过；物理串口和虚拟配对用例默认 ignored，配对用例已按上述方式单独执行。
- `cargo check --workspace`：通过。
- `cargo clippy -p coterm-plugin-serial --all-targets -- -D warnings`：通过。
- `pnpm plugin:serial:build`：更新串口开发包；未修改前端实现。

## 范围

覆盖真实宿主、插件 stdio 控制链路与虚拟串口驱动；MCP 部分验证原生工具映射、审批和结果封装，未启动桌面外部 MCP stdio 适配器。没有执行设备 shell 命令，不替代物理板卡的提示符、设备执行时长、线路质量或硬件流控验收。
