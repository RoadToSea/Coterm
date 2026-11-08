import { attachTerminalTheme, readTerminalTheme } from "@coterm/driver-sdk/components/terminal-theme";
import { attachTerminalShortcuts } from "@coterm/driver-sdk/components/terminal-shortcuts";
import { mountTerminalSearch, TERMINAL_SEARCH_HIGHLIGHT_LIMIT } from "@coterm/driver-sdk/components/terminal-search";
import type { OperationReceipt, DriverAPI, DriverConfigurationAPI, DriverFrontend } from "@coterm/driver-sdk";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Terminal } from "@xterm/xterm";
import terminalCSS from "@xterm/xterm/css/xterm.css?inline";

import { createTranslator } from "./i18n";

const css = `
  :host {
    display: block;
    height: 100%;
    min-height: 0;
    color: var(--coterm-text-primary, #cccccc);
    font: 13px var(--coterm-font-sans, "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif);
  }
  * { box-sizing: border-box; }
  button, input, select, textarea { font: inherit; color: inherit; }
  form input, form select, form button { border-radius: var(--radius, 8px); }
  button {
    border: 1px solid var(--coterm-border-input, #454545);
    padding: 5px 12px;
    background: var(--coterm-surface-sidebar, #333333);
    cursor: pointer;
    border-radius: var(--radius, 8px);
  }
  button:hover { background: var(--coterm-surface-hover, #414141); }
  button:disabled { opacity: .5; cursor: default; }
  button:focus-visible, input:focus-visible { outline: 1px solid var(--coterm-focus, #007fd4); }
  .primary { background: var(--coterm-accent, #0e639c); color: var(--coterm-accent-foreground, #ffffff); }
  .primary:hover { background: var(--coterm-accent-hover, #1177bb); }
  form { display: grid; gap: 14px; }
  label { display: grid; gap: 6px; color: var(--coterm-text-subtle, #aaaaaa); }
  input {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--coterm-border-input, #454545);
    background: var(--coterm-surface, #1e1e1e);
    color: var(--coterm-text-primary, #cccccc);
    padding: 7px 9px;
  }
  .error { color: var(--coterm-danger, #f48771); overflow-wrap: anywhere; margin: 0; }
  .error:empty { display: none; }
  .hint { color: var(--coterm-text-muted, #969696); font-size: 11px; overflow-wrap: anywhere; margin: 0; }
  .quick { display: flex; flex-wrap: wrap; gap: 8px; }
  .quick button { font-size: 11px; padding: 4px 10px; }
  .layout { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--coterm-surface, #1e1e1e); }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    min-height: 42px;
    border-bottom: 1px solid var(--coterm-border, #333333);
    padding: 6px 14px;
    background: var(--coterm-surface-raised, #252526);
  }
  .bar strong { font-weight: 500; }
  .bar .state { font-size: 11px; color: var(--coterm-text-muted, #969696); }
  .bar .spacer { margin-left: auto; }
  .bar button { font-size: 11px; padding: 4px 10px; }
  .terminal { flex: 1; min-height: 0; overflow: hidden; padding: 10px 4px 0 10px; }
  .notice { padding: 7px 14px; font-size: 12px; color: var(--coterm-text-subtle, #aaaaaa); overflow-wrap: anywhere; }
  .notice:empty { display: none; }
`;

interface Status {
  shellId: string | null;
  shell: string;
  family: "posix" | "powershell" | "cmd" | "unknown";
  cwd: string;
  rows: number;
  cols: number;
  pid: number | null;
  running: boolean;
  exitCode: number | null;
  cursor: string;
}

interface OutputPage {
  shellId: string | null;
  data: string;
  cursor: string;
  gap: boolean;
  more: boolean;
  closed: boolean;
}

interface OutputEvent {
  shellId: string;
  data: string;
  cursor: string;
}

/** 一个宿主元素 + 影子根；React 重挂载时互不影响。 */
function surface(container: HTMLElement, withTerminalStyle = false): ShadowRoot {
  const element = document.createElement("div");
  element.style.height = "100%";
  container.replaceChildren(element);
  const root = element.attachShadow({ mode: "open" });
  const style = document.createElement("style");
  style.textContent = css;
  root.append(style);
  if (withTerminalStyle) {
    const xtermStyle = document.createElement("style");
    xtermStyle.textContent = terminalCSS;
    root.append(xtermStyle);
  }
  return root;
}

