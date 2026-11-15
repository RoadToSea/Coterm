import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useRef, useState } from "react";
import type { OperationRecord } from "../../../../shared/index";

interface ApprovalCardProps {
  record: OperationRecord;
  targetLabel: string;
  idPrefix?: string;
  onApprove(args: unknown): Promise<void>;
  onReject(): Promise<void>;
}

/** A single string argument can be edited directly; other shapes retain JSON editing. */
function createEditor(record: OperationRecord) {
  const output = record.output;
  const args: unknown = output && typeof output === "object" && "args" in output
    ? output.args : record.input;
  if (args && typeof args === "object" && !Array.isArray(args)) {
    const fields = Object.entries(args).filter(([key]) => key !== "driverInstanceId");
    const [key, value] = fields[0] ?? [];
    if (fields.length === 1 && typeof value === "string" && !/[\r\n]/.test(value)) {
      return { value, json: false, parse: (text: string): unknown => ({ ...args, [key]: text }) };
    }
  }
  return { value: JSON.stringify(args ?? {}), json: true, parse: (text: string): unknown => JSON.parse(text) };
}

/** Non-modal approval surface. Keyboard actions stay within the card's focus. */
export function ApprovalCard({ record, targetLabel, idPrefix = "approval-command", onApprove, onReject }: ApprovalCardProps) {
  useI18n();
  const [editor] = useState(() => createEditor(record));
  const [draft, setDraft] = useState(editor.value);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const deciding = useRef(false);
  const commandId = `${idPrefix}-${record.operationId}`;
  const targetId = `${commandId}-target`;
  const errorId = `${commandId}-error`;

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
      aria-label={t("Agent operation approval")}
      aria-busy={busy}
      className="approval-card pointer-events-auto w-full rounded-t-lg border border-b-0 border-coterm-border-input bg-coterm-surface-raised px-5 pb-4 pt-4 text-coterm-text shadow-up"
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
      <label htmlFor={commandId} className="sr-only">{editor.json ? t("Operation arguments JSON") : t("Command or argument")}</label>
      <Input
        id={commandId}
        type="text"
        value={draft}
        readOnly={busy}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={!!error}
        aria-describedby={error ? `${targetId} ${errorId}` : targetId}
        onChange={(event) => { setDraft(event.target.value); setError(""); }}
        className="h-10 font-mono text-[13px]"
      />
      <div className="mt-3 flex min-w-0 flex-wrap items-center justify-end gap-2">
        <span id={targetId} title={`${targetLabel} · ${record.operation}`} className="mr-auto min-w-0 flex-1 truncate text-[11px] text-coterm-text-subtle">
          {targetLabel} · {record.operation}
        </span>
        <Button type="submit" size="sm" disabled={busy}>
          {busy ? t("Working…") : t("Approve")}<kbd className="rounded-sm border border-current/20 bg-muted/30 px-1 py-0.5 font-sans text-[10px] opacity-70">Enter</kbd>
        </Button>
        <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => void decide(false)}>
          {t("Reject")}<kbd className="rounded-sm border border-current/20 bg-muted/30 px-1 py-0.5 font-sans text-[10px] text-coterm-text-muted">Esc</kbd>
        </Button>
      </div>
      {error && <p id={errorId} role="alert" className="mt-2 break-words text-xs text-destructive">{error}</p>}
    </form>
  );
}
