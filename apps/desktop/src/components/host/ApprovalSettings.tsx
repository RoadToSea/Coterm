import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useEffect, useRef, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { hostAPI, type ApprovalPolicy, type InstanceInfo } from "../../host-api";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/button";
import { ToggleGroup, ToggleGroupItem } from "../ui/toggle-group";
import { Alert, AlertDescription } from "../ui/alert";
import { Separator } from "../ui/separator";

const DEFAULT_POLICY: ApprovalPolicy = { mode: "allow_matches", patterns: [] };
const inputClass = "w-full min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";
function errorText(reason: unknown): string {
  return typeof reason === "object" && reason !== null && "message" in reason
    ? String(reason.message) : String(reason);
}

export function ApprovalSettings({ instance, onClose }: { instance: InstanceInfo; onClose: () => void }) {
  useI18n();
  const [policy, setPolicy] = useState<ApprovalPolicy>(DEFAULT_POLICY);
  const [saved, setSaved] = useState<ApprovalPolicy | null>(null);
  const [custom, setCustom] = useState("");
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<boolean | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(true);
  const [retry, setRetry] = useState(0);
  const action = useRef(false);
  const dirty = JSON.stringify(saved) !== JSON.stringify(policy);

  useEffect(() => {
    let disposed = false;
    setBusy(true);
    void hostAPI.getApprovalPolicy(instance.driverInstanceId).then(current => {
      if (!disposed) { setPolicy(current); setSaved(current); setError(""); }
    }).catch(reason => { if (!disposed) setError(errorText(reason)); })
      .finally(() => { if (!disposed) setBusy(false); });
    return () => { disposed = true; };
  }, [retry, instance.driverInstanceId]);

  const update = (next: ApprovalPolicy) => { setPolicy(next); setPreview(null); setError(""); };
  const add = (pattern: string) => {
    if (!pattern.trim() || policy.patterns.includes(pattern)) return;
    update({ ...policy, patterns: [...policy.patterns, pattern] });
    setCustom("");
  };
  const perform = async (task: () => Promise<void>) => {
    if (action.current) return;
    action.current = true;
    setBusy(true);
    setError("");
    try { await task(); } catch (reason) { setError(errorText(reason)); }
    finally { action.current = false; setBusy(false); }
  };

  return <Modal id="approval-settings-dialog" title={t("Agent approval")} onClose={onClose}>
    <p className="mb-4 break-all text-sm font-medium" data-approval-instance={instance.driverInstanceId}>{instance.label}</p>
    {error && <Alert variant="destructive" className="mb-4"><AlertDescription>{error}</AlertDescription></Alert>}
    {!saved ? <Button variant="outline" disabled={busy} onClick={() => setRetry(value => value + 1)}>{t(busy ? "Loading…" : "Retry")}</Button> :
    <fieldset disabled={busy} aria-busy={busy} className="m-0 flex min-w-0 flex-col gap-5 border-0 p-0">
      <section aria-label={t("Matching mode")} className="flex flex-col gap-2">
        <h3 className="text-sm font-medium">{t("Matching mode")}</h3>
        <ToggleGroup type="single" value={policy.mode} variant="outline" className="justify-start flex-wrap"
          onValueChange={value => { if (value === "allow_matches" || value === "approve_matches") update({ ...policy, mode: value }); }}>
          <ToggleGroupItem value="allow_matches">{t("Allow matching commands")}</ToggleGroupItem>
          <ToggleGroupItem value="approve_matches">{t("Intercept matching commands")}</ToggleGroupItem>
        </ToggleGroup>
      </section>
      <section className="flex flex-col gap-3" aria-label={t("Regular expression rules")}>
        <h3 className="text-sm font-medium">{t("Regular expression rules")}</h3>
        {policy.patterns.length === 0 ? <p className="text-xs text-muted-foreground">{t("No rules yet.")}</p> :
          <ul className="flex max-h-52 flex-col gap-2 overflow-y-auto">
            {policy.patterns.map((pattern, index) => <li key={index} className="flex min-w-0 items-center gap-2 rounded-md border border-border p-2">
              <code className="min-w-0 flex-1 break-all text-xs">{pattern}</code>
              <Button variant="ghost" size="icon" type="button" aria-label={t("Remove rule {number}", { number: index + 1 })}
                onClick={() => update({ ...policy, patterns: policy.patterns.filter((_, position) => position !== index) })}><Trash2 /></Button>
            </li>)}
          </ul>}
        <label className="flex flex-col gap-2 text-sm" htmlFor="approval-pattern">{t("Add a regular expression")}
          <input id="approval-pattern" value={custom} onChange={event => setCustom(event.target.value)}
            onKeyDown={event => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); add(custom); } }}
            placeholder="^ls$" spellCheck={false} maxLength={2048} className={inputClass} />
        </label>
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline" size="sm" disabled={!custom.trim() || policy.patterns.length >= 128} onClick={() => add(custom)}><Plus data-icon="inline-start" />{t("Add rule")}</Button>
        </div>
      </section>
      <Separator />
      <section className="flex flex-col gap-3">
        <label className="flex flex-col gap-2 text-sm" htmlFor="approval-preview">{t("Test a command")}
          <textarea id="approval-preview" rows={3} value={text} maxLength={65536}
            onChange={event => { setText(event.target.value); setPreview(null); }} className={inputClass} placeholder="ls -la" />
        </label>
        <Button type="button" variant="outline" size="sm" className="self-start" disabled={!text}
          onClick={() => void perform(async () => { setPreview(await hostAPI.previewApprovalPolicy(policy, text)); })}>{t("Test matching")}</Button>
        <p role="status" className="text-sm">{preview === null ? "" : t(preview ? "Requires human approval" : "Skips human approval")}</p>
      </section>
      <div className="flex items-center justify-between gap-3">
        <p role="status" className="text-xs text-muted-foreground">{t(custom.trim() ? "Add the typed rule before saving." : dirty ? "Unsaved changes" : "Saved")}</p>
        <Button type="button" disabled={!dirty || !!custom.trim()} title={custom.trim() ? t("Add the typed rule before saving.") : undefined}
          onClick={() => void perform(async () => { const stored = await hostAPI.setApprovalPolicy(instance.driverInstanceId, policy); setPolicy(stored); setSaved(stored); })}>{t("Save rules")}</Button>
      </div>
    </fieldset>}
  </Modal>;
}
