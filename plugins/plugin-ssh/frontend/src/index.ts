import { attachTerminalTheme, readTerminalTheme } from "@coterm/plugin-sdk/components/terminal-theme";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import terminalCSS from "@xterm/xterm/css/xterm.css?inline";
import type { PluginAPI, PluginConfigurationAPI, PluginFrontend, OperationReceipt } from "@coterm/plugin-sdk";
import { createTranslator } from "./i18n";
import { attachTerminalShortcuts } from "@coterm/plugin-sdk/components/terminal-shortcuts";
import { mountTerminalSearch, TERMINAL_SEARCH_HIGHLIGHT_LIMIT } from "@coterm/plugin-sdk/components/terminal-search";

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
  .main { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .terminal { flex: 1; min-height: 0; overflow: hidden; padding: 10px 4px 0 10px; }
  .jobs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-top: 1px solid var(--coterm-border, #333333);
    background: var(--coterm-surface-raised, #252526);
    min-height: 34px;
  }
  .jobs:empty { display: none; }
  .jobs-label { font-size: 10px; letter-spacing: .06em; color: var(--coterm-text-muted, #969696); margin-right: 4px; }
  .job {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid var(--coterm-border-input, #454545);
    border-radius: 10px;
    background: var(--coterm-surface, #1e1e1e);
    padding: 3px 10px;
    font-size: 11px;
    max-width: 260px;
  }
  .job:hover { background: var(--coterm-surface-hover, #414141); }
  .job[aria-current="true"] {
    border-color: var(--coterm-focus, #007fd4);
    background: var(--coterm-surface-hover, #414141);
  }
  .job .spin { color: var(--coterm-accent, #0e639c); }
  .job .ok { color: #6a9955; }
  .job .bad { color: var(--coterm-danger, #f48771); }
  .job .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 150px; }
  .job .dismiss { border: 0; background: transparent; padding: 0 2px; color: var(--coterm-text-muted, #969696); }
  .job .dismiss:hover { color: var(--coterm-accent-foreground, #ffffff); background: transparent; }
  .job-view {
    flex: 0 0 auto;
    height: 45%;
    min-height: 100px;
    max-height: 65%;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--coterm-border-subtle, #454545);
    background: var(--coterm-surface, #1e1e1e);
  }
  .job-view[hidden] { display: none; }
  .job-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 10px;
    border-bottom: 1px solid var(--coterm-border, #333333);
    font-size: 11px;
    color: var(--coterm-text-subtle, #aaaaaa);
  }
  .job-head code { color: var(--coterm-text-primary, #cccccc); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .job-head button { margin-left: auto; font-size: 11px; padding: 2px 8px; }
  .job-body { flex: 1; min-height: 0; overflow: auto; padding: 6px 10px; }
  .job-stream { margin: 0 0 8px; }
  .job-stream h4 { margin: 0 0 3px; font-size: 10px; letter-spacing: .06em; color: var(--coterm-text-muted, #969696); }
  .job-stream pre {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: 12px var(--coterm-font-mono, Consolas, "Liberation Mono", monospace);
    color: var(--coterm-text-primary, #cccccc);
  }
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
  const main = document.createElement("div");
  main.className = "main";
  const terminalContainer = document.createElement("div");
  terminalContainer.className = "terminal";
  const jobsBar = document.createElement("div");
  jobsBar.className = "jobs";
  jobsBar.setAttribute("role", "list");
  jobsBar.setAttribute("aria-label", t("Background jobs"));
  main.append(terminalContainer, jobsBar);
  const jobView = document.createElement("section");
  jobView.className = "job-view";
  jobView.hidden = true;
  const jobHead = document.createElement("div");
  jobHead.className = "job-head";
  const jobCommand = document.createElement("code");
  const jobState = document.createElement("span");
  const jobStop = document.createElement("button");
  jobStop.type = "button";
  jobStop.textContent = t("Stop");
  const jobClose = document.createElement("button");
  jobClose.type = "button";
  jobClose.textContent = t("Close view");
  jobHead.append(jobCommand, jobState, jobStop, jobClose);
  const jobBody = document.createElement("div");
  jobBody.className = "job-body";
  jobView.append(jobHead, jobBody);
  const notice = document.createElement("div");
  notice.className = "notice";
  notice.setAttribute("role", "status");
  layout.append(bar, main, jobView, notice);
  root.append(layout);

  const terminal = new Terminal({
    allowProposedApi: true,
    cursorBlink: true, scrollback: 3000, fontSize: 13,
    fontFamily: 'Consolas, "Liberation Mono", monospace',
    theme: readTerminalTheme(terminalContainer),
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  const searchAddon = new SearchAddon({ highlightLimit: TERMINAL_SEARCH_HIGHLIGHT_LIMIT });
  terminal.loadAddon(searchAddon);
  terminal.open(terminalContainer);
  const stopTheme = attachTerminalTheme(terminalContainer, (theme) => { terminal.options.theme = theme; });
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
  const searchView = mountTerminalSearch(bar, terminal, searchAddon, t);
  const stopShortcuts = attachTerminalShortcuts(terminal, () => {
    if (!disposed) notice.textContent = t("Could not copy terminal selection. Try copying again.");
  }, searchView.open);

  const invoke = async <T = unknown>(operation: string, args: unknown): Promise<T> => output(await api.invoke<T>(operation, args));
  const showError = (reason: unknown) => { if (!disposed) notice.textContent = String(reason); };
  const setConnected = (next: boolean) => {
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
  const refreshSnapshot = async () => {
    const status = await invoke<Status>("ssh.status", {});
    if (disposed) return;
    label.textContent = `${status.config.username}@${status.config.host}:${status.config.port}`;
    if (connectionId !== status.connectionId) {
      connectionId = status.connectionId;
      cursor = "0";
      if (connectionId) { searchView.close(false); terminal.reset(); }
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
      if (batch === 63) notice.textContent = t("Replay limit reached; waiting for the next output event.");
    }
    if (connected) resize();
  };
  const enqueue = (action: () => Promise<void>) => {
    const next = updates.then(async () => { if (!disposed) await action(); });
    updates = next.catch(showError);
    return updates;
  };
  interface JobView {
    operationId: string;
    text: string;
    exitCode: number | null;
    done: boolean;
    cancelled: boolean;
    timedOut: boolean;
    stdout: string;
    stderr: string;
    stdoutCursor: string;
    stderrCursor: string;
    more: boolean;
  }
  const jobs = new Map<string, JobView>();
  let selectedJob: string | null = null;

  const jobLabel = (job: JobView) => {
    const first = job.text.trim().split(String.fromCharCode(10))[0] ?? "";
    return first.length > 48 ? `${first.slice(0, 48)}…` : first || job.operationId;
  };
  const jobIcon = (job: JobView) => {
    if (!job.done) return "●";
    if (job.cancelled) return "■";
    if (job.timedOut || (job.exitCode ?? 0) !== 0) return "✕";
    return "✓";
  };
  const renderJobs = () => {
    jobsBar.replaceChildren();
    if (jobs.size === 0) { jobView.hidden = true; return; }
    const label = document.createElement("span");
    label.className = "jobs-label";
    label.textContent = t("JOBS");
    jobsBar.append(label);
    for (const job of jobs.values()) {
      const tab = document.createElement("button");
      tab.type = "button";
      tab.className = "job";
      tab.setAttribute("role", "listitem");
      tab.setAttribute("aria-current", String(selectedJob === job.operationId));
      tab.title = job.text;
      const icon = document.createElement("span");
      icon.textContent = jobIcon(job);
      icon.className = !job.done ? "spin" : (job.cancelled || job.timedOut || (job.exitCode ?? 0) !== 0) ? "bad" : "ok";
      const name = document.createElement("span");
      name.className = "name";
      name.textContent = jobLabel(job);
      const dismiss = document.createElement("span");
      dismiss.className = "dismiss";
      dismiss.textContent = "×";
      dismiss.setAttribute("role", "button");
      dismiss.setAttribute("aria-label", t("Remove job from list"));
      dismiss.addEventListener("click", (event) => {
        event.stopPropagation();
        jobs.delete(job.operationId);
        if (selectedJob === job.operationId) selectedJob = null;
        renderJobs();
        renderJobView();
      });
      tab.append(icon, name, dismiss);
      tab.addEventListener("click", () => {
        selectedJob = selectedJob === job.operationId ? null : job.operationId;
        renderJobs();
        renderJobView();
      });
      jobsBar.append(tab);
    }
  };
  const renderJobView = () => {
    const job = selectedJob ? jobs.get(selectedJob) : undefined;
    if (!job) { jobView.hidden = true; return; }
    jobView.hidden = false;
    jobCommand.textContent = job.text;
    jobState.textContent = job.done
      ? job.cancelled
        ? t("cancelled")
        : job.timedOut
          ? t("timed out")
          : t("exit {code}", { code: job.exitCode ?? "?" })
      : t("running");
    jobStop.disabled = job.done;
    jobBody.replaceChildren();
    for (const [name, value] of [["stdout", job.stdout], ["stderr", job.stderr]] as const) {
      if (!value) continue;
      const block = document.createElement("div");
      block.className = "job-stream";
      const heading = document.createElement("h4");
      heading.textContent = name.toUpperCase();
      const pre = document.createElement("pre");
      pre.textContent = value;
      block.append(heading, pre);
      jobBody.append(block);
    }
  };
  const readJob = async (job: JobView) => {
    let more = true;
    for (let batch = 0; batch < 32 && more; batch++) {
      const view = await invoke<{
        done: boolean;
        exitCode: number | null;
        cancelled: boolean;
        timedOut: boolean;
        stdout: { text?: string; data?: string; cursor: string; more: boolean };
        stderr: { text?: string; data?: string; cursor: string; more: boolean };
      }>("ssh.output", { operationId: job.operationId, stdoutCursor: job.stdoutCursor, stderrCursor: job.stderrCursor });
      const decode = (stream: { text?: string; data?: string }) =>
        stream.text ?? (stream.data ? new TextDecoder().decode(Uint8Array.from(atob(stream.data), (c) => c.charCodeAt(0))) : "");
      job.stdout += decode(view.stdout);
      job.stderr += decode(view.stderr);
      job.stdoutCursor = view.stdout.cursor;
      job.stderrCursor = view.stderr.cursor;
      job.done = view.done;
      job.exitCode = view.exitCode;
      job.cancelled = view.cancelled;
      job.timedOut = view.timedOut;
      more = view.stdout.more || view.stderr.more;
      if (job.stdout.length > 64 * 1024) job.stdout = job.stdout.slice(-64 * 1024);
      if (job.stderr.length > 64 * 1024) job.stderr = job.stderr.slice(-64 * 1024);
    }
    renderJobs();
    renderJobView();
  };
  const trackJob = (operationId: string, text: string) => {
    if (jobs.has(operationId)) return;
    jobs.set(operationId, {
      operationId, text, exitCode: null, done: false, cancelled: false, timedOut: false,
      stdout: "", stderr: "", stdoutCursor: "0", stderrCursor: "0", more: false,
    });
    renderJobs();
  };

  jobClose.addEventListener("click", () => { selectedJob = null; renderJobs(); renderJobView(); });
  jobStop.addEventListener("click", async () => {
    const job = selectedJob ? jobs.get(selectedJob) : undefined;
    if (!job || job.done) return;
    try { await api.cancelOperation(job.operationId); }
    catch (reason) { showError(reason); }
  });

  const startEvents = async () => {
    const cancel = await api.subscribeEvents((event) => enqueue(async () => {
      if (event.name === "runtime.closed") { setConnected(false); notice.textContent = t("Plugin connection closed."); return; }
      if (event.name === "runtime.gap" || event.name === "ssh.status") { await refreshSnapshot(); return; }
      if (event.name === "ssh.job") {
        // The plugin announces an exec operation before it finishes; tracking it here
        // is what lets the jobs row appear while the command is still running.
        const payload = event.payload as { operationId?: string; text?: string; done?: boolean };
        if (!payload.operationId) return;
        trackJob(payload.operationId, payload.text ?? payload.operationId);
        const job = jobs.get(payload.operationId);
        if (job) await readJob(job);
        return;
      }
      if (event.name !== "ssh.terminal.output") return;
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
        const confirmation = await api.requestConfirmation({ title: t("Verify SSH server"), prompt: `${label.textContent}\n\n${probe.fingerprint}\n\n${t("Compare this fingerprint with the server administrator before continuing.")}` });
        if (disposed || confirmation.outcome !== "submitted" || !confirmation.value) return;
        const secret = await api.requestSecret({ title: t("SSH authentication"), prompt: t("Password for {server}", { server: label.textContent ?? "" }), purpose: "ssh.password", allowCancel: true, timeoutMs: 60_000 });
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
    if (queuedBytes + data.length > 16 * 1024) { notice.textContent = t("Input queue is full. Wait before typing or pasting more."); return; }
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
  const stopLocale = api.i18n?.subscribe(() => {
    searchView.refreshLocale();
    jobStop.textContent = t("Stop");
    jobClose.textContent = t("Close view");
    jobsBar.setAttribute("aria-label", t("Background jobs"));
    state.textContent = t(stateMessage);
    connect.textContent = t(connected ? "Disconnect" : "Connect");
    renderJobs();
    renderJobView();
  });
  void startEvents().catch(showError);

  return { unmount() {
    disposed = true;
    controller.abort();
    stopLocale?.();
    unsubscribe?.();
    window.clearTimeout(resizeTimer);
    observer.disconnect();
    onData.dispose();
    stopShortcuts();
    searchView.dispose();
    stopTheme();
    terminal.dispose();
    container.replaceChildren();
  } };
}
