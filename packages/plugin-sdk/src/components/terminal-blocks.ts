import type { CommandBlock } from "../plugin-api";
interface Disposable {
  dispose(): void;
}
interface Marker extends Disposable {
  readonly line: number;
  readonly isDisposed: boolean;
}
/** Structural adapter: the SDK does not depend on a terminal implementation at runtime. */
export interface BlockTerminal {
  readonly cols: number;
  readonly rows: number;
  readonly element: HTMLElement | undefined;
  readonly buffer: {
    readonly active: {
      readonly type: string;
      readonly viewportY: number;
      readonly baseY: number;
      readonly cursorY: number;
      readonly cursorX: number;
    };
  };
  registerMarker(offset?: number): Marker | undefined;
  registerDecoration(options: {
    marker: Marker;
    width?: number;
    layer?: "bottom" | "top";
  }): (Disposable & {
    onRender(callback: (element: HTMLElement) => void): Disposable;
  }) | undefined;
  onRender(callback: () => void): Disposable;
  onScroll(callback: () => void): Disposable;
  onResize(callback: () => void): Disposable;
}
export interface TerminalBlockOptions {
  onSelectionChange(ids: string[]): void;
  cancelLabel: string;
}
/** Plugin-supplied boundaries anchor to real buffer markers; the SDK never parses prompts. */
export function mountTerminalBlocks(gutter: HTMLElement, container: HTMLElement, terminal: BlockTerminal, options: TerminalBlockOptions) {
  const style = document.createElement("style");
  style.textContent = `
  .coterm-terminal-gutter { position:relative; flex:none; box-sizing:border-box; width:max(28px,calc(var(--coterm-terminal-number-chars,2) * 1ch + 12px)); min-width:28px; border-right:1px solid var(--coterm-border,#333); font:11px var(--coterm-font-mono,Consolas,monospace); }
  .coterm-terminal-clear { position:absolute; top:5px; left:50%; transform:translateX(-50%); display:grid; place-items:center; box-sizing:border-box; width:16px; height:16px; padding:0; border:1px solid #df4740; border-radius:50%; background:#ff5f57; color:#711c17; box-shadow:inset 0 1px 0 rgba(255,255,255,.3); font:14px/1 sans-serif; cursor:pointer; z-index:2; }
  .coterm-terminal-clear:hover:not(:disabled) { background:#ff746d; color:#47110e; }
  .coterm-terminal-clear:active:not(:disabled) { transform:translateX(-50%) scale(.92); }
  .coterm-terminal-clear:focus-visible { outline:2px solid var(--coterm-focus,#007fd4); outline-offset:2px; }
  .coterm-terminal-clear:disabled { opacity:1; color:rgba(113,28,23,.45); cursor:default; }
  .coterm-terminal-number { position:absolute; left:2px; box-sizing:border-box; width:calc(100% - 4px); padding:0 2px; border:0; background:transparent; text-align:right; font:inherit; color:var(--coterm-text-muted,#969696); }
  .coterm-terminal-number[aria-pressed="true"] { color:var(--coterm-focus,#38bdf8); background:var(--coterm-selection,rgba(56,189,248,.15)); }
  .coterm-terminal-frames { position:absolute; inset:0; pointer-events:none; overflow:hidden; }
  .coterm-terminal-frame { position:absolute; border:1px solid var(--coterm-border-subtle,#454545); border-radius:3px; pointer-events:none; }
  .coterm-terminal-frame[data-selected="true"] { border-color:var(--coterm-focus,#38bdf8); background:color-mix(in srgb,var(--coterm-focus,#38bdf8) 10%,transparent); }
  .coterm-command-start { border-top:1px solid var(--coterm-border-subtle,#454545); pointer-events:none; }
  `;
  gutter.classList.add("coterm-terminal-gutter");
  const clear = document.createElement("button");
  clear.type = "button";
  clear.className = "coterm-terminal-clear";
  const frames = document.createElement("div");
  frames.className = "coterm-terminal-frames";
  gutter.append(style, clear);
  container.append(frames);
  type Entry = {
    block: CommandBlock;
    start: Marker;
    end?: Marker;
    decoration?: Disposable;
    button: HTMLButtonElement;
    frame: HTMLDivElement;
  };
  const entries = new Map<string, Entry>();
  const selected = new Set<string>();
  let available = new Set<string>();
  const controller = new AbortController();
  let disposed = false;
  let scheduled = 0;
  let label = options.cancelLabel;
  const notify = () => {
    clear.disabled = selected.size === 0;
    options.onSelectionChange([...selected]);
  };
  const remove = (id: string) => {
    const entry = entries.get(id);
    if (!entry)
      return;
    entry.decoration?.dispose();
    entry.start.dispose();
    entry.end?.dispose();
    entry.button.remove();
    entry.frame.remove();
    entries.delete(id);
  };
  const render = () => {
    scheduled = 0;
    if (disposed)
      return;
    const screen = terminal.element?.querySelector<HTMLElement>(".xterm-screen");
    if (!screen)
      return;
    let numberChars = 2;
    for (const entry of entries.values()) {
      if (!entry.start.isDisposed)
        numberChars = Math.max(numberChars, String(entry.block.index).length + 1);
    }
    const width = String(numberChars);
    if (gutter.style.getPropertyValue("--coterm-terminal-number-chars") !== width)
      gutter.style.setProperty("--coterm-terminal-number-chars", width);
    const bounds = screen.getBoundingClientRect();
    const origin = container.getBoundingClientRect();
    const gutterBounds = gutter.getBoundingClientRect();
    const buffer = terminal.buffer.active;
    const lineHeight = bounds.height / terminal.rows;
    let changed = false;
    for (const [id, entry] of entries) {
      if (entry.start.isDisposed) {
        changed = selected.delete(id) || changed;
        remove(id);
        continue;
      }
      const start = entry.start.line;
      const end = entry.end && !entry.end.isDisposed ? entry.end.line : buffer.baseY + buffer.cursorY;
      const first = Math.max(start, buffer.viewportY);
      const last = Math.min(Math.max(start, end), buffer.viewportY + terminal.rows - 1);
      const hidden = buffer.type !== "normal" || first > last;
      entry.button.hidden = entry.frame.hidden = hidden;
      entry.button.textContent = `#${entry.block.index}`;
      entry.button.title = entry.block.command;
      entry.button.setAttribute("aria-label", `#${entry.block.index} · ${entry.block.actor === "human" ? "Human" : "Agent"} · ${entry.block.command}`);
      entry.button.setAttribute("aria-pressed", String(selected.has(id)));
      entry.frame.dataset.selected = String(selected.has(id));
      entry.button.style.top = `${bounds.top - gutterBounds.top + (first - buffer.viewportY) * lineHeight}px`;
      entry.button.style.height = `${Math.max(18, lineHeight)}px`;
      entry.frame.style.cssText = `left:${bounds.left - origin.left}px;top:${bounds.top - origin.top + (first - buffer.viewportY) * lineHeight}px;width:${bounds.width}px;height:${Math.max(1, last - first + 1) * lineHeight}px`;
    }
    if (changed)
      notify();
    clear.textContent = "×";
    clear.title = label;
    clear.setAttribute("aria-label", label);
  };
  const schedule = () => { if (!disposed && !scheduled)
    scheduled = requestAnimationFrame(render); };
  const toggle = (id: string) => {
    if (!entries.has(id) && !available.has(id)) return;
    if (selected.has(id))
      selected.delete(id);
    else
      selected.add(id);
    notify();
    schedule();
  };
  const cancel = () => { selected.clear(); notify(); schedule(); };
  clear.addEventListener("click", cancel, { signal: controller.signal });
  gutter.addEventListener("keydown", event => {
    if (event.key === "Escape" && selected.size) {
      event.preventDefault();
      event.stopPropagation();
      cancel();
    }
  }, { signal: controller.signal });
  const listeners = [terminal.onRender(schedule), terminal.onScroll(schedule), terminal.onResize(schedule)];
  const observer = new ResizeObserver(schedule);
  observer.observe(container);
  clear.disabled = true;
  clear.textContent = "×";
  clear.title = label;
  clear.setAttribute("aria-label", label);
  return {
    begin(block: CommandBlock, lineOffset = 0) {
      if (disposed || entries.has(block.id))
        return;
      while (entries.size >= 32) {
        const oldest = entries.keys().next().value;
        if (!oldest) break;
        const changed = selected.delete(oldest);
        remove(oldest);
        if (changed) notify();
      }
      const marker = terminal.registerMarker(lineOffset);
      if (!marker)
        return;
      const button = document.createElement("button");
      button.type = "button";
      button.className = "coterm-terminal-number";
      button.dataset.blockId = block.id;
      button.addEventListener("click", () => toggle(block.id), { signal: controller.signal });
      const frame = document.createElement("div");
      frame.className = "coterm-terminal-frame";
      frame.dataset.blockId = block.id;
      const decoration = terminal.registerDecoration({ marker, width: terminal.cols, layer: "bottom" });
      decoration?.onRender(element => { element.classList.add("coterm-command-start"); element.dataset.blockId = block.id; });
      entries.set(block.id, { block, start: marker, decoration, button, frame });
      gutter.append(button);
      frames.append(frame);
      schedule();
    },
    end(id: string) {
      const entry = entries.get(id);
      if (entry && !entry.end)
        entry.end = terminal.registerMarker(terminal.buffer.active.cursorX === 0 ? -1 : 0);
      schedule();
    },
    update(blocks: CommandBlock[]) {
      const ids = new Set(blocks.map(block => block.id));
      available = ids;
      let changed = false;
      for (const id of entries.keys())
        if (!ids.has(id)) {
          changed = selected.delete(id) || changed;
          remove(id);
        }
      for (const block of blocks) {
        const entry = entries.get(block.id);
        if (entry)
          entry.block = block;
      }
      // Exec blocks have no PTY range, but can share context selection through their capsule.
      for (const id of selected)
        if (!ids.has(id)) {
          selected.delete(id);
          changed = true;
        }
      if (changed)
        notify();
      schedule();
    },
    toggle,
    clear: cancel,
    refreshLocale(next: string) { label = next; clear.title = label; clear.setAttribute("aria-label", label); schedule(); },
    reset() { available.clear(); for (const id of entries.keys())
      remove(id); cancel(); },
    unmount() {
      disposed = true;
      controller.abort();
      cancelAnimationFrame(scheduled);
      observer.disconnect();
      for (const listener of listeners)
        listener.dispose();
      for (const id of entries.keys())
        remove(id);
      selected.clear();
      frames.remove();
      gutter.style.removeProperty("--coterm-terminal-number-chars");
      gutter.replaceChildren();
    },
  };
}
