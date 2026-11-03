import { attachTerminalPreferences, attachTerminalTheme, readTerminalPreferences, readTerminalTheme } from "@coterm/plugin-sdk/components/terminal-theme";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import terminalCSS from "@xterm/xterm/css/xterm.css?inline";
import type { PluginAPI, PluginConfigurationAPI, PluginFrontend, OperationReceipt } from "@coterm/plugin-sdk";
import { createTranslator } from "./i18n";
import { mountSshBlocks } from "./blocks";
import { mountJobs } from "./jobs";
import { observeHumanCommands } from "./human-commands";
import { observeBlockPrompt } from "./observed-boundaries";
import { mountTerminalBlocks } from "@coterm/plugin-sdk/components/terminal-blocks";
import { attachTerminalShortcuts } from "@coterm/plugin-sdk/components/terminal-shortcuts";
import { mountTerminalSearch, TERMINAL_SEARCH_HIGHLIGHT_LIMIT } from "@coterm/plugin-sdk/components/terminal-search";

const css = `
  :host {
    display: block;
    height: 100%;
    min-height: 0;
    color: var(--coterm-text-primary, #cccccc);
    font: var(--coterm-ui-font-size, 13px) var(--coterm-font-sans, "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif);
  }
  * { box-sizing: border-box; }
  button, input, textarea { font: inherit; color: inherit; }
  form input, form button { border-radius: var(--radius, 8px); }
  button {
    border: 1px solid var(--coterm-border-input, #454545);
    padding: 5px 12px;
    background: var(--coterm-surface-sidebar, #333333);
    cursor: pointer;
  }
  button:hover { background: var(--coterm-surface-hover, #414141); }
  button:disabled { opacity: .5; cursor: default; }
  button:focus-visible, input:focus-visible, textarea:focus-visible {
    outline: 1px solid var(--coterm-focus, #007fd4);
  }
  .primary {
    background: var(--coterm-accent, #0e639c);
    color: var(--coterm-accent-foreground, #ffffff);
  }
  .primary:hover { background: var(--coterm-accent-hover, #1177bb); }
  form { display: grid; gap: 14px; margin-bottom: 20px; }
  label { display: grid; gap: 6px; color: var(--coterm-text-subtle, #aaaaaa); }
  input, textarea {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--coterm-border-input, #454545);
    background: var(--coterm-surface, #1e1e1e);
    color: var(--coterm-text-primary, #cccccc);
    padding: 7px 9px;
  }
  .pair { display: grid; grid-template-columns: minmax(0, 1fr) 90px; gap: 12px; }
  .error { color: var(--coterm-danger, #f48771); overflow-wrap: anywhere; margin: 0; }
  .layout { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--coterm-surface, #1e1e1e); }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    min-height: 42px;
    border-bottom: 1px solid var(--coterm-border, #333333);
    padding: 6px 14px;
    background: var(--coterm-surface-raised, #252526);
  }
  .bar strong { font-weight: 500; color: var(--coterm-text-primary, #cccccc); }
  .bar span { font-size: 11px; color: var(--coterm-text-muted, #969696); }
  .bar button { margin-left: auto; font-size: 11px; }
  .terminal-pane { flex:1; min-width:0; min-height:0; display:flex; flex-direction:column; }
  .terminal-shell { position:relative; flex:1; min-width:0; min-height:0; display:flex; }
  /* xterm also uses .terminal; its own padding is measured by FitAddon. */
  .terminal-shell > .terminal { position:relative; flex:1; min-width:0; min-height:0; overflow:hidden; }
  .terminal-shell > .terminal > .xterm { padding:28px 4px 0 5px; }
  .ssh-blocks-error { padding:6px 10px; font-size:11px; }
  .ssh-blocks-error[hidden] { display:none; }
  .ssh-block-history { padding:4px 10px; color:var(--coterm-text-muted,#969696); font-size:10px; }
  .ssh-block-history[hidden] { display:none; }
  .notice { padding: 7px 14px; font-size: 12px; color: var(--coterm-text-subtle, #aaaaaa); overflow-wrap: anywhere; }
  .notice:empty, .approvals:empty { display: none; }
  .approvals {
    flex: none;
    max-height: 180px;
    overflow: auto;
    border-bottom: 1px solid var(--coterm-border-subtle, #454545);
    padding: 10px 14px;
  }
  .proposal { display: grid; gap: 6px; margin-bottom: 8px; }
  .proposal:last-child { margin-bottom: 0; }
  .proposal div { display: flex; gap: 8px; }
  .proposal textarea { resize: vertical; font-family: var(--coterm-font-mono, Consolas, monospace); font-size: var(--coterm-terminal-font-size, 13px); min-height: 36px; }
`;

