import { useRef, useState } from "react";
import type { OperationRecord } from "@coterm/plugin-sdk";

interface ApprovalCardProps {
  record: OperationRecord;
  targetLabel: string;
  onApprove(args: unknown): Promise<void>;
  onReject(): Promise<void>;
}

/** A single string argument can be edited directly; other shapes retain JSON editing. */
function createEditor(record: OperationRecord) {
  const output = record.output;
  const args: unknown = output && typeof output === "object" && "args" in output
    ? output.args : record.input;
  if (args && typeof args === "object" && !Array.isArray(args)) {
    const fields = Object.entries(args).filter(([key]) => key !== "pluginInstanceId");
    const [key, value] = fields[0] ?? [];
    if (fields.length === 1 && typeof value === "string" && !/[\r\n]/.test(value)) {
      return { value, json: false, parse: (text: string): unknown => ({ ...args, [key]: text }) };
    }
  }
  return { value: JSON.stringify(args ?? {}), json: true, parse: (text: string): unknown => JSON.parse(text) };
}

/** Non-modal approval surface. Keyboard actions stay within the card's focus. */
export function ApprovalCard({ record, targetLabel, onApprove, onReject }: ApprovalCardProps) {
  const [editor] = useState(() => createEditor(record));
  const [draft, setDraft] = useState(editor.value);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const deciding = useRef(false);

  const decide = async (approve: boolean) => {
    if (deciding.current) return;
    deciding.current = true;
    setBusy(true);
    setError("");
    try {
      if (approve) await onApprove(editor.parse(draft));
      else await onReject();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      deciding.current = false;
      setBusy(false);
    }
  };

  return (
    <form
      aria-label="Agent operation approval"
      aria-busy={busy}
      className="approval-card pointer-events-auto w-full rounded-t-[32px] border border-b-0 border-[#4c4c4c] bg-[#252526] px-5 pb-4 pt-4 text-[#cccccc] shadow-[0_-6px_24px_rgb(0_0_0/18%)]"
      onSubmit={(event) => { event.preventDefault(); void decide(true); }}
      onKeyDown={(event) => {
        if (event.key === "Enter" && (event.repeat || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229)) {
          event.preventDefault();
        }
        if (event.key === "Escape" && !event.nativeEvent.isComposing) {
          event.preventDefault();
          event.stopPropagation();
          void decide(false);
        }
      }}
    >
      <label htmlFor="approval-command" className="sr-only">{editor.json ? "Operation arguments JSON" : "Command or argument"}</label>
      <input
        id="approval-command"
        type="text"
        value={draft}
        readOnly={busy}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={!!error}
        aria-describedby={error ? "approval-target approval-error" : "approval-target"}
        onChange={(event) => { setDraft(event.target.value); setError(""); }}
        className="h-10 w-full min-w-0 rounded-md border border-[#454545] bg-[#1e1e1e] px-3 font-mono text-[13px] outline-none focus:border-[#007fd4]"
      />
      <div className="mt-3 flex min-w-0 flex-wrap items-center justify-end gap-2">
        <span id="approval-target" title={`${targetLabel} · ${record.operation}`} className="mr-auto min-w-0 flex-1 truncate text-[11px] text-[#a8a8a8]">
          {targetLabel} · {record.operation}
        </span>
        <button type="submit" disabled={busy} className="inline-flex h-8 items-center gap-3 rounded-md bg-[#0e639c] px-4 text-xs text-white transition-colors hover:bg-[#1177bb] active:bg-[#094f80] disabled:opacity-50">
          {busy ? "Working…" : "Approve"}<kbd className="font-sans text-[10px] opacity-70">Enter</kbd>
        </button>
        <button type="button" disabled={busy} onClick={() => void decide(false)} className="inline-flex h-8 items-center gap-3 rounded-md border border-[#4c4c4c] px-4 text-xs transition-colors hover:bg-[#383838] active:bg-[#454545] disabled:opacity-50">
          Reject<kbd className="font-sans text-[10px] text-[#969696]">Esc</kbd>
        </button>
      </div>
      {error && <p id="approval-error" role="alert" className="mt-2 break-words text-xs text-[#f48771]">{error}</p>}
    </form>
  );
}
