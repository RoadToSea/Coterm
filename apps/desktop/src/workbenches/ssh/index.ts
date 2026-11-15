import { viewNodes } from "../view";
import { mountTerminalSurface } from "../common/terminal-surface";
import type { DriverAPI, DriverFrontend, OperationReceipt } from "../../../../shared/index";
import { createTranslator } from "./i18n";
import { mountSshBlocks } from "./blocks";
import { mountJobs } from "./jobs";
import { observeHumanCommands } from "./human-commands";
import { observeBlockPrompt } from "./observed-boundaries";
import { mountTerminalBlocks } from "../common/terminal-blocks";

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

export function mount(container: HTMLElement, api: DriverAPI): DriverFrontend {
  const t = createTranslator(api.i18n);
  const node = viewNodes(container);
  const bar = node<HTMLDivElement>('bar');
  const label = node<HTMLElement>('label');
  const state = node<HTMLSpanElement>('state');
  const connect = node<HTMLButtonElement>('connect');
  const workbench = node<HTMLDivElement>('workbench');
  const terminalPane = node<HTMLDivElement>('terminalPane');
  const gutter = node<HTMLDivElement>('gutter');
  const terminalContainer = node<HTMLDivElement>('terminalContainer');
  const jobsBar = node<HTMLDivElement>('jobsBar');
  const blocksControls = node<HTMLDivElement>('blocksControls');
  const notice = node<HTMLDivElement>('notice');
  let stateMessage = 'Loading…';
  const terminalSurface = mountTerminalSurface(terminalContainer, bar, t, (cols, rows) => {
    if (connected) void invoke("ssh.resize", {cols, rows}).catch(showError);
  }, () => { if (!disposed) notice.textContent = t("Could not copy terminal selection. Try copying again."); });
  const { terminal, searchView } = terminalSurface;
  const humanCommands = observeHumanCommands(terminal);
  let disposed = false;
  let connected = false;
  let working = false;
  let cursor = "0";
  let connectionId: string | null = null;
  let unsubscribe: (() => void) | undefined;
  let updates = Promise.resolve();
  let queuedBytes = 0;
  let inputQueue = Promise.resolve();
  let selectionQueue = Promise.resolve();
  let selectionTimer = 0;
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
    terminalSurface.resize();
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
        const label = block.actor === "human" ? "[Human]" : "[Agent]";
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
      if (event.name === "runtime.closed") { setConnected(false); jobsView.closed(); notice.textContent = t("Driver connection closed."); return; }
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
        // The driver announces an exec operation before it finishes; tracking it here
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
  const onSelection = terminal.onSelectionChange(() => {
    window.clearTimeout(selectionTimer);
    selectionTimer = window.setTimeout(() => {
      if (disposed || !connected || !connectionId) return;
      const targetConnection = connectionId;
      const bytes = new TextEncoder().encode(terminal.getSelection());
      let end = Math.min(bytes.length, 65536);
      while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
      const text = new TextDecoder().decode(bytes.subarray(0, end));
      selectionQueue = selectionQueue.then(async () => {
        if (disposed || !connected || connectionId !== targetConnection) return;
        await invoke("ssh.select_lines", { connectionId: targetConnection, text, truncated: end < bytes.length });
      }).catch(showError);
    }, 100);
  });
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
    if (connected && connectionId) {
      const targetConnection = connectionId;
      void api.invoke("ssh.select_lines", { connectionId: targetConnection, text: "" }).catch(() => {});
    }
    disposed = true;
    controller.abort();
    blocksView.unmount();
    stopLocale?.();
    unsubscribe?.();
    window.clearTimeout(selectionTimer);
    jobsView.unmount();
    terminalBlocks.unmount();
    observedBoundaries.dispose();
    humanCommands.dispose();
    onData.dispose();
    onSelection.dispose();
    terminalSurface.dispose();

  } };
}
