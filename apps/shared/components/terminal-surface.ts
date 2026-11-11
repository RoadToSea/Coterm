import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import terminalCSS from "@xterm/xterm/css/xterm.css?inline";
import { attachTerminalPreferences, attachTerminalTheme, readTerminalPreferences, readTerminalTheme } from "./terminal-theme";
import { attachTerminalShortcuts } from "./terminal-shortcuts";
import { mountTerminalSearch, TERMINAL_SEARCH_HIGHLIGHT_LIMIT } from "./terminal-search";

type Translate = (message: string, values?: Record<string, string | number>) => string;

/**
 * Mount an xterm surface with theme, typography, search and local shortcuts.
 * The caller owns protocol input/output. Resize callbacks run only for visible surfaces.
 * Returns the terminal, explicit resize and idempotent disposal; invalid DOM hosts throw.
 */
export function mountTerminalSurface(container: HTMLElement, toolbar: HTMLElement, t: Translate,
  onResize: (cols: number, rows: number) => void, onCopyError: () => void) {
  const style = document.createElement("style"); style.textContent = terminalCSS; container.append(style);
  const terminal = new Terminal({ allowProposedApi:true, ...readTerminalPreferences(container), scrollback:3000,
    fontFamily:'Consolas, "Liberation Mono", monospace', theme:readTerminalTheme(container) });
  const fit = new FitAddon(); terminal.loadAddon(fit);
  const search = new SearchAddon({ highlightLimit:TERMINAL_SEARCH_HIGHLIGHT_LIMIT }); terminal.loadAddon(search);
  terminal.open(container);
  const searchView = mountTerminalSearch(toolbar,terminal,search,t);
  const stopShortcuts = attachTerminalShortcuts(terminal,onCopyError,searchView.open);
  let disposed = false, timer = 0;
  const resize = () => {
    if (disposed || !container.clientWidth || !container.clientHeight) return;
    fit.fit(); onResize(terminal.cols,terminal.rows);
  };
  const stopTheme = attachTerminalTheme(container,theme => { terminal.options.theme = theme; });
  const stopPreferences = attachTerminalPreferences(container,preferences => { Object.assign(terminal.options,preferences); resize(); });
  const observer = new ResizeObserver(() => { clearTimeout(timer); timer = window.setTimeout(resize,100); });
  observer.observe(container);
  return { terminal, searchView, resize, dispose() {
    if (disposed) return; disposed = true; clearTimeout(timer); observer.disconnect();
    stopShortcuts(); searchView.dispose(); stopTheme(); stopPreferences(); terminal.dispose(); style.remove();
  } };
}
