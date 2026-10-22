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
    let active: { id: string; instance: string } | null = null;
    let unsubscribe: (() => void) | undefined;

    const prompt = (attention: HostAttention) => {      if (disposed || active) return;
      const request = attention.interactions.find((item) => item.remainingMs > MIN_REMAINING_MS);
      if (!request) return;
      active = { id: request.interactionId, instance: request.pluginInstanceId };
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
        if (!disposed && result.outcome !== "submitted") {
          // Expiry/cancellation may already have been handled by the host.
          await hostAPI.respondSecret(request.interactionId, null).catch(() => {});
        }
      }).catch((reason) => { if (!disposed) setError(String(reason)); })
        .finally(() => {
          active = null;
          if (disposed) return;
          // 下一条交互不会产生新的 changed 事件（宿主状态没变），主动补取一次。
          void hostAPI.hostAttention()
            .then((attention) => prompt(attention))
            .catch((reason) => { if (!disposed) setError(`Unable to reload user interactions: ${String(reason)}`); });
        });
    };

    const observe = (attention: HostAttention) => {
      if (disposed) return;
      setError(null);
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
      latest = null;
      unsubscribe?.();
      if (active) cancelSecretInput(`backend:${active.id}`);
    };
  }, []);
  return error ? <p role="alert" className="px-3 text-xs text-[#f48771]">{error}</p> : null;
}