function surface(container: HTMLElement): ShadowRoot {
  // A new host element keeps React StrictMode remounts independent.
  const element = document.createElement("div");
  element.style.height = "100%";
  container.replaceChildren(element);
  const root = element.attachShadow({ mode: "open" });
  const style = document.createElement("style");
  style.textContent = css;
  root.append(style);
  return root;
}

function field(caption: string, value = "", type = "text"): { label: HTMLLabelElement; caption: HTMLSpanElement; input: HTMLInputElement } {
  const label = document.createElement("label");
  const text = document.createElement("span");
  text.textContent = caption;
  const input = document.createElement("input");
  input.type = type;
  input.value = value;
  input.required = true;
  label.append(text, input);
  return { label, caption: text, input };
}

/** The plugin owns configuration fields; the base never learns SSH field semantics. */
export function configure(container: HTMLElement, api: PluginConfigurationAPI): PluginFrontend {
  const t = createTranslator(api.i18n);
  const root = surface(container);
  const form = document.createElement("form");
  const host = field(t("Host"));
  const port = field(t("Port"), "22", "number");
  port.input.min = "1";
  port.input.max = "65535";
  const username = field(t("Username"));
  const pair = document.createElement("div");
  pair.className = "pair";
  pair.append(host.label, port.label);
  const error = document.createElement("p");
  error.className = "error";
  error.setAttribute("role", "alert");
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "primary";
  submit.textContent = t("Open SSH");
  const stopLocale = api.i18n?.subscribe(() => {
    host.caption.textContent = t("Host");
    port.caption.textContent = t("Port");
    username.caption.textContent = t("Username");
    submit.textContent = t("Open SSH");
  });
  form.append(pair, username.label, error, submit);
  root.append(form);
  let disposed = false;
  let busy = false;
  const controller = new AbortController();
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (busy || disposed) return;
    busy = true;
    submit.disabled = true;
    try {
      await api.submit({ host: host.input.value.trim(), port: Number(port.input.value), username: username.input.value.trim() });
    } catch (reason) {
      if (!disposed) error.textContent = String(reason);
    } finally {
      if (!disposed) { busy = false; submit.disabled = false; }
    }
  }, { signal: controller.signal });
  return { unmount() { disposed = true; stopLocale?.(); controller.abort(); container.replaceChildren(); } };
}

interface Status { config: { host: string; port: number; username: string }; connected: boolean; connectionId: string | null }
interface Output { data: string; cursor: string; gap: boolean; closed: boolean; more: boolean; connectionId: string; annotation?: unknown }
interface PushedOutput { data: string; cursor: string; connectionId: string; annotation?: unknown }

function output<T>(receipt: OperationReceipt<T>): T {
  if (receipt.status !== "completed" || receipt.error || receipt.output === null) throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
  return receipt.output;
}

function base64(data: Uint8Array): string {
  let binary = "";
  for (const byte of data) binary += String.fromCharCode(byte);
  return btoa(binary);
}

