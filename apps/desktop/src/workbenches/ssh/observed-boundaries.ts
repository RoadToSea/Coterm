import type { IMarker, Terminal } from "@xterm/xterm";

/** Visual extent only. Recognising a returned prompt never implies an exit status
 * and does not change the backend's explicitly uncertain observation window.
 */
export function observeBlockPrompt(terminal: Terminal, endBefore: (id: string, lineOffset: number) => void) {
  let active: { id: string; start: IMarker; prompt: string; identity?: string } | undefined;
  const reset = () => { active?.start.dispose(); active = undefined; };
  const inspect = () => {
    const buffer = terminal.buffer.active;
    if (!active || buffer.type !== "normal") return;
    if (active.start.isDisposed) { reset(); return; }
    const cursor = buffer.baseY + buffer.cursorY;
    let first = cursor;
    // A long prompt may wrap across several terminal rows.
    while (first > 0 && cursor - first < 64 && buffer.getLine(first)?.isWrapped) first--;
    if (first <= active.start.line) return;
    let prompt = "";
    for (let line = first; line <= cursor; line++) {
      prompt += buffer.getLine(line)?.translateToString(false, 0, line === cursor ? buffer.cursorX : terminal.cols) ?? "";
    }
    const samePrompt = prompt === active.prompt;
    const changedDirectory = active.identity && prompt.startsWith(active.identity) && /^.*[$#%] $/.test(prompt);
    if (!samePrompt && !changedDirectory) return;
    const id = active.id;
    reset();
    endBefore(id, first - cursor - 1);
  };
  const parsed = terminal.onWriteParsed(inspect);
  return {
    begin(id: string, lineOffset: number) {
      reset();
      const buffer = terminal.buffer.active;
      const line = buffer.getLine(buffer.baseY + buffer.cursorY + lineOffset)?.translateToString(false) ?? "";
      const prompt = line.match(/^.*?[$#%] /)?.[0];
      if (!prompt || buffer.type !== "normal") return;
      const start = terminal.registerMarker(lineOffset);
      if (start) active = { id, start, prompt, identity: prompt.match(/^\S+@[^: ]+:/)?.[0] };
    },
    end(id: string) { if (active?.id === id) { inspect(); reset(); } },
    reset,
    dispose() { reset(); parsed.dispose(); },
  };
}
