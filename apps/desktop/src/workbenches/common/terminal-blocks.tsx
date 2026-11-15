import { createPortal } from "react-dom";
import { Button } from "../../components/ui/button";
import { mountReactContent } from "./react-content";
import type { CommandBlock } from "../../../../shared/driver-api";

interface Disposable { dispose(): void }
interface Marker extends Disposable { readonly line: number; readonly isDisposed: boolean }
/** Buffer markers and geometry are the only renderer-specific inputs to the React overlay. */
export interface BlockTerminal {
  readonly cols: number;
  readonly rows: number;
  readonly element: HTMLElement | undefined;
  readonly buffer: { readonly active: {
    readonly type: string; readonly viewportY: number; readonly baseY: number; readonly cursorY: number; readonly cursorX: number;
  } };
  registerMarker(offset?: number): Marker | undefined;
  onRender(callback: () => void): Disposable;
  onScroll(callback: () => void): Disposable;
  onResize(callback: () => void): Disposable;
}
export interface TerminalBlockOptions { onSelectionChange(ids: string[]): void; cancelLabel: string }
interface Entry { block: CommandBlock; start: Marker; end?: Marker }

/** Render driver-provided boundaries without parsing prompts or changing the terminal buffer. */
export function mountTerminalBlocks(gutter: HTMLElement, container: HTMLElement, terminal: BlockTerminal, options: TerminalBlockOptions) {
  gutter.classList.add("coterm-terminal-gutter");
  const frames = container.ownerDocument.createElement("div"); frames.className = "coterm-terminal-frames"; container.append(frames);
  const view = mountReactContent(gutter);
  const entries = new Map<string, Entry>(), selected = new Set<string>();
  let available = new Set<string>(), disposed = false, scheduled = 0, label = options.cancelLabel;
  const notify = () => options.onSelectionChange([...selected]);
  const remove = (id: string) => { const entry = entries.get(id); if (entry) { entry.start.dispose(); entry.end?.dispose(); entries.delete(id); } };
  const schedule = () => { if (!disposed && !scheduled) scheduled = requestAnimationFrame(render); };
  const toggle = (id: string) => {
    if (disposed || (!entries.has(id) && !available.has(id))) return;
    if (selected.has(id)) selected.delete(id); else selected.add(id);
    notify(); schedule();
  };
  const cancel = () => { if (!disposed) { selected.clear(); notify(); schedule(); } };
  const render = () => {
    scheduled = 0;
    if (disposed) return;
    const screen = terminal.element?.querySelector<HTMLElement>(".xterm-screen");
    if (!screen) return;
    const bounds = screen.getBoundingClientRect(), origin = container.getBoundingClientRect(), gutterBounds = gutter.getBoundingClientRect();
    const buffer = terminal.buffer.active, lineHeight = bounds.height / terminal.rows;
    let changed = false;
    for (const [id, entry] of entries) if (entry.start.isDisposed) { changed = selected.delete(id) || changed; remove(id); }
    const ordered = [...entries.entries()].sort((a, b) => a[1].start.line - b[1].start.line);
    gutter.style.setProperty("--coterm-terminal-number-chars", String(Math.max(1, ...ordered.map(([, entry]) => String(entry.block.index).length))));
    const positioned = ordered.map(([id, entry], position) => {
      const start = entry.start.line, following = ordered[position + 1]?.[1].start;
      const end = Math.min(entry.end && !entry.end.isDisposed ? entry.end.line : buffer.baseY + buffer.cursorY, following && !following.isDisposed ? following.line - 1 : Infinity);
      const first = Math.max(start, buffer.viewportY), last = Math.min(Math.max(start, end), buffer.viewportY + terminal.rows - 1);
      return { id, block: entry.block, hidden: buffer.type !== "normal" || first > last,
        top: (first - buffer.viewportY) * lineHeight, height: Math.max(1, last - first + 1) * lineHeight };
    });
    view.render(<>
      <Button variant="ghost" size="icon" className="coterm-terminal-clear size-4 h-4 w-4 p-0 disabled:opacity-[0.45]" disabled={!selected.size} title={label} aria-label={label} onClick={cancel}>×</Button>
      {positioned.map(({ id, block, hidden, top }) => <Button key={id} variant="ghost" size="sm" className="coterm-terminal-number px-0 py-0" data-block-id={id} hidden={hidden}
        title={block.command} aria-label={`${block.index} · ${block.actor === "human" ? "Human" : "Agent"} · ${block.command}`} aria-pressed={selected.has(id)}
        style={{ top: bounds.top - gutterBounds.top + top, height: Math.max(18, lineHeight) }} onClick={() => toggle(id)} onKeyDown={event => {
          if (event.key === "Escape" && selected.size) { event.preventDefault(); event.stopPropagation(); cancel(); }
        }}>{block.index}</Button>)}
      {createPortal(positioned.map(({ id, hidden, top, height }) => <div key={id} className="coterm-terminal-frame" data-block-id={id} hidden={hidden} data-selected={String(selected.has(id))}
        style={{ left: bounds.left - origin.left, top: bounds.top - origin.top + top, width: bounds.width, height }} />), frames)}
    </>);
    if (changed) notify();
  };
  const listeners = [terminal.onRender(schedule), terminal.onScroll(schedule), terminal.onResize(schedule)];
  const observer = new ResizeObserver(schedule); observer.observe(container); schedule();
  return {
    begin(block: CommandBlock, lineOffset = 0) {
      if (disposed || entries.has(block.id)) return;
      while (entries.size >= 32) {
        const oldest = entries.keys().next().value; if (!oldest) break;
        const changed = selected.delete(oldest); remove(oldest); if (changed) notify();
      }
      const marker = terminal.registerMarker(lineOffset); if (!marker) return;
      entries.set(block.id, { block, start: marker }); schedule();
    },
    end(id: string, lineOffset = terminal.buffer.active.cursorX === 0 ? -1 : 0) {
      const entry = entries.get(id); if (entry && !entry.end) entry.end = terminal.registerMarker(lineOffset); schedule();
    },
    update(blocks: CommandBlock[]) {
      if (disposed) return;
      available = new Set(blocks.map(block => block.id)); let changed = false;
      for (const id of entries.keys()) if (!available.has(id)) { changed = selected.delete(id) || changed; remove(id); }
      for (const block of blocks) { const entry = entries.get(block.id); if (entry) entry.block = block; }
      // Exec blocks have no PTY range but share Agent context selection.
      for (const id of selected) if (!available.has(id)) { selected.delete(id); changed = true; }
      if (changed) notify(); schedule();
    },
    toggle, clear: cancel,
    refreshLocale(next: string) { label = next; schedule(); },
    reset() { available.clear(); for (const id of entries.keys()) remove(id); cancel(); },
    unmount() {
      if (disposed) return;
      disposed = true; cancelAnimationFrame(scheduled); observer.disconnect();
      for (const listener of listeners) listener.dispose();
      for (const id of entries.keys()) remove(id);
      selected.clear(); view.unmount(); frames.remove(); gutter.style.removeProperty("--coterm-terminal-number-chars");
    },
  };
}
