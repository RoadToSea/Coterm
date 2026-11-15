import { Button } from "../../components/ui/button";
import { mountReactContent } from "./react-content";

export type ToastTone = "error" | "warning" | "info";
export interface ToastMessage { message: string; tone?: ToastTone; durationMs?: number }
export interface ToastStackOptions { durationMs?: number; maxVisible?: number; ariaLabel?: string; closeLabel?: string }
export interface ToastStack {
  show(message: ToastMessage): () => void;
  updateLabels(labels: Pick<ToastStackOptions, "ariaLabel" | "closeLabel">): void;
  clear(): void;
  unmount(): void;
}
interface Entry { key: string; text: string; tone: ToastTone; timer: number; remaining: number; since: number; hovered: boolean; focused: boolean }

/** Bounded host notices pause expiry while hovered or focused and never move focus. */
export function mountToasts(container: HTMLElement, options: ToastStackOptions = {}): ToastStack {
  const host = container.ownerDocument.createElement("div");
  container.append(host);
  const view = mountReactContent(host);
  const bounded = (value: number | undefined, fallback: number, min: number, max: number) =>
    Number.isFinite(value) ? Math.max(min, Math.min(max, Number(value))) : fallback;
  const limit = Math.trunc(bounded(options.maxVisible, 3, 1, 5));
  let labels = { ariaLabel: "Notifications", closeLabel: "Dismiss notification", ...options };
  let disposed = false;
  const entries: Entry[] = [];
  const render = () => view.render(<div className="coterm-toasts" role="region" aria-label={labels.ariaLabel}>
    {entries.map(entry => <div key={entry.key} className="coterm-toast" data-tone={entry.tone} role={entry.tone === "info" ? "status" : "alert"} aria-atomic="true"
      onMouseEnter={() => { entry.hovered = true; pause(entry); }} onMouseLeave={() => { entry.hovered = false; resume(entry); }}
      onFocus={() => { entry.focused = true; pause(entry); }} onBlur={event => {
        if (event.currentTarget.contains(event.relatedTarget)) return;
        entry.focused = false; resume(entry);
      }}>
      <svg className="coterm-toast-icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M12 7v6m0 3v1" /></svg>
      <span className="coterm-toast-message">{entry.text}</span>
      <Button variant="ghost" size="icon" className="coterm-toast-close" aria-label={labels.closeLabel} onClick={() => dismiss(entry)}><svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5 5 10 10M15 5 5 15" /></svg></Button>
    </div>)}
  </div>);
  const dismiss = (entry: Entry) => {
    window.clearTimeout(entry.timer);
    const index = entries.indexOf(entry);
    if (index >= 0) entries.splice(index, 1);
    if (!disposed) render();
  };
  const pause = (entry: Entry) => {
    if (!entry.timer) return;
    window.clearTimeout(entry.timer); entry.timer = 0;
    entry.remaining = Math.max(0, entry.remaining - (performance.now() - entry.since));
  };
  const resume = (entry: Entry) => {
    if (disposed || entry.timer || entry.hovered || entry.focused || !entries.includes(entry)) return;
    entry.since = performance.now(); entry.timer = window.setTimeout(() => dismiss(entry), entry.remaining);
  };
  const clear = () => { for (const entry of [...entries]) dismiss(entry); };
  render();
  return {
    show({ message, tone = "info", durationMs }) {
      if (disposed || !message.trim()) return () => {};
      const text = message.slice(0, 2048), key = `${tone}:${text}`;
      const duration = bounded(durationMs ?? options.durationMs, 4000, 500, 30_000);
      const existing = entries.find(entry => entry.key === key);
      if (existing) { pause(existing); existing.remaining = duration; resume(existing); return () => dismiss(existing); }
      while (entries.length >= limit) dismiss(entries[0]);
      const entry: Entry = { key, text, tone, timer: 0, remaining: duration, since: 0, hovered: false, focused: false };
      entries.push(entry); render(); resume(entry);
      return () => dismiss(entry);
    },
    updateLabels(next) { if (!disposed) { labels = { ...labels, ...next }; render(); } }, clear,
    unmount() { if (disposed) return; disposed = true; clear(); view.unmount(); host.remove(); },
  };
}
