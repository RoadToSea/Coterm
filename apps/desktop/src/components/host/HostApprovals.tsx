import { useEffect, useRef, useState } from "react";
import { hostAPI, type InstanceInfo, type PluginSummary } from "../../host-api";
import type { OperationRecord } from "@coterm/plugin-sdk";
import { ApprovalCard } from "./ApprovalCard";

/** Common approval fallback also works for inactive instances and headless plugins. */
export function HostApprovals() {
  const [pending, setPending] = useState<{ instance?: InstanceInfo; plugin?: PluginSummary; record: OperationRecord } | null>(null);
  const [pollingError, setPollingError] = useState("");
  const revision = useRef(0);
  useEffect(() => {
    let disposed = false;
    let timer: number | undefined;
    const poll = async () => {
      const startedAt = revision.current;
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
        if (!disposed && startedAt === revision.current) {
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
  const decided = (operationId: string) => {
    // A poll started before the decision must not reopen a stale proposal.
    revision.current++;
    setPending((current) => current?.record.operationId === operationId ? null : current);
  };
  if (!pending && !pollingError) return null;
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-20 flex flex-col items-center gap-2 px-4 pb-4 sm:px-6">
      {pollingError && <p role="alert" className="pointer-events-auto w-full max-w-[760px] rounded-lg border border-[#65433d] bg-[#332626] px-3 py-2 text-xs text-[#f48771]">{pollingError}</p>}
      {pending && <ApprovalCard
        key={pending.record.operationId}
        record={pending.record}
        targetLabel={pending.instance?.label ?? pending.plugin?.name ?? pending.record.pluginId}
        onApprove={async (args) => {
          const proposalId = pending.record.proposalId;
          if (!proposalId) return;
          if (pending.instance) await hostAPI.approveProposal(pending.instance.pluginInstanceId, proposalId, args);
          else if (pending.plugin) await hostAPI.approvePluginProposal(pending.plugin.id, proposalId, args);
          decided(pending.record.operationId);
        }}
        onReject={async () => {
          const proposalId = pending.record.proposalId;
          if (!proposalId) return;
          if (pending.instance) await hostAPI.rejectProposal(pending.instance.pluginInstanceId, proposalId);
          else if (pending.plugin) await hostAPI.rejectPluginProposal(pending.plugin.id, proposalId);
          decided(pending.record.operationId);
        }}
      />}
    </div>
  );
}
