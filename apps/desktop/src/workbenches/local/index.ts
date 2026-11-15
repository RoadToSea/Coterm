import { viewNodes } from "../view";
import { attachTerminalTheme, readTerminalTheme } from "../common/terminal-theme";
import { attachTerminalShortcuts } from "../common/terminal-shortcuts";
import { mountTerminalSearch, TERMINAL_SEARCH_HIGHLIGHT_LIMIT } from "../common/terminal-search";
import type { OperationReceipt, DriverAPI, DriverFrontend } from "../../../../shared/index";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Terminal } from "@xterm/xterm";

import { createTranslator } from "./i18n";

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

/** 实例界面：一个人机共用的真实终端。 */
export function mount(container: HTMLElement, api: DriverAPI): DriverFrontend {
  let t = createTranslator(api.i18n);
  const node = viewNodes(container);
  const bar = node<HTMLDivElement>('bar');
  const label = node<HTMLElement>('label');
  const state = node<HTMLSpanElement>('state');
  const restart = node<HTMLButtonElement>('restart');
  const terminalContainer = node<HTMLDivElement>('terminalContainer');
  const notice = node<HTMLDivElement>('notice');
  const terminal = new Terminal({
    allowProposedApi: true,
    cursorBlink: true,
    scrollback: 3000,
    fontSize: 13,
    fontFamily: getComputedStyle(terminalContainer).getPropertyValue('--coterm-font-mono'),
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
  let running: boolean | null = null;
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
    restart.title = t("The shell exited. Restart it to keep using this instance.");
    restart.setAttribute("aria-description", restart.title);
    renderState();
  };

  const renderState = () => {
    // No lifecycle fact is available until the initial status snapshot arrives.
    if (running === null) {
      restart.hidden = true;
      return;
    }
    if (running) {
      state.textContent = t("running");
      restart.hidden = true;
      return;
    }
    state.textContent = status_?.exitCode != null ? t("exit {code}", { code: status_.exitCode }) : t("exited");
    restart.hidden = false;
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
        const payload = event.payload as Partial<Pick<Status, "shellId" | "running" | "exitCode">>;
        if (status_ && payload?.shellId === shellId && typeof payload.running === "boolean") {
          applyStatus({
            ...status_,
            running: payload.running,
            exitCode: payload.exitCode === undefined ? status_.exitCode : payload.exitCode,
          });
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

    },
  };
}
