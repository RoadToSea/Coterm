import type { InteractionOptions, InteractionOutcome, SecretHandle, SelectionOption } from "@coterm/plugin-sdk";

const pending = new Map<string, () => void>();
export function cancelSecretInput(key: string): void { pending.get(key)?.(); }
type InputKind = "password" | "text" | "confirmation" | SelectionOption[];

/** Host-owned prompts, bounded and cancelled with the frontend scope. */
function prompt<T>(key: string, request: InteractionOptions, kind: InputKind,
  deliver: (value: string) => Promise<T>): Promise<InteractionOutcome<T>> {
  cancelSecretInput(key);
  if (pending.size >= 16 || typeof HTMLDialogElement === "undefined") return Promise.resolve({ outcome: "unavailable" });
  if (Array.isArray(kind) && (!kind.length || kind.length > 256
    || new Set(kind.map((option) => option.value)).size !== kind.length)) return Promise.resolve({ outcome: "unavailable" });
  return new Promise((resolve) => {
    const dialog = document.createElement("dialog");
    dialog.className = "host-secret-dialog";
    const form = document.createElement("form");
    const title = document.createElement("h2");
    title.textContent = request.title;
    const label = document.createElement("label");
    const caption = document.createElement("span");
    caption.textContent = request.prompt;
    const input = Array.isArray(kind) ? document.createElement("select") : document.createElement("input");
    if (input instanceof HTMLInputElement) {
      input.type = kind === "password" ? "password" : "text";
      input.autocomplete = "off";
      input.maxLength = 65_536;
    } else if (Array.isArray(kind)) {
      for (const option of kind) {
        const item = document.createElement("option");
        item.value = option.value;
        item.textContent = option.label;
        input.append(item);
      }
    }
    input.autofocus = true;
    label.append(caption);
    if (kind !== "confirmation") label.append(input);
    const error = document.createElement("p");
    error.setAttribute("role", "alert");
    const actions = document.createElement("div");
    const submit = document.createElement("button");
    submit.type = "submit";
    submit.textContent = kind === "confirmation" ? "Confirm" : "Continue";
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.textContent = "Cancel";
    if (kind === "confirmation") {
      const reject = document.createElement("button");
      reject.type = "button";
      reject.textContent = "Reject";
      reject.addEventListener("click", () => finish({ outcome: "rejected" }));
      actions.append(reject);
    }
    actions.append(cancel, submit);
    form.append(title, label, error, actions);
    dialog.append(form);
    document.body.append(dialog);
    let settled = false;
    let busy = false;
    const finish = (result: InteractionOutcome<T>) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      pending.delete(key);
      input.value = "";
      dialog.close();
      dialog.remove();
      resolve(result);
    };
    const duration = typeof request.timeoutMs === "number" && Number.isFinite(request.timeoutMs)
      ? Math.min(Math.max(request.timeoutMs, 1000), 300_000) : 60_000;
    const timer = window.setTimeout(() => finish({ outcome: "timed_out" }), duration);
    pending.set(key, () => finish({ outcome: "cancelled" }));
    cancel.addEventListener("click", () => finish({ outcome: "cancelled" }));
    cancel.hidden = request.allowCancel === false;
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      if (request.allowCancel !== false) finish({ outcome: "cancelled" });
    });
    dialog.addEventListener("close", () => finish({ outcome: "cancelled" }));
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      if (busy || settled) return;
      busy = true;
      submit.disabled = true;
      try {
        const delivery = deliver(input.value);
        input.value = "";
        finish({ outcome: "submitted", value: await delivery });
      } catch {
        if (!settled) {
          busy = false;
          submit.disabled = false;
          error.textContent = "Unable to deliver the response. Try again or cancel.";
        }
      }
    });
    try { dialog.showModal(); }
    catch { finish({ outcome: "unavailable" }); }
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
