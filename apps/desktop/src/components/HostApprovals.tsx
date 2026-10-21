import { useEffect, useState } from "react";
import { hostAPI, type InstanceInfo, type PluginSummary } from "../host-api";
import type { OperationRecord } from "../plugin-api";
import { Modal } from "./Modal";

/** Common approval fallback also works for inactive instances and headless plugins. */
export function HostApprovals() {
  const [pending, setPending] = useState<{ instance?: InstanceInfo; plugin?: PluginSummary; record: OperationRecord } | null>(null);
  const [draft, setDraft] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [pollingError, setPollingError] = useState("");
  useEffect(() => {
    let disposed = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const instances = await hostAPI.listInstances();
        let next: typeof pending = null;
        for (const instance of instances) {
          const records = await hostAPI.recentOperations(instance.pluginInstanceId, 100);
          const record = records.find((r) => r.status === "awaiting_approval" && r.proposalId);
          if (record) { next = { instance, record }; break; }
        }
        if (!next) {
          for (const plugin of await hostAPI.listPlugins()) {
            const records = await hostAPI.recentPluginOperations(plugin.id, 100);
            const record = records.find((r) => r.status === "awaiting_approval" && r.proposalId);
            if (record) { next = { plugin, record }; break; }
          }
        }
        if (!disposed) {
          setPollingError("");
          setPending((current) => current?.record.operationId === next?.record.operationId ? current : next);
        }
      } catch {
        if (!disposed) setPollingError("Unable to load Agent approvals. The request remains blocked; check the desktop connection.");
      }
      if (!disposed) timer = window.setTimeout(() => void poll(), 1000);
    };
    void poll();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, []);
  useEffect(() => {
    const output = pending?.record.output;
    const args = output && typeof output === "object" && "args" in output ? output.args : pending?.record.input;
    setDraft(JSON.stringify(args ?? {}, null, 2));
    setError("");
  }, [pending?.record.operationId]);

  const decide = async (approve: boolean) => {
    if (!pending?.record.proposalId || busy) return;
    setBusy(true);
    setError("");
    try {
      if (approve) {
        const args: unknown = JSON.parse(draft);
        if (pending.instance) await hostAPI.approveProposal(pending.instance.pluginInstanceId, pending.record.proposalId, args);
        else if (pending.plugin) await hostAPI.approvePluginProposal(pending.plugin.id, pending.record.proposalId, args);
      } else {
        if (pending.instance) await hostAPI.rejectProposal(pending.instance.pluginInstanceId, pending.record.proposalId);
        else if (pending.plugin) await hostAPI.rejectPluginProposal(pending.plugin.id, pending.record.proposalId);
      }
      setPending(null);
    } catch (reason) { setError(JSON.stringify(reason)); }
    finally { setBusy(false); }
  };
  if (!pending) return pollingError
    ? <p role="alert" className="shrink-0 px-3 py-2 text-xs text-[#f48771]">{pollingError}</p>
    : null;
  return <Modal title="Agent operation approval" onClose={() => { if (!busy) void decide(false); }}>
    <p className="mb-2 text-sm">{pending.instance?.label ?? pending.plugin?.name}</p>
    <p className="mb-3 font-mono text-xs">{pending.record.operation}</p>
    <label className="mb-2 block text-xs" htmlFor="agent-draft">Review or edit the arguments before sending</label>
    <textarea id="agent-draft" disabled={busy} spellCheck={false} value={draft} onChange={(event) => setDraft(event.target.value)} className="h-48 w-full border border-[#454545] bg-[#1e1e1e] p-3 font-mono text-xs" />
    {error && <p role="alert" className="text-xs text-[#f48771]">{error}</p>}
    <div className="mt-3 flex justify-end gap-2">
      <button disabled={busy} onClick={() => void decide(false)} className="border border-[#454545] px-3 py-1 text-xs">Reject</button>
      <button disabled={busy} onClick={() => void decide(true)} className="bg-[#0e639c] px-3 py-1 text-xs">Approve & send</button>
    </div>
  </Modal>;
}