function caption(text: string): HTMLLabelElement {
  const label = document.createElement("label");
  const span = document.createElement("span");
  span.textContent = text;
  label.append(span);
  return label;
}

/** 只替换文案节点，语言切换不重建控件。 */
function retitle(label: HTMLLabelElement, text: string) {
  const span = label.querySelector("span");
  if (span) span.textContent = text;
}

function encodeBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function decodeBase64(text: string): Uint8Array {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function unwrap<T>(receipt: OperationReceipt<T>): T {
  if (receipt.status !== "completed" || receipt.error || receipt.output === null) {
    throw new Error(receipt.error?.message ?? `Local operation finished as ${receipt.status}`);
  }
  return receipt.output;
}

/** 配置页只负责 Shell 与工作目录；探测和校验都在驱动后端完成。 */
export function configure(container: HTMLElement, api: DriverConfigurationAPI): DriverFrontend {
  const t = createTranslator(api.i18n);
  const root = surface(container);
  const form = document.createElement("form");
  const controller = new AbortController();

  const shellLabel = caption(t("Shell"));
  const shell = document.createElement("input");
  shell.type = "text";
  shell.spellcheck = false;
  shell.placeholder = isWindows() ? "pwsh.exe" : "/bin/bash";
  shellLabel.append(shell, hint(t("Leave empty to detect the system shell.")));

  const quick = document.createElement("div");
  quick.className = "quick";
  quick.setAttribute("role", "group");
  const quickLabel = document.createElement("p");
  quickLabel.className = "hint";
  quickLabel.textContent = t("Common shells");
  for (const candidate of isWindows()
    ? ["pwsh.exe", "powershell.exe", "cmd.exe"]
    : ["/bin/bash", "/bin/zsh", "/bin/sh"]) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = candidate;
    button.addEventListener("click", () => {
      shell.value = candidate;
      shell.focus();
    }, { signal: controller.signal });
    quick.append(button);
  }

  const cwdLabel = caption(t("Working directory"));
  const cwd = document.createElement("input");
  cwd.type = "text";
  cwd.spellcheck = false;
  cwd.placeholder = isWindows() ? "C:\\Users\\me\\project" : "/home/me/project";
  cwdLabel.append(cwd, hint(t("Leave empty to start in your home directory.")));

  const error = document.createElement("p");
  error.className = "error";
  error.setAttribute("role", "alert");
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "primary";
  submit.textContent = t("Open terminal");
  form.append(shellLabel, quickLabel, quick, cwdLabel, error, submit);
  root.append(form);

  let disposed = false;
  let busy = false;

  const stopLocale = api.i18n?.subscribe(() => {
    const next = createTranslator(api.i18n);
    retitle(shellLabel, next("Shell"));
    retitle(cwdLabel, next("Working directory"));
    quickLabel.textContent = next("Common shells");
    submit.textContent = next("Open terminal");
    shellLabel.querySelector("p")!.textContent = next("Leave empty to detect the system shell.");
    cwdLabel.querySelector("p")!.textContent = next("Leave empty to start in your home directory.");
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (busy || disposed) return;
    busy = true;
    submit.disabled = true;
    error.textContent = "";
    try {
      await api.submit({ shell: shell.value.trim(), cwd: cwd.value.trim() });
    } catch (reason) {
      if (!disposed) error.textContent = String(reason);
    } finally {
      if (!disposed) {
        busy = false;
        submit.disabled = false;
      }
    }
  }, { signal: controller.signal });

  return {
    unmount() {
      disposed = true;
      stopLocale?.();
      controller.abort();
      container.replaceChildren();
    },
  };
}

function isWindows(): boolean {
  return /Windows/i.test(navigator.userAgent);
}

function hint(text: string): HTMLParagraphElement {
  const element = document.createElement("p");
  element.className = "hint";
  element.textContent = text;
  return element;
}

