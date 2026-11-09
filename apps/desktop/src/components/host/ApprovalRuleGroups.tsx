import { useEffect, useRef, useState } from "react";
import { ArrowLeft, Copy, Plus, Trash2 } from "lucide-react";
import { hostAPI, type ApprovalGroupDraft, type ApprovalRuleGroup, type ApprovalSnapshot } from "../../host-api";
import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { cn } from "../../lib/utils";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/button";
import { Alert, AlertDescription } from "../ui/alert";
import { Skeleton } from "../ui/skeleton";
import { ApprovalGroupEditor, approvalError, approvalInputClass } from "./ApprovalGroupEditor";

interface Props { onClose(): void; initialGroupId?: string | null }

/** Manages independent saved groups. Writes use host validation; failed saves keep
 * drafts intact. onClose runs only after pending edits are saved or discarded. */
export function ApprovalRuleGroups({ onClose, initialGroupId }: Props) {
  useI18n();
  const [catalog, setCatalog] = useState<ApprovalSnapshot | null>(null);
  const [draft, setDraft] = useState<ApprovalGroupDraft | null>(null);
  const [saved, setSaved] = useState<ApprovalRuleGroup | null>(null);
  const [custom, setCustom] = useState("");
  const [query, setQuery] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [retry, setRetry] = useState(0);
  const [details, setDetails] = useState(!!initialGroupId);
  const [pending, setPending] = useState<(() => void) | null>(null);
  const action = useRef(false);
  const dirty = !!custom || JSON.stringify(draft) !== JSON.stringify(saved);
  const users = catalog?.instances.filter(instance => instance.ruleGroupId === draft?.id && draft?.id) ?? [];
  const canSave = !!draft?.name.trim() && dirty && !custom.trim();

  useEffect(() => {
    let disposed = false;
    void hostAPI.approvalSnapshot().then(next => {
      if (disposed) return;
      setCatalog(next); setError("");
      const selected = next.groups.find(group => group.id === initialGroupId) ?? next.groups[0] ?? null;
      setDraft(selected ? structuredClone(selected) : null); setSaved(selected);
    }).catch(reason => { if (!disposed) setError(approvalError(reason)); });
    return () => { disposed = true; };
  }, [retry, initialGroupId]);

  const attempt = (next: () => void) => {
    if (action.current) return;
    if (dirty) setPending(() => next); else next();
  };
  const select = (group: ApprovalRuleGroup | null) => {
    setSaved(group); setDraft(group ? structuredClone(group) : { id: null, name: "", mode: "allow_matches", patterns: [] });
    setCustom(""); setError(""); setDetails(true);
  };
  const perform = async (task: () => Promise<void>) => {
    if (action.current) return;
    action.current = true; setBusy(true); setError("");
    try { await task(); } catch (reason) { setError(approvalError(reason)); }
    finally { action.current = false; setBusy(false); }
  };
  const save = async () => {
    const continuation: { run?: () => void } = {};
    await perform(async () => {
      if (!draft || !canSave) return;
      const group = await hostAPI.saveApprovalGroup(draft);
      setCatalog(current => current ? { ...current, groups: current.groups.some(item => item.id === group.id)
        ? current.groups.map(item => item.id === group.id ? group : item) : [...current.groups, group] } : current);
      setDraft(group); setSaved(group); setCustom("");
      continuation.run = pending ?? undefined; setPending(null);
    });
    // A pending delete must acquire the action lock after saving releases it.
    continuation.run?.();
  };
  const discard = () => {
    setDraft(saved ? structuredClone(saved) : null); setCustom(""); setError("");
    const next = pending; setPending(null); next?.();
  };
  const remove = () => perform(async () => {
    if (!draft?.id || users.length) return;
    await hostAPI.deleteApprovalGroup(draft.id);
    setCatalog(current => current ? { ...current, groups: current.groups.filter(group => group.id !== draft.id) } : current);
    setSaved(null); setDraft(null); setCustom(""); setDetails(false);
  });

  return <Modal id="approval-rule-groups-dialog" title={t("Approval rule groups")} onClose={() => attempt(onClose)}
    className="w-[960px] max-h-[90dvh]" bodyClassName="p-0">
    {error && <Alert variant="destructive" className="m-4 w-auto"><AlertDescription>{error}</AlertDescription></Alert>}
    {!catalog ? <div className="flex flex-col gap-3 p-5">{error ? <Button variant="outline" onClick={() => setRetry(value => value + 1)}>{t("Retry")}</Button>
      : <><Skeleton className="h-9 w-48" /><Skeleton className="h-48 w-full" /></>}</div> :
    <div className="grid min-h-[440px] md:grid-cols-[220px_minmax(0,1fr)]">
      <aside aria-label={t("Rule groups")} className={cn("flex min-w-0 flex-col gap-3 border-r border-border p-4", details && "max-md:hidden")}>
        <input type="search" value={query} aria-label={t("Search rule groups")} placeholder={t("Search rule groups")} className={approvalInputClass} onChange={event => setQuery(event.target.value)} />
        <nav className="flex max-h-[52dvh] flex-col gap-1 overflow-y-auto" aria-label={t("Rule group list")}>
          {catalog.groups.filter(group => group.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())).map(group => <Button key={group.id} type="button"
            variant={draft?.id === group.id ? "secondary" : "ghost"} className="h-auto justify-start py-3 text-left" disabled={busy} data-approval-group={group.id}
            aria-current={draft?.id === group.id ? "true" : undefined} onClick={() => { if (draft?.id !== group.id) attempt(() => select(group)); else setDetails(true); }}>
            <span className="flex min-w-0 flex-col gap-1"><span className="truncate">{group.name}</span><span className="text-xs font-normal text-muted-foreground">
              {t(group.mode === "allow_matches" ? "Allow matches" : "Intercept matches")} · {t(catalog.instances.some(instance => instance.ruleGroupId === group.id) ? "Assigned" : "Unassigned")}
            </span></span>
          </Button>)}
          {!catalog.groups.length && <p className="py-4 text-sm text-muted-foreground">{t("No rule groups yet. Create one to get started.")}</p>}
          {!!catalog.groups.length && !catalog.groups.some(group => group.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()))
            && <p className="py-4 text-sm text-muted-foreground">{t("No matching rule groups")}</p>}
        </nav>
        <Button type="button" variant="outline" className="mt-auto" disabled={busy} onClick={() => attempt(() => select(null))}><Plus data-icon="inline-start" />{t("New rule group")}</Button>
      </aside>
      <section aria-label={t("Edit rule group")} className={cn("flex min-w-0 flex-col gap-5 p-5", !details && "max-md:hidden")}>
        <Button variant="ghost" size="sm" className="self-start md:hidden" disabled={busy} onClick={() => attempt(() => setDetails(false))}><ArrowLeft data-icon="inline-start" />{t("Rule groups")}</Button>
        {draft ? <>
          <ApprovalGroupEditor key={draft.id ?? "new"} draft={draft} onChange={next => { setDraft(next); setError(""); }} custom={custom} onCustomChange={setCustom} busy={busy} onError={setError} />
          <div className="border-t border-border pt-4 text-xs text-muted-foreground" data-group-users>
            {t(users.length ? "Saving applies to: {names}" : "This group is not assigned to any instance.", { names: users.map(instance => instance.label).join(", ") })}
          </div>
          <div className="flex flex-wrap items-center gap-2 border-t border-border pt-4">
            <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => attempt(() => {
              select(null); setDraft({ ...structuredClone(draft), id: null, name: t("{name} copy", { name: draft.name }) });
            })}><Copy data-icon="inline-start" />{t("Duplicate")}</Button>
            <Button type="button" variant="ghost" size="sm" disabled={busy || !draft.id || !!users.length}
              title={users.length ? t("Reassign or unassign all instances before deleting this group.") : undefined} onClick={() => attempt(() => void remove())}>
              <Trash2 data-icon="inline-start" />{t("Delete group")}
            </Button>
            <Button type="button" variant="ghost" size="sm" className="ml-auto" disabled={busy || !dirty} onClick={discard}>{t("Discard changes")}</Button>
            <Button type="button" size="sm" disabled={busy || !canSave} onClick={() => void save()}>{t("Save group")}</Button>
          </div>
          <p role="status" className="text-xs text-muted-foreground">{t(custom.trim() ? "Add the typed rule before saving." : dirty ? "Unsaved changes" : "Saved")}</p>
        </> : <p className="py-12 text-sm text-muted-foreground">{t("Select a rule group or create one.")}</p>}
      </section>
    </div>}
    {pending && <Alert className="sticky bottom-0 rounded-none border-x-0 border-b-0"><AlertDescription>
      <p className="mb-3">{t("Save changes before leaving this group?")}</p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={busy || !canSave} onClick={() => void save()}>{t("Save and continue")}</Button>
        <Button size="sm" variant="outline" disabled={busy} onClick={discard}>{t("Discard and continue")}</Button>
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => setPending(null)}>{t("Keep editing")}</Button>
      </div>
    </AlertDescription></Alert>}
  </Modal>;
}