export function mount(container: HTMLElement, api: PluginAPI): PluginFrontend {
  const t = createTranslator(api.i18n);
  const root = surface(container);
  const xtermStyle = document.createElement("style");
  xtermStyle.textContent = terminalCSS;
  root.append(xtermStyle);
  const layout = document.createElement("div");
  layout.className = "layout";
  const bar = document.createElement("div");
  bar.className = "bar";
  const label = document.createElement("strong");
  label.textContent = "SSH";
  const state = document.createElement("span");
  let stateMessage = "Loading…";
  state.textContent = t(stateMessage);
  const connect = document.createElement("button");
  connect.textContent = t("Connect");
  connect.disabled = true;
  bar.append(label, state, connect);
  const workbench = document.createElement("div"); workbench.className = "ssh-workbench";
  const terminalPane = document.createElement("div"); terminalPane.className = "terminal-pane";
  const shell = document.createElement("div"); shell.className = "terminal-shell";
  const gutter = document.createElement("div");
  const terminalContainer = document.createElement("div"); terminalContainer.className = "terminal";
  const jobsBar = document.createElement("div");
  const blocksControls = document.createElement("div");
  shell.append(gutter, terminalContainer); terminalPane.append(shell, jobsBar, blocksControls); workbench.append(terminalPane);
  const notice = document.createElement("div");
  notice.className = "notice";
  notice.setAttribute("role", "status");
  layout.append(bar, workbench, notice);
  root.append(layout);

  const terminal = new Terminal({
    allowProposedApi: true,
    ...readTerminalPreferences(terminalContainer), scrollback: 3000,
    fontFamily: 'Consolas, "Liberation Mono", monospace',
    theme: readTerminalTheme(terminalContainer),
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  const searchAddon = new SearchAddon({ highlightLimit: TERMINAL_SEARCH_HIGHLIGHT_LIMIT });
  terminal.loadAddon(searchAddon);
  terminal.open(terminalContainer);
  const humanCommands = observeHumanCommands(terminal);
  const stopTheme = attachTerminalTheme(terminalContainer, (theme) => { terminal.options.theme = theme; });
  const stopPreferences = attachTerminalPreferences(terminalContainer, (preferences) => {
    Object.assign(terminal.options, preferences);
    resize();
  });
  let disposed = false;
  let connected = false;
  let working = false;
  let cursor = "0";
  let connectionId: string | null = null;
  let unsubscribe: (() => void) | undefined;
  let updates = Promise.resolve();
  let resizeTimer = 0;
  let queuedBytes = 0;
  let inputQueue = Promise.resolve();
  const controller = new AbortController();
  const terminalBlocks = mountTerminalBlocks(gutter, terminalContainer, terminal, {
    cancelLabel: t("Clear selection"),
    onSelectionChange(ids) { blocksView.select(ids); jobsView.selection(ids); },
  });
  const observedBoundaries = observeBlockPrompt(terminal, (id, offset) => terminalBlocks.end(id, offset));
  const jobsView = mountJobs(workbench, terminalPane, jobsBar, api, t, id => terminalBlocks.toggle(id), reason => showError(reason));
  const blocksView = mountSshBlocks(blocksControls, api, t, blocks => {
    terminalBlocks.update(blocks); jobsView.setBlocks(blocks);
  }, () => { observedBoundaries.reset(); terminalBlocks.reset(); jobsView.reset(); });
  const searchView = mountTerminalSearch(bar, terminal, searchAddon, t);
  const stopShortcuts = attachTerminalShortcuts(terminal, () => {
    if (!disposed) notice.textContent = t("Could not copy terminal selection. Try copying again.");
  }, searchView.open);

  const invoke = async <T = unknown>(operation: string, args: unknown): Promise<T> => output(await api.invoke<T>(operation, args));
  const showError = (reason: unknown) => { if (!disposed) notice.textContent = String(reason); };
  const setConnected = (next: boolean) => {
    if (!next) humanCommands.suspend(false);
    connected = next;
    stateMessage = next ? "Connected" : "Disconnected";
    state.textContent = t(stateMessage);
    connect.textContent = next ? t("Disconnect") : t("Connect");
  };
  const resize = () => {
    if (disposed || terminalContainer.clientWidth === 0 || terminalContainer.clientHeight === 0) return;
    fit.fit();
    if (connected) void invoke("ssh.resize", { cols: terminal.cols, rows: terminal.rows }).catch(showError);
  };
  const write = async (data: string) => {
    if (!data || disposed) return;
    const decoded = atob(data);
    await new Promise<void>((resolve) => terminal.write(Uint8Array.from(decoded, (char) => char.charCodeAt(0)), resolve));
  };
  const writeFrame = async (frame: {data: string; annotation?: unknown}) => {
    const value = frame.annotation;
    if (value && typeof value === "object") {
      const annotation = value as Record<string, unknown>;
      if (typeof annotation.id !== "string" || annotation.id.length > 128
        || typeof annotation.index !== "number" || !Number.isSafeInteger(annotation.index) || annotation.index < 1
        || !["human", "robot"].includes(String(annotation.actor))
        || !["begin", "end"].includes(String(annotation.kind))) throw new Error("Invalid terminal block annotation");
      if (annotation.kind === "begin") {
        const block = {id:annotation.id,index:annotation.index,command:"",actor:annotation.actor as "human" | "robot"};
        const observed = annotation.boundary === "observed";
        const offset = observed ? annotation.lineOffset : 0;
        if (typeof offset !== "number" || !Number.isInteger(offset) || offset > 0 || offset < -64) throw new Error("Invalid terminal block offset");
        terminalBlocks.begin(block, offset);
        blocksView.invalidate();
        if (observed) {
          observedBoundaries.begin(block.id, offset);
          // The command is already echoed on this row. Adding a label here would
          // alter the prompt/cursor and corrupt readline editing or replay.
          await write(frame.data);
          return;
        }
        humanCommands.suspend(true);
        observedBoundaries.reset();
        const command = (await blocksView.command(block.id)).replace(/[\u0000-\u001f\u007f-\u009f]/g, " ");
        const label = block.actor === "human" ? "[👤 Human]" : "[🤖 Agent]";
        const prefix = block.actor === "human" ? "\x1b[38;2;52;211;153m" : "\x1b[38;2;56;189;248m";
        // Display metadata stays local; it never goes to ssh.input.
        await new Promise<void>(resolve => terminal.write(prefix + label + "\x1b[0m " + command + "\r\n", resolve));
      } else {
        observedBoundaries.end(annotation.id);
        terminalBlocks.end(annotation.id);
        if (annotation.boundary !== "observed") humanCommands.suspend(false);
      }
    }
    await write(frame.data);
  };
  const refreshSnapshot = async () => {
    const status = await invoke<Status>("ssh.status", {});
    if (disposed) return;
    label.textContent = `${status.config.username}@${status.config.host}:${status.config.port}`;
    if (connectionId !== status.connectionId) {
      observedBoundaries.reset();
      humanCommands.suspend(false);
      connectionId = status.connectionId;
      cursor = "0";
      if (connectionId) { searchView.close(false); terminalBlocks.reset(); jobsView.reset(); terminal.reset(); }
    }
    setConnected(status.connected);
    connect.disabled = working;
    if (!connectionId) return;
    // Replay only on mount, connection change or a reported gap; never idle polling.
    for (let batch = 0; batch < 256 && !disposed; batch++) {
      const data = await invoke<Output>("ssh.read", { cursor, terminal: true });
      if (disposed || data.connectionId !== connectionId) return;
      if (data.gap) {
        observedBoundaries.reset();
        humanCommands.suspend(false);
        terminalBlocks.reset(); terminal.reset();
        terminal.write("\r\n[Output gap: older data expired from replay buffer]\r\n");
      }
      await writeFrame(data);
      cursor = data.cursor;
      if (!data.more) break;
      if (batch === 255) notice.textContent = t("Replay limit reached; waiting for the next output event.");
    }
    blocksView.invalidate();
    if (connected) resize();
  };
  const enqueue = (action: () => Promise<void>) => {
    const next = updates.then(async () => { if (!disposed) await action(); });
    updates = next.catch(showError);
    return updates;
  };
  const startEvents = async () => {
    const cancel = await api.subscribeEvents((event) => enqueue(async () => {
      if (event.name === "runtime.closed") { setConnected(false); jobsView.closed(); notice.textContent = t("Plugin connection closed."); return; }
      if (event.name === "ssh.blocks.changed") { blocksView.invalidate(); return; }
      if (event.name === "runtime.gap" || event.name === "ssh.status") {
        if(event.name === "ssh.status" && (event.payload as {reset?:boolean}).reset) {
          observedBoundaries.reset();
          humanCommands.suspend(false);
          searchView.close(false); terminalBlocks.reset(); terminal.reset();
          const value = (event.payload as {cursor?:unknown}).cursor;
          if(typeof value === "string" && /^\d+$/.test(value)) cursor=value;
        }
        blocksView.invalidate(); await refreshSnapshot(); return;
      }
      if (event.name === "ssh.job") {
        // The plugin announces an exec operation before it finishes; tracking it here
        // is what lets the jobs row appear while the command is still running.
        const payload = event.payload as { operationId?: string; text?: string; done?: boolean };
        if (!payload.operationId) return;
        await jobsView.event(payload);
        return;
      }
      if (event.name !== "ssh.terminal.output") return;
      const data = event.payload as PushedOutput;
      if (typeof data.data !== "string" || typeof data.connectionId !== "string" || typeof data.cursor !== "string" || !/^\d+$/.test(data.cursor)) throw new Error("Invalid SSH output event");
      if (data.connectionId !== connectionId) await refreshSnapshot();
      if (data.connectionId !== connectionId || BigInt(data.cursor) <= BigInt(cursor)) return;
      if (BigInt(data.cursor) !== BigInt(cursor) + 1n) { await refreshSnapshot(); return; }
      await writeFrame(data);
      cursor = data.cursor;
    }));
    if (disposed) { cancel(); return; }
    unsubscribe = cancel;
    await enqueue(refreshSnapshot);
  };

  connect.addEventListener("click", async () => {
    if (working || disposed) return;
    working = true;
    connect.disabled = true;
    notice.textContent = "";
    try {
      if (connected) {
        await invoke("ssh.disconnect", {});
        setConnected(false);
      } else {
        const probe = await invoke<{ fingerprint: string }>("ssh.probe", {});
        if (disposed) return;
        const confirmation = await api.requestConfirmation({ title: t("Verify SSH server"), prompt: `${label.textContent}\n\n${probe.fingerprint}\n\n${t("Compare this fingerprint with the server administrator before continuing.")}` });
        if (disposed || confirmation.outcome !== "submitted" || !confirmation.value) return;
        const secret = await api.requestSecret({ title: t("SSH authentication"), prompt: t("Password for {server}", { server: label.textContent ?? "" }), purpose: "ssh.password", allowCancel: true, timeoutMs: 60_000 });
        if (disposed || secret.outcome !== "submitted") return;
        await invoke("ssh.connect", { fingerprint: probe.fingerprint, secret: secret.value });
        if (disposed) return;
        await enqueue(refreshSnapshot);
        if (!terminalPane.hidden) terminal.focus();
      }
    } catch (reason) { showError(reason); }
    finally { if (!disposed) { working = false; connect.disabled = false; } }
  }, { signal: controller.signal });

  const onData = terminal.onData((text) => {
    if (!connected || working || disposed) return;
    const data = new TextEncoder().encode(text);
    if (queuedBytes + data.length > 16 * 1024) { notice.textContent = t("Input queue is full. Wait before typing or pasting more."); return; }
    queuedBytes += data.length;
    const targetConnection = connectionId;
    inputQueue = inputQueue.then(async () => {
      if (disposed || !connected || targetConnection !== connectionId) return;
      await updates;
      const observation = await humanCommands.beforeInput(text);
      if (disposed || !connected || targetConnection !== connectionId) return;
      humanCommands.sent();
      await invoke("ssh.input", { data: base64(data), connectionId: targetConnection, ...(observation ? { observation } : {}) });
    }).catch(showError).finally(() => { queuedBytes -= data.length; });
  });
  const observer = new ResizeObserver(() => {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(resize, 100);
  });
  observer.observe(terminalContainer);
  const stopLocale = api.i18n?.subscribe(() => {
    terminalBlocks.refreshLocale(t("Clear selection"));
    blocksView.refreshLocale();
    searchView.refreshLocale();
    jobsView.refreshLocale();
    state.textContent = t(stateMessage);
    connect.textContent = t(connected ? "Disconnect" : "Connect");

  });
  void startEvents().catch(showError);

  return { unmount() {
    disposed = true;
    controller.abort();
    blocksView.unmount();
    stopLocale?.();
    unsubscribe?.();
    window.clearTimeout(resizeTimer);
    observer.disconnect();
    jobsView.unmount();
    terminalBlocks.unmount();
    observedBoundaries.dispose();
    humanCommands.dispose();
    onData.dispose();
    stopShortcuts();
    searchView.dispose();
    stopTheme(); stopPreferences();
    terminal.dispose();
    container.replaceChildren();
  } };
}