/** 实例界面：一个人机共用的真实终端。 */
export function mount(container: HTMLElement, api: DriverAPI): DriverFrontend {
  let t = createTranslator(api.i18n);
  const root = surface(container, true);
  const layout = document.createElement("div");
  layout.className = "layout";

  const bar = document.createElement("div");
  bar.className = "bar";
  const label = document.createElement("strong");
  const state = document.createElement("span");
  state.className = "state";
  const spacer = document.createElement("span");
  spacer.className = "spacer";
  const restart = document.createElement("button");
  restart.type = "button";
  restart.hidden = true;
  bar.append(label, state, spacer, restart);

  const terminalContainer = document.createElement("div");
  terminalContainer.className = "terminal";
  const notice = document.createElement("div");
  notice.className = "notice";
  notice.setAttribute("role", "status");
  layout.append(bar, terminalContainer, notice);
  root.append(layout);

  const terminal = new Terminal({
    allowProposedApi: true,
    cursorBlink: true,
    scrollback: 3000,
    fontSize: 13,
    fontFamily: 'Consolas, "Liberation Mono", monospace',
    theme: readTerminalTheme(terminalContainer),
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  const searchAddon = new SearchAddon({ highlightLimit: TERMINAL_SEARCH_HIGHLIGHT_LIMIT });
  terminal.loadAddon(searchAddon);
  terminal.open(terminalContainer);
  const stopTheme = attachTerminalTheme(terminalContainer, (theme) => { terminal.options.theme = theme; });
  const searchView = mountTerminalSearch(bar, terminal, searchAddon, (message, values) => t(message, values));
  const stopShortcuts = attachTerminalShortcuts(terminal, () => {
    if (!disposed) notice.textContent = t("Could not copy terminal selection. Try copying again.");
  }, searchView.open);

  let disposed = false;
  let shellId: string | null = null;
  let cursor = 0n;
  let running = false;
  let resizing = false;
  let status_: Status | null = null;
  let unsubscribe: (() => void) | undefined;
  let updates = Promise.resolve();
  let resizeTimer = 0;
  let selectionTimer = 0;
  let selections = Promise.resolve();
  const controller = new AbortController();

  const enqueue = (task: () => Promise<void> | void) => {
    updates = updates.then(task).catch((reason) => {
      if (!disposed) notice.textContent = String(reason);
    });
  };

  const invoke = async <T,>(operation: string, args: unknown): Promise<T> =>
    unwrap(await api.invoke<T>(operation, args));

  const applyLocale = () => {
    t = createTranslator(api.i18n);
    searchView.refreshLocale();
    label.textContent = t("Local");
    restart.textContent = t("Restart shell");
    renderState();
  };

  const renderState = () => {
    if (running) {
      state.textContent = t("running");
      restart.hidden = true;
      return;
    }
    state.textContent = status_?.exitCode != null ? t("exit {code}", { code: status_.exitCode }) : t("exited");
    restart.hidden = false;
    notice.textContent = t("The shell exited. Restart it to keep using this instance.");
  };

  const applyStatus = (next: Status) => {
    status_ = next;
    running = next.running;
    if (next.shellId) shellId = next.shellId;
    label.textContent = `${t("Local")} · ${next.family}${next.pid ? ` · ${next.pid}` : ""}`;
    renderState();
  };

  /** 从 0 重放整段缓冲；缺口、重启或事件跳号时走这条路重新对齐。 */
  const replay = async () => {
    shellId = null;
    cursor = 0n;
    terminal.reset();
    for (;;) {
      const page = await invoke<OutputPage>("local.read", { cursor: cursor.toString() });
      if (disposed) return;
      shellId = page.shellId;
      terminal.write(decodeBase64(page.data));
      cursor = BigInt(page.cursor);
      if (!page.more) break;
    }
  };

  const refreshSnapshot = async () => {
    let next = await invoke<Status>("local.status", {});
    if (disposed) return;
    if (next.shellId === null) {
      const started = await invoke<{ status: Status }>("local.restart", {});
      if (disposed) return;
      next = started.status;
    }
    // 换了 Shell（首次进入或重启）时按当前缓冲重建终端内容。
    const previous = shellId;
    applyStatus(next);
    if (next.shellId !== previous) await replay();
    await resize();
  };

  const resize = async () => {
    if (disposed || resizing) return;
    resizing = true;
    try {
      fit.fit();
      if (running && terminal.rows > 0 && terminal.cols > 0) {
        await invoke("local.resize", { rows: terminal.rows, cols: terminal.cols });
      }
    } catch (reason) {
      // 尺寸同步失败不该打断终端输入；退出状态由状态事件纠正。
      if (!disposed) notice.textContent = String(reason);
    } finally {
      resizing = false;
    }
  };

  const startEvents = async () => {
    const cancel = await api.subscribeEvents((event) => enqueue(async () => {
      if (event.name === "runtime.closed") {
        running = false;
        renderState();
        notice.textContent = t("Driver connection closed.");
        return;
      }
      if (event.name === "local.status") {
        const payload = event.payload as { shellId?: string; running?: boolean };
        if (typeof payload?.running === "boolean") {
          running = payload.running;
          renderState();
        }
        return;
      }
      if (event.name !== "local.terminal.output") return;
      const payload = event.payload as OutputEvent;
      if (typeof payload?.data !== "string" || typeof payload?.cursor !== "string" || typeof payload?.shellId !== "string") {
        throw new Error("Invalid local output event");
      }
      // 重启前的迟到事件必须丢弃，跳号则重新对齐缓冲。
      if (shellId !== null && payload.shellId !== shellId) return;
      if (shellId === null) shellId = payload.shellId;
      const next = BigInt(payload.cursor);
      if (next <= cursor) return;
      if (next !== cursor + 1n) {
        await replay();
        return;
      }
      terminal.write(decodeBase64(payload.data));
      cursor = next;
    }));
    if (disposed) {
      cancel();
      return;
    }
    unsubscribe = cancel;
    await enqueue(refreshSnapshot);
  };

  const sendInput = async (text: string) => {
    if (!running || !text) return;
    try {
      await invoke("local.input", { data: encodeBase64(new TextEncoder().encode(text)) });
    } catch (reason) {
      const message = String(reason);
      notice.textContent = /queue is full/i.test(message) ? t("Input queue is full. Wait before typing or pasting more.") : message;
    }
  };

  restart.addEventListener("click", async () => {
    if (disposed) return;
    restart.disabled = true;
    try {
      const output = await invoke<{ status: Status }>("local.restart", {});
      if (disposed) return;
      notice.textContent = "";
      applyStatus(output.status);
      await replay();
      await resize();
    } catch (reason) {
      if (!disposed) notice.textContent = String(reason);
    } finally {
      if (!disposed) restart.disabled = false;
    }
  }, { signal: controller.signal });

  const onSelection = terminal.onSelectionChange(() => {
    window.clearTimeout(selectionTimer);
    selectionTimer = window.setTimeout(() => {
      if (disposed || !running || !shellId) return;
      const target = shellId;
      const bytes = new TextEncoder().encode(terminal.getSelection());
      let end = Math.min(bytes.length, 16384);
      while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
      const text = new TextDecoder().decode(bytes.subarray(0,end));
      selections = selections.then(async () => {
        if (disposed || !running || shellId !== target) return;
        await invoke("local.select_lines", {shellId:target,text,truncated:end<bytes.length});
      }).catch(reason => {if (!disposed) notice.textContent = String(reason);});
    },100);
  });
  const onData = terminal.onData((text) => { void sendInput(text); });
  const observer = new ResizeObserver(() => {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => { void resize(); }, 100);
  });
  observer.observe(terminalContainer);
  const stopLocale = api.i18n?.subscribe(() => applyLocale());
  applyLocale();
  void startEvents().catch((reason) => {
    if (!disposed) notice.textContent = String(reason);
  });
  requestAnimationFrame(() => {
    if (disposed) return;
    void resize();
    terminal.focus();
  });

  return {
    unmount() {
      disposed = true;
      controller.abort();
      stopLocale?.();
      unsubscribe?.();
      window.clearTimeout(resizeTimer);
      observer.disconnect();
      onData.dispose();
      onSelection.dispose();
      window.clearTimeout(selectionTimer);
      if (running && shellId) void invoke("local.select_lines", {shellId,text:"",truncated:false}).catch(() => {});
      stopShortcuts();
      searchView.dispose();
      stopTheme();
      terminal.dispose();
      container.replaceChildren();
    },
  };
}
