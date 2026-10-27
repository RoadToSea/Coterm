import { useEffect, useRef, useState } from "react";
import { hostAPI, subscribeHostAttention, type AttentionApproval } from "../../host-api";
import type { OperationRecord } from "@coterm/plugin-sdk";
import { ApprovalCard } from "./ApprovalCard";

/**
 * 每条待审批记录附带渲染 ApprovalCard 所需的历史记录。
 * 历史读取失败（例如实例刚销毁）时保留条目，只是暂时无法编辑参数。
 */
interface PendingApproval extends AttentionApproval {
  record: OperationRecord | null;
  loadError?: string;
}

async function loadRecord(approval: AttentionApproval): Promise<{ record: OperationRecord | null; loadError?: string }> {
  try {
    const record = approval.pluginInstanceId
      ? await hostAPI.getOperation(approval.pluginInstanceId, approval.operationId)
      : (await hostAPI.recentPluginOperations(approval.pluginId, 100))
        .find((item) => item.operationId === approval.operationId) ?? null;
    return { record };
  } catch (reason) {
    return { record: null, loadError: String(reason) };
  }
}

/** Host-level approval surface: inactive instances and headless plugins share it. */
export function HostApprovals() {
  const [pending, setPending] = useState<PendingApproval[]>([]);
  const [notice, setNotice] = useState("");
  const [expanded, setExpanded] = useState(false);
  const disposed = useRef(false);

  useEffect(() => {
    disposed.current = false;
    const records = new Map<string, OperationRecord | null>();
    let unsubscribe: (() => void) | undefined;
    let timer: number | undefined;

    void subscribeHostAttention({
      snapshot: (attention) => {
        if (disposed.current) return;
        const approvals = attention.approvals;
        setPending((current) => approvals.map((approval) => {
          const previous = current.find((item) => item.operationId === approval.operationId);
          return {
            ...approval,
            record: previous?.record ?? records.get(approval.operationId) ?? null,
            loadError: previous?.loadError,
          };
        }));
        setNotice("");
        // 只为尚无历史记录的条目补取一次；决定后由快照移除。
        const missing = approvals.filter((approval) => !records.has(approval.operationId));
        if (missing.length === 0) return;
        window.clearTimeout(timer);
        timer = window.setTimeout(() => {
          void Promise.all(missing.map(async (approval) => {
            const loaded = await loadRecord(approval);
            if (disposed.current) return;
            records.set(approval.operationId, loaded.record);
            setPending((current) => current.map((item) =>
              item.operationId === approval.operationId
                ? { ...item, record: loaded.record, loadError: loaded.loadError }
                : item));
          }));
        }, 0);
      },
      error: (message) => {
        // 保留上一次有效列表，只提示；清空会让用户误以为待审批已消失。
        if (!disposed.current) setNotice(`Unable to follow Agent approvals: ${message}`);
      },
    }).then((stop) => {
      if (disposed.current) stop(); else unsubscribe = stop;
    }).catch((reason) => {
      if (!disposed.current) setNotice(`Unable to follow Agent approvals: ${String(reason)}`);
    });

    return () => {
      disposed.current = true;
      window.clearTimeout(timer);
      unsubscribe?.();
    };
  }, []);

  const decide = (operationId: string) => {
    setPending((current) => current.filter((item) => item.operationId !== operationId));
  };

  if (pending.length === 0 && !notice) return null;
  const visible = expanded ? pending : pending.slice(0, 1);
  const queued = Math.max(0, pending.length - 1);

  const renderApproval = (item: PendingApproval) => item.record ? (
    <ApprovalCard
      key={item.operationId}
      record={item.record}
      targetLabel={item.targetLabel}
      idPrefix="agent-approval"
      onApprove={async (args) => {
        if (item.pluginInstanceId) {
          await hostAPI.approveProposal(item.pluginInstanceId, item.proposalId, args);
        } else {
          await hostAPI.approvePluginProposal(item.pluginId, item.proposalId, args);
        }
        decide(item.operationId);
      }}
      onReject={async () => {
        if (item.pluginInstanceId) {
          await hostAPI.rejectProposal(item.pluginInstanceId, item.proposalId);
        } else {
          await hostAPI.rejectPluginProposal(item.pluginId, item.proposalId);
        }
        decide(item.operationId);
      }}
    />
  ) : (
    <div key={item.operationId} className="pointer-events-auto flex w-full flex-wrap items-center gap-2 border-t border-[#383838] bg-[#252526] px-5 py-3 text-xs text-[#cccccc]">
      <span className="min-w-0 flex-1 truncate" title={`${item.targetLabel} · ${item.operation}`}>
        {item.targetLabel} · {item.operation}
        {item.loadError && <span className="ml-2 text-[#f48771]">details unavailable: {item.loadError}</span>}
      </span>
      <button
        type="button"
        onClick={async () => {
          try {
            if (item.pluginInstanceId) {
              await hostAPI.rejectProposal(item.pluginInstanceId, item.proposalId);
            } else {
              await hostAPI.rejectPluginProposal(item.pluginId, item.proposalId);
            }
            decide(item.operationId);
          } catch (reason) {
            setNotice(String(reason));
          }
        }}
        className="inline-flex h-8 items-center rounded-md border border-[#4c4c4c] px-4 text-xs transition-colors hover:bg-[#383838]"
      >
        Reject
      </button>
    </div>
  );

  return (
    <section aria-label="Agent approvals" className="approval-tray-shell shrink-0 border-t border-[#454545] bg-[#1e1e1e] shadow-[0_-8px_24px_rgb(0_0_0/18%)]">
      <div className="mx-auto w-full max-w-[900px]">
        <div className="flex min-h-10 items-center gap-3 border-b border-[#383838] px-4 py-2 text-xs text-[#cccccc]">
          <span className="font-medium">Agent approval</span>
          <span className="text-[#969696]">{pending.length} pending</span>
          {queued > 0 && <span className="text-[#d7ba7d]">{queued} queued</span>}
          {pending.length > 1 && <button type="button" aria-expanded={expanded} onClick={() => setExpanded((value) => !value)} className="ml-auto rounded px-2 py-1 text-[#75beff] transition-colors hover:bg-white/10">{expanded ? "Collapse queue" : "Show queue"}</button>}
        </div>
        {notice && <p role="alert" className="mx-4 mt-2 rounded border border-[#65433d] bg-[#332626] px-3 py-2 text-xs text-[#f48771]">{notice}</p>}
        <div className={`approval-queue ${expanded ? "approval-queue-expanded" : ""}`}>
          {visible.map(renderApproval)}
        </div>
      </div>
    </section>
  );
}
