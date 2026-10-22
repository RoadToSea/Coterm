import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import terminalCSS from "@xterm/xterm/css/xterm.css?inline";
import type { PluginAPI, PluginConfigurationAPI, PluginFrontend, OperationReceipt } from "@coterm/plugin-sdk";

const css = `
  :host {
    display: block;
    height: 100%;
    min-height: 0;
    color: var(--coterm-text-primary, #cccccc);
    font: 13px var(--coterm-font-sans, "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif);
  }
  * { box-sizing: border-box; }
  button, input, textarea { font: inherit; color: inherit; }
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
    color: var(--coterm-text-bright, #ffffff);
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
  .terminal { flex: 1; min-height: 0; overflow: hidden; padding: 10px 4px 0 10px; }
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
  .proposal textarea { resize: vertical; font-family: var(--coterm-font-mono, Consolas, monospace); min-height: 36px; }
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

function field(caption: string, value = "", type = "text"): { label: HTMLLabelElement; input: HTMLInputElement } {
  const label = document.createElement("label");
  label.textContent = caption;
  const input = document.createElement("input");
  input.type = type;
  input.value = value;
  input.required = true;
  label.append(input);
  return { label, input };
}

/** The plugin owns configuration fields; the base never learns SSH field semantics. */
export function configure(container: HTMLElement, api: PluginConfigurationAPI): PluginFrontend {
  const root = surface(container);
  const form = document.createElement("form");
  const host = field("Host");
  const port = field("Port", "22", "number");
  port.input.min = "1";
  port.input.max = "65535";
  const username = field("Username");
  const pair = document.createElement("div");
  pair.className = "pair";
  pair.append(host.label, port.label);
  const error = document.createElement("p");
  error.className = "error";
  error.setAttribute("role", "alert");
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "primary";
  submit.textContent = "Open SSH";
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
  return { unmount() { disposed = true; controller.abort(); container.replaceChildren(); } };
}

interface Status { config: { host: string; port: number; username: string }; connected: boolean; connectionId: string | null }
interface Output { data: string; cursor: string; gap: boolean; closed: boolean; more: boolean; connectionId: string }
interface PushedOutput { data: string; cursor: string; connectionId: string }

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
  state.textContent = "Loading…";
  const connect = document.createElement("button");
  connect.textContent = "Connect";
  connect.disabled = true;
  bar.append(label, state, connect);
  const terminalContainer = document.createElement("div");
  terminalContainer.className = "terminal";
  const notice = document.createElement("div");
  notice.className = "notice";
  notice.setAttribute("role", "status");
  layout.append(bar, terminalContainer, notice);
  root.append(layout);

  const terminal = new Terminal({
    cursorBlink: true, scrollback: 3000, fontSize: 13,
    fontFamily: 'Consolas, "Liberation Mono", monospace',
    theme: { background: "#1e1e1e", foreground: "#cccccc", cursor: "#cccccc" },
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  terminal.open(terminalContainer);
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

  const invoke = async <T = unknown>(operation: string, args: unknown): Promise<T> => output(await api.invoke<T>(operation, args));
  const showError = (reason: unknown) => { if (!disposed) notice.textContent = String(reason); };
  const setConnected = (next: boolean) => {
    connected = next;
    state.textContent = next ? "Connected" : "Disconnected";
    connect.textContent = next ? "Disconnect" : "Connect";
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
  const refreshSnapshot = async () => {
    const status = await invoke<Status>("ssh.status", {});
    if (disposed) return;
    label.textContent = `${status.config.username}@${status.config.host}:${status.config.port}`;
    if (connectionId !== status.connectionId) {
      connectionId = status.connectionId;
      cursor = "0";
      if (connectionId) terminal.reset();
    }
    setConnected(status.connected);
    connect.disabled = working;
    if (!connectionId) return;
    // Replay only on mount, connection change or a reported gap; never idle polling.
    for (let batch = 0; batch < 64 && !disposed; batch++) {
      const data = await invoke<Output>("ssh.read", { cursor });
      if (disposed || data.connectionId !== connectionId) return;
      if (data.gap) {
        terminal.reset();
        terminal.write("\r\n[Output gap: older data expired from replay buffer]\r\n");
      }
      await write(data.data);
      cursor = data.cursor;
      if (!data.more) break;
      if (batch === 63) notice.textContent = "Replay limit reached; waiting for the next output event.";
    }
    if (connected) resize();
  };
  const enqueue = (action: () => Promise<void>) => {
    const next = updates.then(async () => { if (!disposed) await action(); });
    updates = next.catch(showError);
    return updates;
  };
  const startEvents = async () => {
    const cancel = await api.subscribeEvents((event) => enqueue(async () => {
      if (event.name === "runtime.closed") { setConnected(false); notice.textContent = "Plugin connection closed."; return; }
      if (event.name === "runtime.gap" || event.name === "ssh.status") { await refreshSnapshot(); return; }
      if (event.name !== "ssh.output") return;
      const data = event.payload as PushedOutput;
      if (typeof data.data !== "string" || typeof data.connectionId !== "string" || typeof data.cursor !== "string" || !/^\d+$/.test(data.cursor)) throw new Error("Invalid SSH output event");
      if (data.connectionId !== connectionId) await refreshSnapshot();
      if (data.connectionId !== connectionId || BigInt(data.cursor) <= BigInt(cursor)) return;
      if (BigInt(data.cursor) !== BigInt(cursor) + 1n) { await refreshSnapshot(); return; }
      await write(data.data);
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
        const confirmation = await api.requestConfirmation({ title: "Verify SSH server", prompt: `${label.textContent}\n\n${probe.fingerprint}\n\nCompare this fingerprint with the server administrator before continuing.` });
        if (disposed || confirmation.outcome !== "submitted" || !confirmation.value) return;
        const secret = await api.requestSecret({ title: "SSH authentication", prompt: `Password for ${label.textContent}`, purpose: "ssh.password", allowCancel: true, timeoutMs: 60_000 });
        if (disposed || secret.outcome !== "submitted") return;
        await invoke("ssh.connect", { fingerprint: probe.fingerprint, secret: secret.value });
        if (disposed) return;
        await enqueue(refreshSnapshot);
        terminal.focus();
      }
    } catch (reason) { showError(reason); }
    finally { if (!disposed) { working = false; connect.disabled = false; } }
  }, { signal: controller.signal });

  const onData = terminal.onData((text) => {
    if (!connected || working || disposed) return;
    const data = new TextEncoder().encode(text);
    if (queuedBytes + data.length > 16 * 1024) { notice.textContent = "Input queue is full. Wait before typing or pasting more."; return; }
    queuedBytes += data.length;
    inputQueue = inputQueue.then(async () => {
      if (!disposed && connected) await invoke("ssh.input", { data: base64(data) });
    }).catch(showError).finally(() => { queuedBytes -= data.length; });
  });
  const observer = new ResizeObserver(() => {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(resize, 100);
  });
  observer.observe(terminalContainer);
  void startEvents().catch(showError);

  return { unmount() {
    disposed = true;
    controller.abort();
    unsubscribe?.();
    window.clearTimeout(resizeTimer);
    observer.disconnect();
    onData.dispose();
    terminal.dispose();
    container.replaceChildren();
  } };
}
