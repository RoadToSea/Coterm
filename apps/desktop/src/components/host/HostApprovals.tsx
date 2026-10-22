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
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-20 flex max-h-full flex-col items-center gap-2 overflow-y-auto">
      {notice && <p role="alert" className="pointer-events-auto w-full max-w-[760px] rounded-lg border border-[#65433d] bg-[#332626] px-3 py-2 text-xs text-[#f48771]">{notice}</p>}
      {pending.map((item) => item.record ? (
        <ApprovalCard
          key={item.operationId}
          record={item.record}
          targetLabel={item.targetLabel}
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
        <div key={item.operationId} className="pointer-events-auto flex w-full max-w-[760px] flex-wrap items-center gap-2 rounded-t-[32px] border border-b-0 border-[#4c4c4c] bg-[#252526] px-5 py-3 text-xs text-[#cccccc]">
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
      ))}
    </div>
  );
}
