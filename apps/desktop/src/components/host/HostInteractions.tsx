import { useEffect, useState } from "react";
import { hostAPI, subscribeHostAttention, type HostAttention } from "../../host-api";
import { cancelSecretInput, promptForSecret } from "../../secret-input";

/** 剩余时间不足时不再弹窗，避免用户输入后才发现宿主已判定超时。 */
const MIN_REMAINING_MS = 5_000;

/** Lives outside the plugin workspace, so inactive/headless instances can prompt. */
export function HostInteractions() {
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    /** 每次弹窗的身份：旧完成回调只允许清理自己，不能动更新的那一项。 */
    let ticket = 0;
    let active: { id: string; instance: string; ticket: number } | null = null;
    /** 本组件已经自行收敛的请求；宿主状态还没跟上时不要立刻重开同一项。 */
    const settled = new Set<string>();

    const owns = (mine: number) => !disposed && active?.ticket === mine;

    const prompt = (attention: HostAttention) => {
      if (disposed || active) return;
      const request = attention.interactions.find(
        (item) => item.remainingMs > MIN_REMAINING_MS && !settled.has(item.interactionId),
      );
      if (!request) return;
      const mine = ++ticket;
      active = { id: request.interactionId, instance: request.pluginInstanceId, ticket: mine };
      void promptForSecret<void>(request.pluginInstanceId, {
        title: request.title,
        prompt: request.prompt,
        timeoutMs: request.remainingMs,
        allowCancel: request.allowCancel,
        purpose: request.kind.kind === "secret" ? request.kind.purpose : "",
      },
        async (_instance, _purpose, value) => hostAPI.respondSecret(request.interactionId, value),
        `backend:${request.interactionId}`,
      ).then(async (result) => {
        // 迟到的结果可能属于已被新交互取代的弹窗：既不能清空新的 active，
        // 也不能把秘密提交给别的实例或重新打开同一项。
        if (!owns(mine)) return;
        settled.add(request.interactionId);
        if (result.outcome !== "submitted") {
          // 过期/取消由宿主收尾；这里只通知，不改队列。
          await hostAPI.respondSecret(request.interactionId, null).catch(() => {});
        }
      }).catch((reason) => { if (owns(mine)) setError(String(reason)); })
        .finally(() => {
          if (!owns(mine)) return;
          active = null;
          // 下一条交互不会产生新的 changed 事件（宿主状态没变），主动补取一次。
          void hostAPI.hostAttention()
            .then((attention) => { if (!disposed && !active) prompt(attention); })
            .catch((reason) => { if (!disposed) setError(`Unable to reload user interactions: ${String(reason)}`); });
        });
    };

    const observe = (attention: HostAttention) => {
      if (disposed) return;
      setError(null);
      // 宿主已不再列出的请求不必继续占位，避免收敛后残留状态挡住同 id 的新请求。
      for (const id of settled) {
        if (!attention.interactions.some((request) => request.interactionId === id)) settled.delete(id);
      }
      if (active && !attention.interactions.some((request) => request.interactionId === active?.id)) {
        // 过期、取消、插件停止或实例销毁：宿主已收敛，弹窗必须跟着关闭。
        cancelSecretInput(`backend:${active.id}`);
        active = null;
      }
      if (!active) prompt(attention);
    };

    void subscribeHostAttention({
      snapshot: observe,
      error: (message) => {
        // 保留当前弹窗与上一次有效状态，只显示可恢复提示。
        if (!disposed) setError(`Unable to follow user interactions: ${message}`);
      },
    }).then((stop) => {
      if (disposed) stop(); else unsubscribe = stop;
    }).catch((reason) => {
      if (!disposed) setError(`Unable to follow user interactions: ${String(reason)}`);
    });

    return () => {
      disposed = true;
      unsubscribe?.();
      if (active) cancelSecretInput(`backend:${active.id}`);
    };
  }, []);
  return error ? <p role="alert" className="px-3 text-xs text-[#f48771]">{error}</p> : null;
}
