/** xterm-compatible surface; the SDK does not depend on a terminal renderer. */
export interface ShortcutTerminal {
  readonly element: HTMLElement | undefined;
  hasSelection(): boolean;
  getSelection(): string;
  selectAll(): void;
  clearSelection(): void;
  attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean): void;
}

/** Local shortcuts only run for events delivered to this terminal, not the page. */
export function attachTerminalShortcuts(terminal: ShortcutTerminal, onCopyError: () => void): () => void {
  let disposed = false;
  const localKeys = new Set<string>();
  const element = terminal.element;
  const clearKeys = () => localKeys.clear();
  element?.addEventListener("focusout", clearKeys);
  const copy = async () => {
    const text = terminal.getSelection();
    if (!text) return;
    const document = terminal.element?.ownerDocument;
    try {
      const clipboard = document?.defaultView?.navigator.clipboard;
      if (clipboard) {
        try { await clipboard.writeText(text); return; }
        catch { /* Older WebViews may only support the native copy event. */ }
      }
      // xterm's copy listener supplies its selection to the native clipboard event.
      if (!disposed && element?.matches(":focus-within") && terminal.getSelection() === text
        && document?.execCommand("copy")) return;
    } catch { /* Copy failure must never become remote Ctrl+C. */ }
    if (!disposed) onCopyError();
  };

  terminal.attachCustomKeyEventHandler((event) => {
    if (disposed || event.isComposing || event.keyCode === 229 || event.altKey) return true;
    const key = event.key.toLowerCase();
    const identity = event.code || key;
    if (localKeys.has(identity)) {
      if (event.type === "keyup") localKeys.delete(identity);
      if (key !== "v" && !(key === "insert" && event.shiftKey)) event.preventDefault();
      event.stopPropagation();
      return false;
    }
    const ctrl = event.ctrlKey && !event.metaKey;
    const cmd = event.metaKey && !event.ctrlKey;
    const copyKey = (ctrl && key === "c" && (event.shiftKey || terminal.hasSelection()))
      || (ctrl && !event.shiftKey && key === "insert")
      || (cmd && !event.shiftKey && key === "c");
    const pasteKey = ((ctrl || cmd) && key === "v")
      || (!event.ctrlKey && !event.metaKey && event.shiftKey && key === "insert");
    const selectAll = key === "a" && ((ctrl && event.shiftKey) || (cmd && !event.shiftKey));
    const clearSelection = key === "escape" && !event.ctrlKey && !event.metaKey
      && !event.shiftKey && terminal.hasSelection();

    if (pasteKey) {
      // Leave the native paste event to xterm: no clipboard-read permission and no
      // duplicate send. Its paste path also applies bracketed-paste handling.
      event.stopPropagation();
      if (event.type === "keydown") localKeys.add(identity);
      return false;
    }
    if (!copyKey && !selectAll && !clearSelection) return true;
    event.preventDefault();
    event.stopPropagation();
    if (event.type === "keydown" && !event.repeat) {
      localKeys.add(identity);
      if (copyKey) void copy();
      else if (selectAll) terminal.selectAll();
      else terminal.clearSelection();
    }
    return false;
  });

  return () => {
    disposed = true;
    localKeys.clear();
    element?.removeEventListener("focusout", clearKeys);
    terminal.attachCustomKeyEventHandler(() => true);
  };
}
