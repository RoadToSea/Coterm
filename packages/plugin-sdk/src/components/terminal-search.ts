interface SearchOptions {
  incremental: boolean;
  caseSensitive: boolean;
  regex: boolean;
  decorations: {
    matchBackground: string; matchOverviewRuler: string;
    activeMatchBackground: string; activeMatchColorOverviewRuler: string;
  };
}

/** Compatible with xterm's SearchAddon without coupling the SDK to its renderer. */
export interface TerminalSearchAdapter {
  findNext(term: string, options: SearchOptions): boolean;
  findPrevious(term: string, options: SearchOptions): boolean;
  clearDecorations(): void;
  onDidChangeResults(listener: (results: { resultIndex: number; resultCount: number }) => void): { dispose(): void };
}

interface SearchTerminal {
  getSelection(): string;
  clearSelection(): void;
  focus(): void;
}

type Translate = (message: string, values?: Record<string, string | number>) => string;
export const TERMINAL_SEARCH_HIGHLIGHT_LIMIT = 1000;

const css = `
  .terminal-find { display: flex; align-items: center; flex-wrap: wrap; gap: 8px;
    padding: 6px 14px; border-bottom: 1px solid var(--coterm-border, #333333);
    background: var(--coterm-surface-raised, #252526); }
  .terminal-find[hidden] { display: none; }
  .terminal-find input { flex: 1 1 180px; width: auto; min-width: 0; max-width: 440px; }
  .terminal-find button { flex: none; padding: 4px 9px; }
  .terminal-find-result { color: var(--coterm-text-muted, #969696); font-size: 11px; }
`;

/** Mount one local search bar after a plugin's toolbar. Query text stays in memory. */
export function mountTerminalSearch(anchor: HTMLElement, terminal: SearchTerminal,
  search: TerminalSearchAdapter, t: Translate) {
  const document = anchor.ownerDocument;
  const panel = document.createElement("div");
  panel.className = "terminal-find";
  panel.hidden = true;
  panel.setAttribute("role", "search");
  const style = document.createElement("style");
  style.textContent = css;
  const input = document.createElement("input");
  input.type = "search";
  input.maxLength = 512;
  input.spellcheck = false;
  input.autocomplete = "off";
  const result = document.createElement("span");
  result.className = "terminal-find-result";
  result.setAttribute("role", "status");
  result.setAttribute("aria-live", "polite");
  const previous = document.createElement("button");
  previous.type = "button";
  previous.textContent = "↑";
  const next = document.createElement("button");
  next.type = "button";
  next.textContent = "↓";
  const closeButton = document.createElement("button");
  closeButton.type = "button";
  closeButton.textContent = "×";
  panel.append(style, input, result, previous, next, closeButton);
  anchor.after(panel);

  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let results = { resultIndex: -1, resultCount: 0 };
  const controller = new AbortController();
  const stopTimer = () => { clearTimeout(timer); timer = undefined; };
  const render = () => {
    const total = results.resultCount >= TERMINAL_SEARCH_HIGHLIGHT_LIMIT
      ? `${TERMINAL_SEARCH_HIGHLIGHT_LIMIT}+` : results.resultCount;
    result.textContent = !input.value ? "" : results.resultCount === 0 ? t("No matches")
      : results.resultIndex < 0 ? t("{total} matches", { total })
        : t("{index} of {total}", { index: results.resultIndex + 1, total });
    previous.disabled = next.disabled = !input.value || results.resultCount === 0;
  };
  const subscription = search.onDidChangeResults((value) => {
    if (disposed || panel.hidden) return;
    results = value;
    render();
  });
  const run = (backwards = false, incremental = false) => {
    stopTimer();
    if (disposed || panel.hidden) return;
    if (!input.value) {
      search.clearDecorations();
      terminal.clearSelection();
      results = { resultIndex: -1, resultCount: 0 };
      render();
      return;
    }
    const options: SearchOptions = {
      caseSensitive: false, regex: false, incremental,
      decorations: {
        matchBackground: "#515C6A", matchOverviewRuler: "#969696",
        activeMatchBackground: "#9D5D00", activeMatchColorOverviewRuler: "#CCA700",
      },
    };
    if (backwards) search.findPrevious(input.value, options);
    else search.findNext(input.value, options);
  };
  const refreshLocale = () => {
    panel.setAttribute("aria-label", t("Find in terminal"));
    input.setAttribute("aria-label", t("Find in terminal"));
    input.placeholder = t("Find in terminal");
    for (const [button, label] of [
      [previous, "Previous match (Shift+Enter)"], [next, "Next match (Enter)"], [closeButton, "Close search (Esc)"],
    ] as const) {
      button.title = t(label);
      button.setAttribute("aria-label", t(label));
    }
    render();
  };
  const close = (restoreFocus = true) => {
    if (disposed) return;
    stopTimer();
    panel.hidden = true;
    search.clearDecorations();
    terminal.clearSelection();
    if (restoreFocus) terminal.focus();
  };
  const open = () => {
    if (disposed) return;
    if (panel.hidden) {
      const selection = terminal.getSelection();
      if (!input.value && selection && selection.length <= input.maxLength && !/[\r\n]/.test(selection)) input.value = selection;
      panel.hidden = false;
      run(false, true);
    }
    input.focus();
    input.select();
  };
  input.addEventListener("input", (event) => {
    stopTimer();
    if (!(event as InputEvent).isComposing) timer = setTimeout(() => run(false, true), 100);
  }, { signal: controller.signal });
  input.addEventListener("compositionend", () => { stopTimer(); timer = setTimeout(() => run(false, true), 100); }, { signal: controller.signal });
  previous.addEventListener("click", () => run(true), { signal: controller.signal });
  next.addEventListener("click", () => run(), { signal: controller.signal });
  closeButton.addEventListener("click", () => close(), { signal: controller.signal });
  panel.addEventListener("keydown", (event) => {
    if (event.isComposing || event.keyCode === 229 || event.altKey) return;
    if (event.key === "Escape" || ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f")) {
      event.preventDefault(); event.stopPropagation();
      if (event.key === "Escape") close(); else open();
    } else if (event.key === "Enter" && event.target === input && !event.ctrlKey && !event.metaKey) {
      event.preventDefault(); event.stopPropagation(); run(event.shiftKey);
    }
  }, { signal: controller.signal });
  refreshLocale();
  return {
    open, close, refreshLocale,
    dispose() {
      if (disposed) return;
      close(false);
      disposed = true;
      controller.abort();
      subscription.dispose();
      panel.remove();
    },
  };
}
