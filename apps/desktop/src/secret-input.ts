import type { InteractionOptions, InteractionOutcome, SecretHandle } from "./plugin-api";

const pending = new Map<string, () => void>();

export function cancelSecretInput(instanceId: string): void {
  pending.get(instanceId)?.();
}

/** Host-owned password dialog: the plugin frontend receives only a short-lived handle. */
export function promptForSecret<T = SecretHandle>(
  instanceId: string,
  request: InteractionOptions & { purpose: string },
  deliver: (instanceId: string, purpose: string, value: string) => Promise<T>,
  cancellationKey = instanceId,
): Promise<InteractionOutcome<T>> {
  cancelSecretInput(cancellationKey);
  return new Promise((resolve) => {
    const dialog = document.createElement("dialog");
    dialog.className = "host-secret-dialog";
    const form = document.createElement("form");
    const title = document.createElement("h2");
    title.textContent = request.title;
    const label = document.createElement("label");
    const caption = document.createElement("span");
    caption.textContent = request.prompt;
    const input = document.createElement("input");
    input.type = "password";
    input.autocomplete = "off";
    input.autofocus = true;
    label.append(caption, input);
    const error = document.createElement("p");
    error.setAttribute("role", "alert");
    const actions = document.createElement("div");
    const submit = document.createElement("button");
    submit.type = "submit";
    submit.textContent = "Continue";
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.textContent = "Cancel";
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
      pending.delete(cancellationKey);
      input.value = "";
      dialog.close();
      dialog.remove();
      resolve(result);
    };
    const duration = Math.min(Math.max(request.timeoutMs ?? 60_000, 1000), 300_000);
    const timer = window.setTimeout(() => finish({ outcome: "timed_out" }), duration);
    pending.set(cancellationKey, () => finish({ outcome: "cancelled" }));
    cancel.addEventListener("click", () => finish({ outcome: "cancelled" }));
    cancel.hidden = request.allowCancel === false;
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      if (request.allowCancel !== false) finish({ outcome: "cancelled" });
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      if (busy || settled) return;
      busy = true;
      submit.disabled = true;
      try {
        const delivery = deliver(instanceId, request.purpose, input.value);
        input.value = "";
        const handle = await delivery;
        finish({ outcome: "submitted", value: handle });
      } catch {
        if (!settled) {
          busy = false;
          submit.disabled = false;
          error.textContent = "Unable to deliver the credential. Try again or cancel.";
        }
      }
    });
    dialog.showModal();
  });
}
