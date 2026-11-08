export type ToastTone = "error" | "warning" | "info";
export interface ToastMessage { message: string; tone?: ToastTone; durationMs?: number }
export interface ToastStackOptions {
  durationMs?: number;
  maxVisible?: number;
  ariaLabel?: string;
  closeLabel?: string;
}
export interface ToastStack {
  show(message: ToastMessage): () => void;
  updateLabels(labels: Pick<ToastStackOptions, "ariaLabel" | "closeLabel">): void;
  clear(): void;
  unmount(): void;
}

const css = `
  .coterm-toasts, .coterm-toasts * { box-sizing: border-box; }
  .coterm-toasts { position: absolute; z-index: 10; top: 12px; right: 12px;
    display: grid; gap: 8px; width: min(380px, calc(100% - 24px)); max-height: calc(100% - 24px);
    overflow: auto; pointer-events: none; font: 13px/1.5 var(--coterm-font-sans, "Segoe UI", sans-serif); }
  .coterm-toast { --toast-tone: var(--coterm-accent, #3794ff); display: grid;
    grid-template-columns: 18px minmax(0, 1fr) 24px; gap: 9px; align-items: start;
    padding: 12px; border: 1px solid var(--toast-tone); border-radius: var(--radius, 8px);
    background: var(--coterm-surface-raised, #252526);
    background: color-mix(in srgb, var(--toast-tone) 13%, var(--coterm-surface-raised, #252526));
    color: var(--coterm-text-primary, #cccccc); box-shadow: 0 4px 16px #0002;
    pointer-events: auto; animation: coterm-toast-enter 140ms ease-out; }
  .coterm-toast[data-tone="error"] { --toast-tone: var(--coterm-danger, #e86464); }
  .coterm-toast[data-tone="warning"] { --toast-tone: var(--coterm-warning, hsl(var(--warning, 40 85% 60%))); }
  .coterm-toast-icon { width: 18px; height: 18px; margin-top: 2px; fill: none; stroke: var(--toast-tone); stroke-width: 1.7; }
  .coterm-toast-message { min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .coterm-toast-close { display: grid; place-items: center; width: 24px; height: 24px; padding: 3px;
    border: 0; border-radius: 4px; color: inherit; background: transparent; cursor: pointer; }
  .coterm-toast-close:hover { background: var(--coterm-surface-hover, #ffffff18); }
  .coterm-toast-close:focus-visible { outline: 2px solid var(--coterm-focus, #3794ff); outline-offset: 1px; }
  .coterm-toast-close svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.7; }
  @keyframes coterm-toast-enter { from { opacity: 0; transform: translateY(-6px); } to { opacity: 1; transform: translateY(0); } }
  @media (prefers-reduced-motion: reduce) { .coterm-toast { animation: none; } }
`;

/** Non-modal notices for both Shadow DOM drivers and host-owned surfaces. No focus is moved. */
export function mountToasts(container: HTMLElement, options: ToastStackOptions = {}): ToastStack {
  const document = container.ownerDocument;
  const clock = document.defaultView ?? window;
  const root = document.createElement("div");
  root.className = "coterm-toasts";
  root.setAttribute("role", "region");
  root.setAttribute("aria-label", options.ariaLabel ?? "Notifications");
  const style = document.createElement("style"); style.textContent = css;
  container.append(style, root);
  const bounded = (value: number | undefined, fallback: number, min: number, max: number) =>
    Number.isFinite(value) ? Math.max(min, Math.min(max, Number(value))) : fallback;
  const limit = Math.trunc(bounded(options.maxVisible, 3, 1, 5));
  let closeLabel = options.closeLabel ?? "Dismiss notification";
  let disposed = false;
  interface Entry { key: string; node: HTMLElement; button: HTMLButtonElement; timer: number; remaining: number; since: number; hovered: boolean }
  const entries: Entry[] = [];
  const dismiss = (entry: Entry) => {
    clock.clearTimeout(entry.timer);
    const index = entries.indexOf(entry);
    if (index >= 0) entries.splice(index, 1);
    entry.node.remove();
  };
  const pause = (entry: Entry) => {
    if (!entry.timer) return;
    clock.clearTimeout(entry.timer); entry.timer = 0;
    entry.remaining = Math.max(0, entry.remaining - (clock.performance.now() - entry.since));
  };
  const resume = (entry: Entry) => {
    if (disposed || entry.timer || entry.hovered || entry.node.matches(":focus-within")) return;
    entry.since = clock.performance.now();
    entry.timer = clock.setTimeout(() => dismiss(entry), entry.remaining);
  };
  const clear = () => { for (const entry of [...entries]) dismiss(entry); };
  return {
    show({ message, tone = "info", durationMs }) {
      if (disposed || !message.trim()) return () => {};
      const text = message.slice(0, 2048);
      const key = `${tone}:${text}`;
      const duration = bounded(durationMs ?? options.durationMs, 4000, 500, 30_000);
      const existing = entries.find(entry => entry.key === key);
      if (existing) {
        pause(existing); existing.remaining = duration; resume(existing);
        return () => dismiss(existing);
      }
      while (entries.length >= limit) dismiss(entries[0]);
      const node = document.createElement("div");
      node.className = "coterm-toast"; node.dataset.tone = tone;
      node.setAttribute("role", tone === "info" ? "status" : "alert");
      node.setAttribute("aria-atomic", "true");
      const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      icon.classList.add("coterm-toast-icon"); icon.setAttribute("viewBox", "0 0 24 24");
      icon.setAttribute("aria-hidden", "true");
      icon.innerHTML = '<circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 3v1"/>';
      const body = document.createElement("span"); body.className = "coterm-toast-message"; body.textContent = text;
      const button = document.createElement("button"); button.type = "button";
      button.className = "coterm-toast-close"; button.setAttribute("aria-label", closeLabel);
      button.innerHTML = '<svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5 5 10 10M15 5 5 15"/></svg>';
      node.append(icon, body, button);
      const entry: Entry = { key, node, button, timer: 0, remaining: duration, since: 0, hovered: false };
      entries.push(entry); root.append(node);
      button.addEventListener("click", () => dismiss(entry));
      node.addEventListener("mouseenter", () => { entry.hovered = true; pause(entry); });
      node.addEventListener("mouseleave", () => { entry.hovered = false; resume(entry); });
      node.addEventListener("focusin", () => pause(entry));
      node.addEventListener("focusout", () => clock.queueMicrotask(() => { if (entries.includes(entry)) resume(entry); }));
      resume(entry);
      return () => dismiss(entry);
    },
    updateLabels(labels) {
      if (labels.ariaLabel !== undefined) root.setAttribute("aria-label", labels.ariaLabel);
      if (labels.closeLabel !== undefined) {
        closeLabel = labels.closeLabel;
        for (const entry of entries) entry.button.setAttribute("aria-label", closeLabel);
      }
    },
    clear,
    unmount() { disposed = true; clear(); root.remove(); style.remove(); },
  };
}
