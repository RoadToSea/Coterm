import type { IMarker, Terminal } from "@xterm/xterm";

export interface CommandObservation { text: string; lineOffset: number }

/** Observe echoed shell lines; never accumulate raw input (which can be a password).
 * This is deliberately a prompt heuristic, not shell execution/exit detection.
 */
export function observeHumanCommands(terminal: Terminal) {
  let start: { marker: IMarker; column: number; prefix: string } | undefined;
  let suspended = false;
  let disposed = false;
  let revision = 0;
  let inputRevision = -1;
  let wake: (() => void) | undefined;
  const parsed = terminal.onWriteParsed(() => { revision++; wake?.(); });
  const reset = () => { start?.marker.dispose(); start = undefined; inputRevision = -1; wake?.(); };
  const resized = terminal.onResize(reset);
  const read = (): CommandObservation | undefined => {
    const buffer = terminal.buffer.active;
    if (!start || start.marker.isDisposed || buffer.type !== "normal") return;
    const first = start.marker.line;
    const current = buffer.baseY + buffer.cursorY;
    if (current < first || current - first > 64) return;
    // An intervening redraw, prompt change or cursor movement invalidates the anchor.
    if (buffer.getLine(first)?.translateToString(false, 0, start.column) !== start.prefix) return;
    let text = "";
    let last = first;
    for (; last <= first + 64; last++) {
      const line = buffer.getLine(last);
      if (!line || (last > first && !line.isWrapped)) break;
      text += line.translateToString(false, last === first ? start.column : 0);
    }
    text = text.trimEnd();
    if (!text.trim() || /[\u0000-\u001f\u007f-\u009f]/.test(text) || new TextEncoder().encode(text).length > 4096) return;
    return { text, lineOffset: first - current };
  };
  return {
    async beforeInput(text: string): Promise<CommandObservation | undefined> {
      if (disposed || suspended || terminal.buffer.active.type !== "normal") { reset(); return; }
      if (text === "\r") {
        // A fast Enter may overtake the last remote echo. Wait briefly for parsing,
        // without retaining the typed characters or delaying password input.
        const deadline = performance.now() + 180;
        let pending = inputRevision === revision;
        while (start && !disposed && !suspended && performance.now() < deadline) {
          const previous = revision;
          await new Promise<void>(resolve => {
            const timer = window.setTimeout(done, Math.min(pending ? 180 : 16, deadline - performance.now()));
            function done() { window.clearTimeout(timer); wake = undefined; resolve(); }
            wake = done;
          });
          pending = false;
          if (revision === previous) break;
        }
        const observation = !disposed && !suspended ? read() : undefined;
        reset();
        return observation;
      }
      if (/[\r\n\x03\x04]/.test(text)) {
        // Bulk multiline paste and interrupted lines have ambiguous boundaries.
        reset(); return;
      }
      if (!start) {
        const buffer = terminal.buffer.active;
        const prefix = buffer.getLine(buffer.baseY + buffer.cursorY)?.translateToString(false, 0, buffer.cursorX) ?? "";
        // Common Bash/sh/zsh prompts. Password/continuation prompts do not qualify.
        if (!/[$#%] $/.test(prefix)) return;
        const marker = terminal.registerMarker(0);
        if (marker) start = { marker, column: buffer.cursorX, prefix };
      }
    },
    sent() { inputRevision = revision; },
    suspend(value: boolean) { suspended = value; reset(); },
    reset,
    dispose() { disposed = true; reset(); parsed.dispose(); resized.dispose(); },
  };
}
