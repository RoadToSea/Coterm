import type { OperationReceipt, PluginAPI } from "@coterm/plugin-sdk";

export interface CommandRequest {
  commands: string[];
  lineEnding: string;
  intervalMs: number;
}

export interface CommandResult {
  total: number;
  sent: number;
  bytes: number;
  remaining: number;
  deviceCompletion: "unknown";
}

/** 空白行只作为编辑时的分隔，不发送额外的空命令。 */
export function commandList(source: string, lineEnding: string, intervalMs: number): CommandRequest {
  const commands = source.split(/\r?\n/).filter((line) => line.trim().length > 0);
  if (commands.length === 0 || commands.length > 64) throw new Error("Provide 1 to 64 commands, one per line.");
  if (!["cr", "lf", "crlf"].includes(lineEnding)) throw new Error("Choose CR, LF, or CRLF for commands.");
  if (!Number.isInteger(intervalMs) || intervalMs < 0 || intervalMs > 1000) throw new Error("Command interval must be between 0 and 1000 ms.");
  const encoder = new TextEncoder();
  let bytes = 0;
  for (const line of commands) {
    const size = encoder.encode(line).length;
    if (size > 4096 || /[\u0000-\u0008\u000a-\u001f\u007f-\u009f]/.test(line)) {
      throw new Error("Each command must be a single line up to 4096 UTF-8 bytes without control characters.");
    }
    bytes += size + (lineEnding === "crlf" ? 2 : 1);
  }
  if (bytes > 16384) throw new Error("Command list exceeds 16384 bytes including line endings.");
  return { commands, lineEnding, intervalMs };
}

const pending = new Set(["accepted", "awaiting_approval", "dispatched", "running"]);

/** 等待宿主账本的真实终态；事件丢失或完成早于 invoke 返回都不会悬挂 UI。 */
export async function waitForCommands(api: Pick<PluginAPI, "getOperation">,
  receipt: OperationReceipt<CommandResult>, signal: AbortSignal): Promise<OperationReceipt<CommandResult>> {
  while (pending.has(receipt.status)) {
    await new Promise<void>((resolve, reject) => {
      if (signal.aborted) { reject(new Error("Command view closed")); return; }
      const cancel = () => { clearTimeout(timer); reject(new Error("Command view closed")); };
      const timer = setTimeout(() => { signal.removeEventListener("abort", cancel); resolve(); }, 200);
      signal.addEventListener("abort", cancel, { once: true });
    });
    const record = await api.getOperation(receipt.operationId);
    receipt = { ...record, output: record.output as CommandResult | null };
  }
  return receipt;
}
