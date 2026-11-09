import { useEffect, useRef, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { hostAPI, type ApprovalGroupDraft } from "../../host-api";
import { t } from "../../i18n";
import { Button } from "../ui/button";
import { Field, FieldGroup, FieldLabel } from "../ui/field";
import { ToggleGroup, ToggleGroupItem } from "../ui/toggle-group";

export const approvalInputClass = "w-full min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";

/** Converts host configuration errors to user text; no submitted command is logged. */
export function approvalError(reason: unknown): string {
  if (typeof reason === "object" && reason !== null && "code" in reason) {
    const code = String(reason.code);
    const translated = t(code);
    if (translated !== code) return translated;
  }
  return typeof reason === "object" && reason !== null && "message" in reason ? String(reason.message) : String(reason);
}

interface Props {
  draft: ApprovalGroupDraft;
  onChange(draft: ApprovalGroupDraft): void;
  custom: string;
  onCustomChange(text: string): void;
  busy: boolean;
  onError(message: string): void;
}

/** Edits a local group draft and previews it through the host; never saves or executes. */
export function ApprovalGroupEditor({ draft, onChange, custom, onCustomChange, busy, onError }: Props) {
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<boolean | null>(null);
  const [testing, setTesting] = useState(false);
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  useEffect(() => { setPreview(null); }, [draft, text]);
  const add = () => {
    if (!custom.trim() || draft.patterns.includes(custom)) return;
    onChange({ ...draft, patterns: [...draft.patterns, custom] }); onCustomChange("");
  };
  const test = async () => {
    if (testing || busy) return;
    const submitted = draft;
    setTesting(true); setPreview(null);
    try {
      const result = await hostAPI.previewApprovalPolicy({ mode: submitted.mode, patterns: submitted.patterns }, text);
      if (live.current) setPreview(result);
    }
    catch (reason) { if (live.current) onError(approvalError(reason)); }
    finally { if (live.current) setTesting(false); }
  };
  return <FieldGroup disabled={busy || testing} aria-busy={busy || testing}>
    <Field>
      <FieldLabel htmlFor="approval-group-name">{t("Group name")}</FieldLabel>
      <input id="approval-group-name" value={draft.name} maxLength={64} className={approvalInputClass}
        onChange={event => onChange({ ...draft, name: event.target.value })} />
    </Field>
    <Field>
      <span id="approval-mode-label" className="text-sm font-medium">{t("Matching mode")}</span>
      <ToggleGroup type="single" value={draft.mode} variant="outline" aria-labelledby="approval-mode-label" className="flex-wrap justify-start"
        onValueChange={value => { if (value === "allow_matches" || value === "approve_matches") onChange({ ...draft, mode: value }); }}>
        <ToggleGroupItem value="allow_matches">{t("Matching commands skip approval")}</ToggleGroupItem>
        <ToggleGroupItem value="approve_matches">{t("Matching commands require approval")}</ToggleGroupItem>
      </ToggleGroup>
      <p className="text-xs text-muted-foreground">{t(draft.mode === "allow_matches" ? "Unmatched commands require approval." : "Unmatched commands skip approval.")}</p>
      {draft.patterns.length === 0 && <p role="status" className="text-xs text-muted-foreground">{t(draft.mode === "allow_matches"
        ? "Empty group: all commands require approval." : "Empty group: all commands eligible for matching skip approval.")}</p>}
    </Field>
    <Field>
      <FieldLabel htmlFor="approval-pattern">{t("Regular expression rules")}</FieldLabel>
      {draft.patterns.map((pattern, index) => <div key={index} className="flex min-w-0 items-center gap-2">
        <input aria-label={t("Rule {number}", { number: index + 1 })} value={pattern} spellCheck={false} maxLength={2048}
          className={approvalInputClass} onChange={event => onChange({ ...draft, patterns: draft.patterns.map((item, position) => position === index ? event.target.value : item) })} />
        <Button type="button" variant="ghost" size="icon" aria-label={t("Remove rule {number}", { number: index + 1 })}
          onClick={() => onChange({ ...draft, patterns: draft.patterns.filter((_, position) => position !== index) })}><Trash2 /></Button>
      </div>)}
      {!draft.patterns.length && <p className="text-xs text-muted-foreground">{t("No rules yet.")}</p>}
      <input id="approval-pattern" value={custom} spellCheck={false} maxLength={2048} placeholder="^pwd$" className={approvalInputClass}
        onChange={event => onCustomChange(event.target.value)} onKeyDown={event => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); add(); }
        }} />
      <Button type="button" variant="outline" size="sm" className="self-start" disabled={!custom.trim() || draft.patterns.length >= 128 || draft.patterns.includes(custom)} onClick={add}>
        <Plus data-icon="inline-start" />{t("Add rule")}
      </Button>
    </Field>
    <Field>
      <FieldLabel htmlFor="approval-preview">{t("Test a command")}</FieldLabel>
      <textarea id="approval-preview" rows={2} value={text} maxLength={65536} className={approvalInputClass} onChange={event => setText(event.target.value)} />
      <Button type="button" variant="outline" size="sm" className="self-start" disabled={!text || !!custom.trim()} onClick={() => void test()}>{t("Test matching")}</Button>
      <p role="status" className="text-sm">{preview === null ? "" : t(preview ? "Requires human approval" : "Skips human approval")}</p>
    </Field>
  </FieldGroup>;
}
