import { useEffect, useState } from "react";
import { hostAPI } from "../../host-api";
import { cancelSecretInput, promptForSecret } from "../../secret-input";

/** Lives outside the plugin workspace, so inactive/headless instances can prompt. */
export function HostInteractions() {
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let active: { id: string; instance: string } | null = null;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const requests = await hostAPI.pendingInteractions();
        if (disposed) return;
        if (active && !requests.some((r) => r.interactionId === active?.id)) {
          cancelSecretInput(`backend:${active.id}`);
        }
        if (!active && requests.length > 0) {
          const request = requests[0];
          active = { id: request.interactionId, instance: request.pluginInstanceId };
          void promptForSecret<void>(request.pluginInstanceId, { ...request, purpose: request.kind.purpose },
            async (_instance, _purpose, value) => hostAPI.respondSecret(request.interactionId, value),
            `backend:${request.interactionId}`,
          ).then(async (result) => {
            if (!disposed && result.outcome !== "submitted") {
              // Expiry/cancellation may already have been handled by the host.
              await hostAPI.respondSecret(request.interactionId, null).catch(() => {});
            }
          }).catch((reason) => { if (!disposed) setError(String(reason)); })
            .finally(() => { active = null; });
        }
      } catch { /* The initial Tauri connection may not be ready. Retry boundedly. */ }
      if (!disposed) timer = window.setTimeout(() => void poll(), 500);
    };
    void poll();
    return () => {
      disposed = true;
      window.clearTimeout(timer);
      if (active) cancelSecretInput(`backend:${active.id}`);
    };
  }, []);
  return error ? <p role="alert" className="px-3 text-xs text-[#f48771]">{error}</p> : null;
}
