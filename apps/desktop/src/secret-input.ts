import type { InteractionOptions, InteractionOutcome, SecretHandle, SelectionOption } from "../../shared/index";
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import type { InputKind } from "./components/host/SecretPrompt";

const pending = new Map<string, () => void>();
export function cancelSecretInput(key: string): void { pending.get(key)?.(); }


/** Host-owned prompts, bounded and cancelled with the frontend scope. */
function prompt<T>(key: string, request: InteractionOptions, kind: InputKind,
  deliver: (value: string) => Promise<T>): Promise<InteractionOutcome<T>> {
  cancelSecretInput(key);
  if (pending.size >= 16 || typeof HTMLDialogElement === "undefined") return Promise.resolve({ outcome: "unavailable" });
  if (Array.isArray(kind) && (!kind.length || kind.length > 256
    || new Set(kind.map((option) => option.value)).size !== kind.length)) return Promise.resolve({ outcome: "unavailable" });
  return new Promise((resolve) => {
    const container = document.createElement('div');
    document.body.append(container);
    const root = createRoot(container);
    let settled = false;
    let submitting = false;
    const finish = (result: InteractionOutcome<T>) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      pending.delete(key);
      queueMicrotask(() => { root.unmount(); container.remove(); });
      resolve(result);
    };
    const duration = typeof request.timeoutMs === 'number' && Number.isFinite(request.timeoutMs)
      ? Math.min(Math.max(request.timeoutMs, 1000), 300_000) : 60_000;
    const timer = window.setTimeout(() => finish({outcome:'timed_out'}), duration);
    pending.set(key, () => finish({outcome:'cancelled'}));
    void import("./components/host/SecretPrompt").then(({ SecretPrompt }) => {
      if (settled) return;
      root.render(createElement(SecretPrompt, { request, kind,
      cancel: () => finish({outcome:'cancelled'}), reject: () => finish({outcome:'rejected'}),
      unavailable: () => finish({outcome:'unavailable'}),
      deliver: async (value: string) => {
        if (settled || submitting) return;
        submitting = true;
        try { finish({outcome:'submitted',value:await deliver(value)}); }
        finally { submitting = false; }
      },
      }));
    }).catch(() => finish({ outcome: "unavailable" }));
  });

}

/** The frontend receives only a short-lived handle, never the password. */
export function promptForSecret<T = SecretHandle>(instanceId: string,
  request: InteractionOptions & { purpose: string },
  deliver: (instanceId: string, purpose: string, value: string) => Promise<T>,
  cancellationKey = instanceId): Promise<InteractionOutcome<T>> {
  return prompt(cancellationKey, request, "password", (value) => deliver(instanceId, request.purpose, value));
}
export const promptForText = (key: string, request: InteractionOptions) =>
  prompt(key, request, "text", async (value) => value);
export const promptForConfirmation = (key: string, request: InteractionOptions) =>
  prompt(key, request, "confirmation", async () => true);
export const promptForSelection = (key: string, request: InteractionOptions & { options: SelectionOption[] }) =>
  prompt(key, request, request.options, async (value) => value);
