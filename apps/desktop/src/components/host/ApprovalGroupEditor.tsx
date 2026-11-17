import { useEffect, useRef, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { hostAPI, type ApprovalGroupDraft } from "../../host-api";
import { t } from "../../i18n";
import { Button } from "../ui/button";
import { Input, Textarea } from "../ui/input";
import { Field, FieldGroup, FieldLabel } from "../ui/field";
import { ToggleGroup, ToggleGroupItem } from "../ui/toggle-group";

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
  busy: boolean;
  onError(message: string): void;
}

/** Edits a local group draft and previews it through the host; never saves or executes. */
export function ApprovalGroupEditor({ draft, onChange, busy, onError }: Props) {
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<boolean | null>(null);
  const [testing, setTesting] = useState(false);
  const newRuleInput = useRef<HTMLInputElement>(null);
  const focusNewRule = useRef(false);
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  useEffect(() => { setPreview(null); }, [draft, text]);
  useEffect(() => {
    if (!focusNewRule.current) return;
    focusNewRule.current = false;
    newRuleInput.current?.focus();
  }, [draft.patterns.length]);
  const add = () => {
    if (draft.patterns.length >= 128) return;
    focusNewRule.current = true;
    onChange({ ...draft, patterns: [...draft.patterns, ""] });
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
      <Input id="approval-group-name" value={draft.name} maxLength={64}
        onChange={event => onChange({ ...draft, name: event.target.value })} />
    </Field>
    <Field>
      <span id="approval-mode-label" className="text-sm font-medium">{t("Matching mode")}</span>
      <ToggleGroup type="single" value={draft.mode} variant="outline" aria-labelledby="approval-mode-label" className="flex-wrap justify-start"
        title={t(draft.mode === "allow_matches" ? "Unmatched commands require approval." : "Unmatched commands skip approval.")}
        aria-description={t(draft.mode === "allow_matches" ? "Unmatched commands require approval." : "Unmatched commands skip approval.")}
        onValueChange={value => { if (value === "allow_matches" || value === "approve_matches") onChange({ ...draft, mode: value }); }}>
        <ToggleGroupItem value="allow_matches">{t("Matching commands skip approval")}</ToggleGroupItem>
        <ToggleGroupItem value="approve_matches">{t("Matching commands require approval")}</ToggleGroupItem>
      </ToggleGroup>
      {draft.patterns.length === 0 && <p role="status" className="text-xs text-muted-foreground">{t(draft.mode === "allow_matches"
        ? "Empty group: all commands require approval." : "Empty group: all commands eligible for matching skip approval.")}</p>}
    </Field>
    <Field>
      <FieldLabel>{t("Regular expression rules")}</FieldLabel>
      {draft.patterns.map((pattern, index) => <div key={index} data-rule-row className="flex min-w-0 items-center gap-2">
        <Input ref={index === draft.patterns.length - 1 ? newRuleInput : undefined} id={`approval-pattern-${index}`}
          aria-label={t("Rule {number}", { number: index + 1 })} aria-invalid={!pattern.trim()} value={pattern} spellCheck={false} maxLength={2048}
          placeholder={t("Enter a regular expression")}
          onChange={event => onChange({ ...draft, patterns: draft.patterns.map((item, position) => position === index ? event.target.value : item) })} />
        <Button type="button" variant="ghost" size="icon" aria-label={t("Remove rule {number}", { number: index + 1 })}
          onClick={() => onChange({ ...draft, patterns: draft.patterns.filter((_, position) => position !== index) })}><Trash2 data-icon="inline-start" /></Button>
      </div>)}
      {!draft.patterns.length && <p className="text-xs text-muted-foreground">{t("No rules yet.")}</p>}
      <Button type="button" variant="outline" size="sm" className="self-start" disabled={draft.patterns.length >= 128} onClick={add}>
        <Plus data-icon="inline-start" />{t("Add rule")}
      </Button>
    </Field>
    <Field>
      <FieldLabel htmlFor="approval-preview">{t("Test a command")}</FieldLabel>
      <Textarea id="approval-preview" rows={2} value={text} maxLength={65536} onChange={event => setText(event.target.value)} />
      <Button type="button" variant="outline" size="sm" className="self-start" disabled={!text || draft.patterns.some(pattern => !pattern.trim())} onClick={() => void test()}>{t("Test matching")}</Button>
      <p role="status" className="text-sm">{preview === null ? "" : t(preview ? "Requires human approval" : "Skips human approval")}</p>
    </Field>
  </FieldGroup>;
}
