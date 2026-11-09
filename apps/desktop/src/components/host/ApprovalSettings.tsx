import { useEffect, useRef, useState } from "react";
import { hostAPI, type ApprovalSnapshot } from "../../host-api";
import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/button";
import { Alert, AlertDescription } from "../ui/alert";
import { Field, FieldGroup, FieldLabel } from "../ui/field";
import { ApprovalRuleGroups } from "./ApprovalRuleGroups";
import { approvalError, approvalInputClass } from "./ApprovalGroupEditor";

export interface ApprovalTarget { label: string; configRef?: string; driverInstanceId?: string }

/** Assigns one group to a saved instance, including offline entries. Failed writes
 * keep the selection; switching targets remounts the form in the parent. */
export function ApprovalSettings({ instance, onClose }: { instance: ApprovalTarget; onClose(): void }) {
  useI18n();
  const [catalog, setCatalog] = useState<ApprovalSnapshot | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [retry, setRetry] = useState(0);
  const [manager, setManager] = useState(false);
  const [showRules, setShowRules] = useState(false);
  const [closing, setClosing] = useState(false);
  const action = useRef(false);
  const binding = catalog?.instances.find(item => instance.configRef ? item.configRef === instance.configRef : item.instanceId === instance.driverInstanceId);
  const group = catalog?.groups.find(item => item.id === selected);
  const dirty = selected !== saved;
  const close = () => { if (action.current) return; if (dirty) setClosing(true); else onClose(); };
  useEffect(() => {
    let disposed = false;
    void hostAPI.approvalSnapshot().then(next => {
      if (disposed) return;
      setCatalog(next); setError("");
      const entry = next.instances.find(item => instance.configRef ? item.configRef === instance.configRef : item.instanceId === instance.driverInstanceId);
      setSelected(entry?.ruleGroupId ?? null); setSaved(entry?.ruleGroupId ?? null);
    }).catch(reason => { if (!disposed) setError(approvalError(reason)); });
    return () => { disposed = true; };
  }, [retry, instance.configRef, instance.driverInstanceId]);
  const save = async () => {
    if (!binding || action.current) return;
    action.current = true; setBusy(true); setError("");
    try {
      await hostAPI.assignApprovalGroup(binding.configRef, selected);
      setSaved(selected);
      if (closing) onClose();
    } catch (reason) { setError(approvalError(reason)); }
    finally { action.current = false; setBusy(false); }
  };
  const returnFromManager = () => {
    setManager(false);
    void hostAPI.approvalSnapshot().then(next => {
      setCatalog(next);
      if (selected && !next.groups.some(item => item.id === selected)) setSelected(null);
    }).catch(reason => setError(approvalError(reason)));
  };
  if (manager) return <ApprovalRuleGroups initialGroupId={selected} onClose={returnFromManager} />;
  return <Modal id="approval-settings-dialog" title={t("Instance approval")} onClose={close}>
    <p className="mb-4 break-all text-sm font-medium" data-approval-instance={instance.configRef ?? instance.driverInstanceId}>{instance.label}</p>
    {error && <Alert variant="destructive" className="mb-4"><AlertDescription>{error}</AlertDescription></Alert>}
    {!catalog ? <Button variant="outline" onClick={() => setRetry(value => value + 1)}>{t(error ? "Retry" : "Loading…")}</Button> : <FieldGroup disabled={busy} aria-busy={busy}>
      <Field>
        <FieldLabel htmlFor="approval-group-select">{t("Rule group")}</FieldLabel>
        <select id="approval-group-select" value={selected ?? ""} className={approvalInputClass} disabled={!binding}
          onChange={event => { setSelected(event.target.value || null); setShowRules(false); setError(""); }}>
          <option value="">{t("Unassigned")}</option>
          {catalog.groups.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
        </select>
        {!binding && <p role="alert" className="text-sm text-destructive">{t("This instance has no saved configuration to assign a rule group.")}</p>}
        <p role="status" className="text-sm">{t(binding?.bypass ? "Bypass is on: this instance currently skips approval." : !group ? "All Agent commits require approval."
          : group.mode === "allow_matches" ? "Matching commands skip approval; unmatched commands require approval." : "Matching commands require approval; unmatched commands skip approval.")}</p>
      </Field>
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" size="sm" disabled={!group} onClick={() => setShowRules(value => !value)}>{t("View rules")}</Button>
        <Button type="button" variant="outline" size="sm" onClick={() => setManager(true)}>{t("Manage rule groups")}</Button>
      </div>
      {showRules && group && <ul className="flex max-h-48 flex-col gap-2 overflow-auto" aria-label={t("Regular expression rules")}>
        {group.patterns.map((pattern, index) => <li key={index}><code className="break-all text-xs">{pattern}</code></li>)}
        {!group.patterns.length && <li className="text-xs text-muted-foreground">{t("No rules yet.")}</li>}
      </ul>}
      <div className="flex items-center justify-between gap-3 border-t border-border pt-4">
        <p role="status" className="text-xs text-muted-foreground">{t(dirty ? "Unsaved changes" : "Saved")}</p>
        <Button type="button" disabled={!binding || !dirty} onClick={() => void save()}>{t("Save assignment")}</Button>
      </div>
    </FieldGroup>}
    {closing && <Alert className="mt-4"><AlertDescription>
      <p className="mb-3">{t("Save this assignment before closing?")}</p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={busy} onClick={() => void save()}>{t("Save and continue")}</Button>
        <Button size="sm" variant="outline" disabled={busy} onClick={onClose}>{t("Discard and continue")}</Button>
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => setClosing(false)}>{t("Keep editing")}</Button>
      </div>
    </AlertDescription></Alert>}
  </Modal>;
}
