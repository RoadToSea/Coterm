import type { DriverAPI, DriverFrontend, OperationReceipt } from "../../index";
import { mountTerminalSurface } from "../terminal-surface";
import { createTranslator, type Translator } from "./i18n";
import { mountToasts, type ToastTone } from "../toasts";
import { createFormatSwitch, type Format } from "./format-switch";
import { createToolbar } from "./toolbar";
import { createReceiveNewlineNormalizer, type ReceiveNewlineMode } from "./receive-newlines";
import { readDisplayPreferences, saveDisplayPreferences as persistDisplayPreferences } from "./display-preferences";
import { surface, caption, option } from "./styles";

export interface StreamConfig { mode?: "stream" | "shell"; prompt?: string | null }
export interface StreamStatus<C> {
  config: C; connected: boolean; connectionId: string | null; reconnecting?: boolean;
  rxBytes: number; txBytes: number; commandQueue: number; remoteEcho?: boolean;
  [key: string]: unknown;
}
interface Settings<C> {
  element: HTMLFormElement;
  update(config: C, connected: boolean, applied?: boolean): void;
  prepare(): Partial<C> | null;
  locale(t: Translator): void;
  setBusy(busy: boolean): void;
  sync?(status: StreamStatus<C>): void;
}
interface Options<C> {
  namespace: "serial" | "tcp" | "telnet";
  settingsLabel: string; byteLabel: string; terminalOnly?: boolean;
  createSettings(t: Translator, warn: (message: string) => void,
    invoke: <T>(operation: string, args: unknown) => Promise<T>, canInteract: () => boolean): Settings<C>;
  bindConnection?: boolean; pollStatusMs?: number;
  input?(text: string): string;
  echo?(status: StreamStatus<C>): boolean;
  display?(config: C): {receiveNewlines: ReceiveNewlineMode; backspace: "bs" | "del"};
}
interface Output { text: string; bytes: number; cursor: number; gap: boolean; closed: boolean; more: boolean; connectionId: string }
interface HexOutput { hex: string; bytes: number; cursor: number; gap: boolean; closed: boolean; more: boolean; connectionId: string }
interface PushedOutput { text: string; bytes: number; cursor: number; connectionId: string }
/** 回放缓冲是 256 KiB；调试台保留同样大小的可读日志，超出后丢弃最老的块。 */
const LOG_LIMIT = 256 * 1024;
/** 单次渲染最多保留的字符数，避免 HEX 视图把整段缓冲塞进 DOM。 */
const RENDER_LIMIT = 400_000;
const REPEAT_MIN_MS = 20;
const REPEAT_MAX_MS = 60_000;
const WRITE_LIMIT = 16 * 1024;
const HISTORY_LIMIT = 50;
const HISTORY_BYTES = 64 * 1024;

function output<T>(receipt: OperationReceipt<T>): T {
  if (receipt.status !== "completed" || receipt.error || receipt.output === null) {
    throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
  }
  return receipt.output;
}

/** Only explicit HEX paths decode byte strings. Text never passes through this helper. */
function fromHex(input: string): Uint8Array {
  const cleaned = input.replace(/\s+/g, "").replace(/^0x/i, "");
  if (!cleaned || cleaned.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(cleaned)) {
    throw new Error("HEX input must contain an even number of hexadecimal digits.");
  }
  const bytes = new Uint8Array(cleaned.length / 2);
  for (let index = 0; index < bytes.length; index++) {
    bytes[index] = Number.parseInt(cleaned.slice(index * 2, index * 2 + 2), 16);
  }
  return bytes;
}

function hexDump(bytes: Uint8Array, offset: number): string {
  let text = "";
  for (let start = 0; start < bytes.length; start += 16) {
    const line = bytes.subarray(start, start + 16);
    const hex = Array.from(line, (byte) => byte.toString(16).padStart(2, "0")).join(" ");
    const ascii = Array.from(line, (byte) => (byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : ".")).join("");
    text += `${(offset + start).toString(16).padStart(8, "0")}  ${hex.padEnd(47)}  ${ascii}\n`;
  }
  return text;
}

function stamp(time: number): string {
  const date = new Date(time);
  const pad = (value: number, width = 2) => String(value).padStart(width, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}`;
}

interface LogEntry { time: number; bytes: number; text: string; hex: string; boundary?: boolean }

export function mountStreamWorkbench<C extends StreamConfig>(container: HTMLElement, api: DriverAPI, options: Options<C>): DriverFrontend {
  const prefix = options.namespace;
  const displayPreferences = readDisplayPreferences(api.instanceConfigRef, api.driverInstanceId, prefix);
  const root = surface(container);
  const layout = document.createElement("div");
  layout.className = "layout";
  const bar = document.createElement("div");
  bar.className = "bar";
  const modes = document.createElement("div");
  modes.className = "modes";
  modes.setAttribute("role", "group");
  const terminalMode = document.createElement("button");
  terminalMode.type = "button";
  const uartMode = document.createElement("button");
  uartMode.type = "button";
  modes.append(terminalMode, uartMode);

  const connect = document.createElement("button");
  connect.className = "connect-button";
  connect.type = "button";
  connect.disabled = true;
  const spacer = document.createElement("span");
  spacer.className = "spacer";
  const settingsButton = document.createElement("button");
  settingsButton.type = "button";
  settingsButton.className = "settings-toggle";
  settingsButton.setAttribute("aria-expanded", "false");
  settingsButton.setAttribute("aria-controls", `${prefix}-settings`);
  const toolbarButton = document.createElement("button");
  toolbarButton.type = "button"; toolbarButton.className = "toolbar-toggle icon-button";
  toolbarButton.setAttribute("aria-expanded", "false"); toolbarButton.setAttribute("aria-controls", `${prefix}-toolbar-settings`);
  toolbarButton.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h7m4 0h5M4 17h2m4 0h10M11 4v6M6 14v6"/></svg>';
  toolbarButton.hidden = !!options.terminalOnly;
  bar.append(modes, spacer, connect, settingsButton, toolbarButton);
  const receiveArea = document.createElement("div");
  receiveArea.className = "receive-area";
  const settingsPanel = document.createElement("section");
  settingsPanel.id = `${prefix}-settings`;
  settingsPanel.className = "settings-panel";
  settingsPanel.tabIndex = -1;
  settingsPanel.inert = true;
  settingsPanel.setAttribute("aria-hidden", "true");
  const communication = options.createSettings(createTranslator(api.i18n),
    message => notify(message, "warning"),
    async <T>(operation: string, args: unknown): Promise<T> => invoke<T>(operation, args),
    () => !disposed && connected && !working);
  settingsPanel.append(communication.element);

  const receiveNewlines = document.createElement("div");
  receiveNewlines.className = "receive-newlines checks";
  receiveNewlines.setAttribute("role", "group");
  const completeLF = document.createElement("input");
  completeLF.type = "checkbox"; completeLF.name = "receiveCompleteLF";
  const completeCR = document.createElement("input");
  completeCR.type = "checkbox"; completeCR.name = "receiveCompleteCR";
  const completeLFLabel = document.createElement("span");
  const completeCRLabel = document.createElement("span");
  for (const [control, text] of [[completeLF, completeLFLabel], [completeCR, completeCRLabel]]) {
    const label = document.createElement("label"); label.className = "signal-check";
    label.append(control, text); receiveNewlines.append(label);
  }
  let receiveNewlineMode: ReceiveNewlineMode = displayPreferences.receiveNewlines;
  completeLF.checked = receiveNewlineMode === "lf" || receiveNewlineMode === "both";
  completeCR.checked = receiveNewlineMode === "cr" || receiveNewlineMode === "both";
  if (!options.display) communication.element.append(receiveNewlines);
  let receiveNormalizer = createReceiveNewlineNormalizer(receiveNewlineMode);
  const backspaceLabel = caption(createTranslator(api.i18n)("Backspace key"));
  const backspaceLabelText = backspaceLabel.firstElementChild as HTMLSpanElement;
  const backspaceKey = document.createElement("select");
  backspaceKey.name = "backspaceKey";
  backspaceKey.append(option("bs", "BS (0x08)"), option("del", "DEL (0x7F)"));
  backspaceKey.value = displayPreferences.backspace;
  backspaceLabel.append(backspaceKey);
  if (!options.display) communication.element.append(backspaceLabel);

  // 终端模式：原始字节与远端回显，按键即发送。
  const terminalPanel = document.createElement("div");
  terminalPanel.className = "panel";
  const terminalContainer = document.createElement("div");
  terminalContainer.className = "terminal";
  terminalPanel.append(terminalContainer);

  // UART 调试模式：接收视图可切 ASCII/HEX，发送可定时重复。
  const uartPanel = document.createElement("div");
  uartPanel.className = "panel uart";
  uartPanel.hidden = true;
  const uartBar = document.createElement("div");
  uartBar.className = "uart-bar";
  const receiveFormats = createFormatSwitch(createTranslator(api.i18n)("Receive format"));
  receiveFormats.value = displayPreferences.receiveFormat;
  const timestampsLabel = document.createElement("label");
  const timestamps = document.createElement("input");
  timestamps.type = "checkbox";
  timestamps.checked = displayPreferences.timestamps;
  const timestampsText = document.createElement("span");
  timestampsLabel.append(timestamps, timestampsText);
  const autoscrollLabel = document.createElement("label");
  const autoscroll = document.createElement("input");
  autoscroll.type = "checkbox";
  autoscroll.checked = displayPreferences.autoScroll;
  const autoscrollText = document.createElement("span");
  autoscrollLabel.append(autoscroll, autoscrollText);
  const pause = document.createElement("button");
  pause.type = "button";
  const clear = document.createElement("button");
  clear.type = "button";
  const stats = document.createElement("span");
  stats.className = "stats";
  const receiveFilenameLabel = caption(createTranslator(api.i18n)("Save file (download directory)"));
  const receiveFilename = document.createElement("input");
  receiveFilename.className = "receive-filename";
  receiveFilename.value = `${prefix}-receive.txt`;
  receiveFilename.maxLength = 128;
  receiveFilenameLabel.append(receiveFilename);
  const exportButton = document.createElement("button");
  exportButton.type = "button";
  exportButton.className = "export-receive";
  exportButton.classList.add("icon-button");
  exportButton.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/></svg>';
  const toolbar = createToolbar(uartBar, [
    { id: "format", label: "Receive format", element: receiveFormats.element },
    { id: "timestamps", label: "Show timestamps", element: timestampsLabel },
    { id: "autoscroll", label: "Auto-scroll", element: autoscrollLabel },
    { id: "pause", label: "Pause / resume", element: pause },
    { id: "clear", label: "Clear", element: clear },
    { id: "save", label: "Save receive buffer", element: exportButton },
    { id: "stats", label: "Byte counters", element: stats },
  ], createTranslator(api.i18n), `coterm.${prefix}.toolbar.v1`);
  toolbar.panel.id = `${prefix}-toolbar-settings`;

  const uartOutput = document.createElement("pre");
  uartOutput.className = "uart-output";
  uartOutput.tabIndex = 0;

  const uartSend = document.createElement("div");
  uartSend.className = "uart-send";
  const uartEntry = document.createElement("label");
  uartEntry.className = "entry";
  const uartEntryText = document.createElement("span");
  uartEntryText.hidden = true;
  const uartText = document.createElement("textarea");
  uartText.rows = 3;
  uartText.spellcheck = false;
  uartText.maxLength = WRITE_LIMIT * 3 + 2;
  uartEntry.append(uartEntryText, uartText);
  const fileActions = document.createElement("div");
  fileActions.className = "file-actions";
  const importButton = document.createElement("button");
  importButton.type = "button";
  importButton.className = "import-send";
  importButton.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7V4h6l3 3h9v13H3V7zm0 3h18"/></svg>';
  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.hidden = true;
  const historyButton = document.createElement("button");
  historyButton.type = "button";
  historyButton.className = "history-toggle";
  historyButton.setAttribute("aria-expanded", "false");
  historyButton.setAttribute("aria-controls", `${prefix}-send-history`);
  historyButton.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 12a9 9 0 1 0 2.6-6.4L3 8M3 3v5h5M12 7v5l4 2"/></svg>';
  const historyPanel = document.createElement("div");
  historyPanel.id = `${prefix}-send-history`;
  historyPanel.className = "send-history";
  historyPanel.hidden = true;
  historyPanel.setAttribute("role", "region");
  // Keep history buttons outside the textarea label to avoid implicit focus/click activation.
  const entryArea = document.createElement("div");
  entryArea.className = "entry";
  entryArea.append(uartEntry, historyPanel);
  fileActions.append(importButton, historyButton, fileInput);
  const sendFormat = createFormatSwitch(createTranslator(api.i18n)("Send format"), next => {
    if (disposed || importing) return false;
    if (!uartText.value) return true;
    try {
      const bytes = next === "hex" ? new TextEncoder().encode(uartText.value) : fromHex(uartText.value);
      if (bytes.length > WRITE_LIMIT) throw new Error(t("A send must contain 1–16384 bytes including the line ending."));
      const converted = next === "hex"
        ? Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join(" ")
        : new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
      uartText.value = converted;
      return true;
    } catch (reason) {
      notify(reason instanceof TypeError ? t("These bytes are not UTF-8 text. Keep HEX to preserve them.")
        : reason instanceof Error && !reason.message.startsWith("HEX input") ? reason.message
        : t("HEX input must contain an even number of hexadecimal digits."), "warning");
      return false;
    }
  });
  sendFormat.element.classList.add("send-mode");
  const uartEndingLabel = caption(createTranslator(api.i18n)("Line ending"));
  const uartEndingLabelText = uartEndingLabel.firstElementChild as HTMLSpanElement;
  const uartEnding = document.createElement("select");
  uartEnding.className = "line-ending";
  uartEnding.append(option("none", "None"), option("cr", "CR"), option("lf", "LF"), option("crlf", "CRLF"));
  uartEnding.value = displayPreferences.sendLineEnding;
  uartEndingLabel.append(uartEnding);
  if (!options.terminalOnly) communication.element.append(uartEndingLabel, receiveFilenameLabel);
  const repeatRow = document.createElement("div");
  repeatRow.className = "inline";
  const repeatMs = document.createElement("input");
  repeatMs.type = "number";
  repeatMs.min = String(REPEAT_MIN_MS);
  repeatMs.max = String(REPEAT_MAX_MS);
  repeatMs.step = "10";
  repeatMs.value = "100";
  repeatMs.setAttribute("aria-label", createTranslator(api.i18n)("Repeat interval (ms)"));
  const repeatUnit = document.createElement("span");
  repeatUnit.textContent = "ms";
  const repeatButton = document.createElement("button");
  repeatButton.type = "button";
  repeatButton.className = "repeat-button";
  repeatButton.disabled = true;
  repeatRow.append(repeatMs, repeatUnit, repeatButton);
  const sendControls = document.createElement("div");
  sendControls.className = "send-controls";
  sendControls.append(sendFormat.element, repeatRow);
  uartSend.append(entryArea, fileActions, sendControls);

  uartPanel.append(uartBar, uartOutput);
  receiveArea.append(terminalPanel, uartPanel, settingsPanel, toolbar.panel, toolbar.trash);

  layout.append(bar, receiveArea, uartSend);
  root.append(layout);
  const toasts = mountToasts(layout);
  let t = createTranslator(api.i18n);

  const terminalSurface = mountTerminalSurface(terminalContainer, bar,
    (message, values) => t(message, values), (cols, rows) => {
      if (options.bindConnection && prefix === "telnet" && connected && connectionId) {
        void invoke(`${prefix}.resize`, {connectionId, cols, rows}).catch(showError);
      }
    }, () => notify(t("Could not copy terminal selection. Try copying again."), "error"));
  const { terminal, searchView } = terminalSurface;

  let disposed = false;
  let connected = false;
  let currentConfig: C | null = null;
  let pendingConfiguration: C | null = null;
  let working = false;
  let cursor = 0;
  let connectionId: string | null = null;
  let unsubscribe: (() => void) | undefined;
  let updates = Promise.resolve();
  let reconnecting = false;
  let localEcho = false;
  let statusTimer = 0;
  let queuedBytes = 0;
  let inputQueue = Promise.resolve();
  let selectionQueue = Promise.resolve();
  let selectionTimer = 0;
  let commandQueue = 0;
  let mode: "terminal" | "uart" = "terminal";
  let uartView: "text" | "hex" = displayPreferences.receiveFormat;
  const saveDisplayPreferences = () => {
    persistDisplayPreferences(api.instanceConfigRef, {
      view: mode, receiveFormat: uartView, timestamps: timestamps.checked, autoScroll: autoscroll.checked,
      receiveNewlines: receiveNewlineMode, backspace: backspaceKey.value === "del" ? "del" : "bs",
      sendLineEnding: uartEnding.value as "none" | "cr" | "lf" | "crlf",
    }, api.driverInstanceId, prefix);
  };
  let paused = false;
  let pendingWhilePaused = 0;
  let renderTimer = 0;
  let repeatTimer = 0;
  let repeating = false;
  let repeatGeneration = 0;
  let settingsOpen = false;
  let importing = false;
  let historyBytes = 0;
  let inputGeneration = 0;
  interface SendHistory { text: string; format: Format; ending: string; bytes: number }
  const history: SendHistory[] = [];
  let statsRx = 0;
  let statsTx = 0;
  const entries: LogEntry[] = [];
  let logBytes = 0;
  let receiveBoundary = true;
  let hexOffset = 0;
  let hexCursor = 0;
  const hexEntries: LogEntry[] = [];
  let hexLogBytes = 0;
  const controller = new AbortController();
  const invoke = async <T = unknown>(operation: string, args: unknown): Promise<T> => output(await api.invoke<T>(operation, args));
  const notify = (message: string, tone: ToastTone = "info") => { if (!disposed) toasts.show({ message, tone }); };
  const showError = (reason: unknown) => notify(reason instanceof Error ? reason.message : String(reason), "error");
  const onSelection = terminal.onSelectionChange(() => {
    window.clearTimeout(selectionTimer);
    selectionTimer = window.setTimeout(() => {
      if (disposed || working || mode !== "terminal" || !connected || !connectionId || currentConfig?.mode !== "shell") return;
      const targetConnection = connectionId;
      const bytes = new TextEncoder().encode(terminal.getSelection());
      let end = Math.min(bytes.length, 16384);
      while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
      const text = new TextDecoder().decode(bytes.subarray(0, end));
      selectionQueue = selectionQueue.then(async () => {
        if (disposed || working || mode !== "terminal" || !connected || connectionId !== targetConnection || currentConfig?.mode !== "shell") return;
        await invoke(`${prefix}.set_selection`, {connectionId:targetConnection, text, truncated:end < bytes.length});
      }).catch(showError);
    }, 100);
  });
  const renderConnection = () => {
    connect.dataset.connected = String(connected);
    connect.textContent = working ? t("Working…") : connected ? t("Connected") : reconnecting ? t("Stop reconnecting") : t("Connect");
    connect.title = connected || reconnecting ? t("Disconnect") : t("Connect");
    connect.setAttribute("aria-label", connect.title);
    connect.setAttribute("aria-pressed", String(connected));
    connect.disabled = working;
    terminalMode.disabled = working || !currentConfig;
    uartMode.disabled = working || !currentConfig;
    modes.hidden = !!options.terminalOnly;
  };
  const renderPause = () => {
    pause.className = "icon-button";
    pause.title = paused ? t("Resume") : t("Pause"); pause.setAttribute("aria-label", pause.title);
    pause.setAttribute("aria-pressed", String(paused));
    pause.innerHTML = paused
      ? '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m8 5 11 7-11 7Z"/></svg>'
      : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14M16 5v14"/></svg>';
  };
  const setToolbarOpen = (open: boolean, restoreFocus = true) => {
    toolbar.panel.hidden = !open;
    toolbar.setEditing(open);
    toolbarButton.setAttribute("aria-expanded", String(open));
    if (open) {
      setSettingsOpen(false, false); closeHistory();
      (uartBar.querySelector<HTMLElement>(".toolbar-item:not([hidden])") ?? toolbar.done).focus();
    }
    else if (restoreFocus) toolbarButton.focus();
  };

  const setSettingsOpen = (open: boolean, restoreFocus = true) => {
    settingsOpen = open;
    settingsPanel.dataset.open = String(open);
    settingsPanel.inert = !open;
    settingsPanel.setAttribute("aria-hidden", String(!open));
    settingsButton.setAttribute("aria-expanded", String(open));
    terminalPanel.inert = open;
    uartPanel.inert = open;
    if (open) { setToolbarOpen(false, false); closeHistory(); settingsPanel.focus(); }
    else if (restoreFocus) settingsButton.focus();
  };

  const closeHistory = () => {
    historyPanel.hidden = true;
    historyButton.setAttribute("aria-expanded", "false");
  };

  const renderHistory = () => {
    historyPanel.replaceChildren();
    if (!history.length) {
      const empty = document.createElement("p");
      empty.className = "hint"; empty.textContent = t("No send history yet.");
      historyPanel.append(empty);
    }
    for (const item of history) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = `${item.format === "hex" ? "HEX" : "ASCII"} · ${item.text.slice(0, 120).replace(/\s+/g, " ")}`;
      button.addEventListener("click", () => {
        stopRepeat();
        sendFormat.value = item.format;
        uartEnding.value = item.ending;
        uartEnding.disabled = item.format === "hex";
        uartText.value = item.text;
        closeHistory(); uartText.focus();
      });
      historyPanel.append(button);
    }
  };

  const remember = (text: string, format: Format, ending: string) => {
    const previous = history.findIndex(item => item.text === text && item.format === format && item.ending === ending);
    if (previous >= 0) historyBytes -= history.splice(previous, 1)[0].bytes;
    const bytes = new TextEncoder().encode(text).length;
    history.unshift({ text, format, ending, bytes }); historyBytes += bytes;
    while (history.length > HISTORY_LIMIT || historyBytes > HISTORY_BYTES) {
      historyBytes -= history.pop()!.bytes;
    }
    renderHistory();
  };

  /** 静态标签集中在这里刷新，语言切换不重建实例或连接。 */
  const applyLocale = () => {
    const tr = createTranslator(api.i18n);
    t = tr;
    searchView.refreshLocale();
    terminalMode.textContent = tr("Terminal");
    uartMode.textContent = tr(options.byteLabel);
    renderConnection();
    settingsButton.textContent = tr("Settings");
    settingsPanel.setAttribute("aria-label", tr(options.settingsLabel));
    toolbarButton.title = tr("Customize toolbar"); toolbarButton.setAttribute("aria-label", toolbarButton.title);
    toolbar.locale(tr); communication.locale(tr);
    backspaceLabelText.textContent = tr("Backspace key");
    receiveNewlines.setAttribute("aria-label", tr("Receive newline completion"));
    receiveNewlines.title = tr("Display only; terminal changes affect new output. Raw data and saved files stay unchanged.");
    completeLFLabel.textContent = tr("Complete LF with CR");
    completeCRLabel.textContent = tr("Complete CR with LF");
    toasts.updateLabels({ ariaLabel: tr("Notifications"), closeLabel: tr("Dismiss notification") });
    receiveFilenameLabel.firstElementChild!.textContent = tr("Save file (download directory)");
    exportButton.title = tr("Save receive buffer"); exportButton.setAttribute("aria-label", exportButton.title);
    receiveFormats.label(tr("Receive format")); sendFormat.label(tr("Send format"));
    timestampsText.textContent = tr("Show timestamps");
    autoscrollText.textContent = tr("Auto-scroll");
    renderPause();
    clear.className = "icon-button"; clear.title = tr("Clear"); clear.setAttribute("aria-label", clear.title);
    clear.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7"/></svg>';
    uartEntryText.textContent = tr("Send");
    uartText.placeholder = tr("Enter to send · Shift+Enter for a new line");
    uartEndingLabelText.textContent = tr("Line ending");
    uartEnding.options[0].textContent = tr("None");
    repeatButton.textContent = tr("Timed send");
    repeatButton.title = repeating ? tr("Stop repeating") : tr("Start repeating");
    repeatMs.setAttribute("aria-label", tr("Repeat interval (ms)"));
    importButton.title = tr("Import send content from file");
    importButton.setAttribute("aria-label", importButton.title);
    historyButton.title = tr("Send history");
    historyButton.setAttribute("aria-label", historyButton.title);
    historyPanel.setAttribute("aria-label", historyButton.title);
    renderHistory();
    uartOutput.dataset.empty = tr("Receive");
    renderStats();
    renderUart(true);
  };

  const renderStats = () => {
    stats.textContent = t("RX {rx} B · TX {tx} B", { rx: statsRx, tx: statsTx });
  };

  const setConnected = (next: boolean) => {
    connected = next;
    if (currentConfig) communication.update(currentConfig, next);
    renderConnection();
    repeatButton.disabled = !next || working;
    if (!next) { inputGeneration++; stopRepeat(t("Repeat stopped: the port is disconnected.")); }
    communication.setBusy(working || reconnecting);
  };

  const resize = () => {
    if (disposed || terminalPanel.hidden) return;
    terminalSurface.resize();
  };

  const scheduleRender = () => {
    if (renderTimer || paused || mode !== "uart") return;
    renderTimer = window.setTimeout(() => {
      renderTimer = 0;
      renderUart(false);
    }, 80);
  };

  /** 由缓存的块拼接接收视图；只有可见时才写入 DOM。 */
  const renderUart = (force: boolean) => {
    if (disposed || (!force && (paused || mode !== "uart"))) return;
    let text = "";
    const normalizer = createReceiveNewlineNormalizer(receiveNewlineMode);
    for (const item of uartView === "hex" ? hexEntries : entries) {
      if (item.boundary) normalizer.reset();
      const prefix = timestamps.checked ? `[${stamp(item.time)}] ` : "";
      const content = uartView === "hex" ? item.hex : normalizer.write(item.text);
      if (content) text += prefix + content;
    }
    if (uartView === "text") text = text.replace(/\r\n/g, "\n");
    uartOutput.textContent = text.length > RENDER_LIMIT ? text.slice(-RENDER_LIMIT) : text;
    if (autoscroll.checked) uartOutput.scrollTop = uartOutput.scrollHeight;
  };

  const clearLog = () => {
    searchView.close(false);
    entries.length = 0;
    hexEntries.length = 0;
    hexLogBytes = 0;
    hexCursor = cursor;
    logBytes = 0;
    hexOffset = 0;
    pendingWhilePaused = 0;
    uartOutput.textContent = "";
    receiveBoundary = true;
    receiveNormalizer.reset();
    terminal.reset();
  };

  const appendOutput = (text: string, bytes: number) => {
    if (!bytes && !text) return;
    const now = Date.now();
    entries.push({
      time: now,
      bytes,
      text,
      hex: "",
      boundary: receiveBoundary,
    });
    receiveBoundary = false;
    logBytes += bytes;
    statsRx += bytes;
    while (logBytes > LOG_LIMIT && entries.length > 1) {
      const dropped = entries.shift();
      if (!dropped) break;
      logBytes -= dropped.bytes;
    }
    if (paused) pendingWhilePaused += bytes;
    renderStats();
    scheduleRender();
  };

  const write = async (data: string) => {
    if (!data || disposed) return;
    const display = receiveNormalizer.write(data);
    if (display) await new Promise<void>((resolve) => terminal.write(display, resolve));
  };

  const refreshHex = async (explicitExport = false) => {
    // Merely mounting a terminal or using text mode never invokes the byte interface.
    for (let batch = 0; batch < 257 && !disposed && (mode === "uart" || explicitExport) && uartView === "hex" && connectionId; batch++) {
      const data = await invoke<HexOutput>(`${prefix}.read_hex`, { cursor: hexCursor });
      if (disposed || data.connectionId !== connectionId) return;
      if (data.gap) { hexEntries.length = 0; hexLogBytes = 0; hexOffset = 0; }
      if (data.hex) {
        const bytes = fromHex(data.hex);
        hexEntries.push({ time: Date.now(), bytes: bytes.length, text: "", hex: hexDump(bytes, hexOffset) });
        hexOffset += bytes.length;
        hexLogBytes += bytes.length;
        while (hexLogBytes > LOG_LIMIT && hexEntries.length > 1) {
          const dropped = hexEntries.shift();
          if (dropped) hexLogBytes -= dropped.bytes;
        }
      }
      hexCursor = data.cursor;
      if (!data.more) break;
    }
    renderUart(true);
  };

  const refreshSnapshot = async () => {
    const status = await invoke<StreamStatus<C>>(`${prefix}.status`, {});
    if (disposed) return;
    currentConfig = status.config;
    const view = status.config.mode === undefined ? displayPreferences.view : status.config.mode === "shell" ? "terminal" : "uart";
    if (mode !== view) renderMode(view, false);
    if (connectionId !== status.connectionId) {
      connectionId = status.connectionId;
      cursor = 0;
      statsRx = 0;
      statsTx = 0;
      if (connectionId) clearLog();
    }
    communication.sync?.(status);
    reconnecting = status.reconnecting ?? false;
    localEcho = options.echo?.(status) ?? false;
    if (options.display) {
      const display = options.display(status.config);
      if (receiveNewlineMode !== display.receiveNewlines) {
        receiveNewlineMode = display.receiveNewlines;
        receiveNormalizer = createReceiveNewlineNormalizer(receiveNewlineMode);
        renderUart(true);
      }
      backspaceKey.value = display.backspace;
    }
    commandQueue = status.commandQueue ?? 0;
    if (commandQueue > 0) stopRepeat(t("MCP command queue is busy. Wait for it to finish before sending."));
    statsTx = status.txBytes;
    renderStats();
    setConnected(status.connected);
    connect.disabled = working;
    if (!connectionId) return;
    // Replay only on mount, connection change or a reported gap; never idle polling.
    for (let batch = 0; batch < 257 && !disposed; batch++) {
      const data = await invoke<Output>(`${prefix}.read`, { cursor });
      if (disposed || data.connectionId !== connectionId) return;
      if (data.gap) {
        receiveBoundary = true;
        receiveNormalizer.reset();
        terminal.reset();
        terminal.write(`\r\n[${t("Output gap: older data expired from the replay buffer")}]\r\n`);
      }
      await write(data.text);
      appendOutput(data.text, data.bytes);
      cursor = data.cursor;
      if (!data.more) break;
      if (batch === 256) notify(t("Replay limit reached; waiting for the next output event."), "warning");
    }
    await refreshHex();
    if (connected) resize();
  };

  const enqueue = (action: () => Promise<void>) => {
    const next = updates.then(async () => { if (!disposed) await action(); });
    updates = next.catch(showError);
    return updates;
  };

  const sendInput = (operation: string, args: {text: string; lineEnding: string} | {hex: string}, bytes: number): Promise<boolean> => {
    if (disposed || !connected || working) return Promise.resolve(false);
    if (commandQueue > 0) {
      notify(t("MCP command queue is busy. Wait for it to finish before sending."), "warning");
      return Promise.resolve(false);
    }
    if (bytes < 1 || bytes > WRITE_LIMIT) {
      notify(t("A send must contain 1–16384 bytes including the line ending."), "warning");
      return Promise.resolve(false);
    }
    if (queuedBytes + bytes > WRITE_LIMIT) {
      notify(t("Input queue is full. Wait before sending more."), "warning");
      return Promise.resolve(false);
    }
    const generation = inputGeneration;
    const target = connectionId;
    queuedBytes += bytes;
    const pending = inputQueue.then(async () => {
      if (!disposed && connected && generation === inputGeneration && target === connectionId && !commandQueue) {
        // 驱动可能补上行结束符，计数以驱动回报的实际字节为准。
        const written = await invoke<{ bytes: number }>(operation, { ...args, ...(options.bindConnection ? {connectionId:target} : {}) });
        if (disposed || target !== connectionId) return false;
        if (localEcho) {
          const text = "text" in args ? args.text + (({cr:"\r",lf:"\n",crlf:"\r\n"} as Record<string,string>)[args.lineEnding] ?? "")
            : new TextDecoder().decode(fromHex(args.hex));
          const display = createReceiveNewlineNormalizer(receiveNewlineMode).write(text);
          if (display) terminal.write(display);
        }
        statsTx += written.bytes;
        renderStats();
        return true;
      }
      return false;
    }).catch(reason => { showError(reason); return false; }).finally(() => { queuedBytes -= bytes; });
    inputQueue = pending.then(() => {});
    return pending;
  };

  const sendText = (text: string, lineEnding: string) => {
    const endingBytes = lineEnding === "crlf" ? 2 : lineEnding === "none" ? 0 : 1;
    return sendInput(`${prefix}.write`, {text, lineEnding}, new TextEncoder().encode(text).length + endingBytes);
  };

  const stopRepeat = (reason?: string) => {
    if (!repeating) return;
    window.clearTimeout(repeatTimer);
    repeating = false;
    repeatGeneration++;
    inputGeneration++;
    repeatTimer = 0;
    sendFormat.disabled = false;
    repeatMs.disabled = false;
    repeatButton.textContent = t("Timed send");
    repeatButton.title = t("Start repeating");
    repeatButton.setAttribute("aria-pressed", "false");
    if (reason) notify(reason, "warning");
  };

  const doConnect = async () => {
    if (working || disposed) return;
    working = true;
    renderConnection();
    repeatButton.disabled = true;
    communication.setBusy(true);
    stopRepeat();
    inputGeneration++;
    toasts.clear();
    try {
      if (connected || reconnecting) {
        await invoke(`${prefix}.disconnect`, {});
        reconnecting = false; setConnected(false);
      } else {
        let parameters: ReturnType<typeof communication.prepare>;
        try { parameters = communication.prepare(); }
        catch (reason) { notify(reason instanceof Error ? reason.message : String(reason), "warning"); return; }
        if (parameters) {
          const configured = await invoke<{config: C}>(`${prefix}.configure`, parameters);
          if (disposed) return;
          pendingConfiguration = configured.config;
          if (disposed) return;
          await enqueue(refreshSnapshot);
          if (currentConfig) communication.update(currentConfig, false, true);
        }
        if (disposed) return;
        if (pendingConfiguration && api.saveConfiguration) {
          await api.saveConfiguration(pendingConfiguration);
          pendingConfiguration = null;
        }
        if (disposed) return;
        await invoke(`${prefix}.connect`, {});
        if (disposed) return;
        await enqueue(refreshSnapshot);
        if (!settingsOpen) { if (mode === "terminal") terminal.focus(); else uartText.focus(); }
      }
    } catch (reason) { showError(reason); }
    finally { if (!disposed) { working = false; connect.disabled = false; setConnected(connected); } }
  };

  const startEvents = async () => {
    const cancel = await api.subscribeEvents((event) => enqueue(async () => {
      if (event.name === "runtime.closed") { setConnected(false); notify(t("Driver connection closed."), "warning"); return; }
      if (event.name === "runtime.gap" || event.name === `${prefix}.status`) { await refreshSnapshot(); return; }
      if (event.name === `${prefix}.command`) {
        const progress = event.payload as { connectionId: string; txBytes: number; finished: boolean };
        if (progress.connectionId !== connectionId) return;
        statsTx = progress.txBytes;
        renderStats();
        if (progress.finished) await refreshSnapshot();
        return;
      }
      if (event.name !== `${prefix}.output`) return;
      const data = event.payload as PushedOutput;
      if (typeof data.text !== "string" || typeof data.bytes !== "number" || typeof data.connectionId !== "string" || !Number.isSafeInteger(data.cursor) || data.cursor < 0) {
        throw new Error("Invalid stream output event");
      }
      if (data.connectionId !== connectionId) await refreshSnapshot();
      if (data.connectionId !== connectionId || data.cursor <= cursor) return;
      if (data.cursor !== cursor + 1) { await refreshSnapshot(); return; }
      await write(data.text);
      appendOutput(data.text, data.bytes);
      cursor = data.cursor;
      await refreshHex();
    }));
    if (disposed) { cancel(); return; }
    unsubscribe = cancel;
    await enqueue(refreshSnapshot);
    const poll = async () => {
      try { await enqueue(refreshSnapshot); }
      finally { if (!disposed) statusTimer = window.setTimeout(() => void poll(), options.pollStatusMs); }
    };
    if (options.pollStatusMs && !disposed) statusTimer = window.setTimeout(() => void poll(), options.pollStatusMs);
  };

  const renderMode = (next: "terminal" | "uart", remember = true) => {
    if (mode !== next) { stopRepeat(); inputGeneration++; closeHistory(); terminal.clearSelection(); }
    if (options.terminalOnly) next = "terminal";
    mode = next;
    if (remember) saveDisplayPreferences();
    terminalMode.setAttribute("aria-pressed", String(next === "terminal"));
    uartMode.setAttribute("aria-pressed", String(next === "uart"));
    terminalPanel.hidden = next !== "terminal";
    uartPanel.hidden = next !== "uart";
    uartSend.hidden = next !== "uart";
    if (next === "uart") {
      searchView.close(false);
      renderUart(true);
      void enqueue(refreshHex);
      uartText.focus();
    } else {
      requestAnimationFrame(() => {
        if (disposed || settingsOpen) return;
        resize();
        terminal.refresh(0, terminal.rows - 1);
        terminal.focus();
      });
    }
  };

  const switchMode = async (next: "terminal" | "uart") => {
    if (disposed || working || !currentConfig) return;
    const interpretation = next === "terminal" ? "shell" : "stream";
    if (mode === next && currentConfig.mode === interpretation && !pendingConfiguration) return;
    if (commandQueue > 0) {
      notify(t("Wait for the command queue to finish before switching views."), "warning"); return;
    }
    working = true; renderConnection(); communication.setBusy(true);
    stopRepeat(); inputGeneration++;
    try {
      await Promise.all([inputQueue, selectionQueue]);
      if (disposed) return;
      if (currentConfig.mode !== interpretation) {
        const result = await invoke<StreamStatus<C>>(`${prefix}.configure`, {mode:interpretation, prompt:currentConfig.prompt ?? "# "});
        if (disposed) return;
        currentConfig = result.config;
        pendingConfiguration = result.config;
        renderMode(next);
        await refreshSnapshot();
      }
      if (pendingConfiguration && api.saveConfiguration) {
        try {
          await api.saveConfiguration(pendingConfiguration);
          pendingConfiguration = null;
        } catch (reason) {
          notify(t("View changed, but configuration could not be saved: {reason}", {reason:String(reason)}), "error");
        }
      }
    } catch (reason) { showError(reason); }
    finally { if (!disposed) { working = false; setConnected(connected); } }
  };
  terminalMode.addEventListener("click", () => { void enqueue(() => switchMode("terminal")); }, { signal: controller.signal });
  uartMode.addEventListener("click", () => { void enqueue(() => switchMode("uart")); }, { signal: controller.signal });

  connect.addEventListener("click", doConnect, { signal: controller.signal });

  const uartInput = (): (() => Promise<boolean>) | null => {
    if (!uartText.value) return null;
    try {
      if (sendFormat.value === "hex") {
        const bytes = fromHex(uartText.value);
        const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
        return () => sendInput(`${prefix}.write_hex`, {hex}, bytes.length);
      }
      const text = uartText.value;
      const ending = uartEnding.value;
      return () => sendText(text, ending);
    } catch (reason) {
      notify(reason instanceof Error && reason.message.startsWith("HEX input")
        ? t("HEX input must contain an even number of hexadecimal digits.")
        : reason instanceof Error ? reason.message : String(reason), "warning");
      return null;
    }
  };

  const sendUart = async () => {
    if (mode !== "uart" || repeating || importing) return;
    const text = uartText.value, format = sendFormat.value, ending = uartEnding.value;
    if (await uartInput()?.() && !disposed) remember(text, format, ending);
  };
  sendFormat.element.addEventListener("change", () => { stopRepeat(); uartEnding.disabled = sendFormat.value === "hex"; }, { signal: controller.signal });
  uartEnding.addEventListener("change", () => { stopRepeat(); saveDisplayPreferences(); }, { signal: controller.signal });
  uartText.addEventListener("input", () => {
    stopRepeat();
    if (sendFormat.value === "hex" && /[^0-9a-f\s]/i.test(uartText.value.replace(/^0x/i, ""))) {
      notify(t("HEX input must contain an even number of hexadecimal digits."), "warning");
    }
  }, { signal: controller.signal });
  uartText.addEventListener("blur", () => {
    if (sendFormat.value !== "hex" || !uartText.value) return;
    try { uartText.value = Array.from(fromHex(uartText.value), byte => byte.toString(16).padStart(2, "0")).join(" "); }
    catch { notify(t("HEX input must contain an even number of hexadecimal digits."), "warning"); }
  }, { signal: controller.signal });
  uartText.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing && event.keyCode !== 229) {
      event.preventDefault();
      if (!event.repeat) void sendUart();
    }
  }, { signal: controller.signal });

  repeatButton.addEventListener("click", () => {
    if (disposed) return;
    if (repeating) { stopRepeat(); return; }
    if (!connected || working || importing) return;
    const interval = Number(repeatMs.value);
    if (!Number.isInteger(interval) || interval < REPEAT_MIN_MS || interval > REPEAT_MAX_MS) {
      notify(t("Repeat interval must be between 20 and 60000 ms."), "warning");
      return;
    }
    // 重复发送复用同一条输入队列，不绕过节流和连接检查。
    const send = uartInput();
    if (!send) return;
    const text = uartText.value, format = sendFormat.value, ending = uartEnding.value;
    repeating = true;
    const generation = ++repeatGeneration;
    sendFormat.disabled = true;
    repeatMs.disabled = true;
    closeHistory();
    let remembered = false;
    const tick = async () => {
      if (!repeating || generation !== repeatGeneration || disposed) return;
      if (!connected) { stopRepeat(t("Repeat stopped: the port is disconnected.")); return; }
      const sent = await send();
      if (disposed || generation !== repeatGeneration || !repeating) return;
      if (!sent) { stopRepeat(); return; }
      if (!remembered) { remember(text, format, ending); remembered = true; }
      repeatTimer = window.setTimeout(() => void tick(), interval);
    };
    repeatTimer = window.setTimeout(() => void tick(), interval);
    repeatButton.textContent = t("Timed send");
    repeatButton.title = t("Stop repeating");
    repeatButton.setAttribute("aria-pressed", "true");
  }, { signal: controller.signal });

  receiveFormats.element.addEventListener("change", () => {
    uartView = receiveFormats.value;
    saveDisplayPreferences();
    if (uartView === "hex") void enqueue(refreshHex);
    else renderUart(true);
  }, { signal: controller.signal });
  timestamps.addEventListener("change", () => { saveDisplayPreferences(); renderUart(true); }, { signal: controller.signal });
  receiveNewlines.addEventListener("change", () => {
    receiveNewlineMode = completeLF.checked ? (completeCR.checked ? "both" : "lf") : (completeCR.checked ? "cr" : "none");
    receiveNormalizer = createReceiveNewlineNormalizer(receiveNewlineMode);
    saveDisplayPreferences();
    renderUart(true);
  }, { signal: controller.signal });
  backspaceKey.addEventListener("change", saveDisplayPreferences, { signal: controller.signal });
  autoscroll.addEventListener("change", () => { saveDisplayPreferences(); if (autoscroll.checked) renderUart(true); }, { signal: controller.signal });
  clear.addEventListener("click", () => {
    clearLog();
    if (paused) { paused = false; renderPause(); }
    pendingWhilePaused = 0;
    toasts.clear();
  }, { signal: controller.signal });
  pause.addEventListener("click", () => {
    paused = !paused;
    renderPause();
    if (paused) {
      pendingWhilePaused = 0;
    } else {
      if (pendingWhilePaused > 0) notify(t("The device sent {bytes} bytes while paused.", { bytes: pendingWhilePaused }));
      pendingWhilePaused = 0;
      renderUart(true);
    }
  }, { signal: controller.signal });

  settingsButton.addEventListener("click", () => setSettingsOpen(!settingsOpen), { signal: controller.signal });
  toolbarButton.addEventListener("click", () => setToolbarOpen(toolbar.panel.hidden), { signal: controller.signal });
  toolbar.done.addEventListener("click", () => setToolbarOpen(false), { signal: controller.signal });
  historyButton.addEventListener("click", () => {
    const open = historyPanel.hidden;
    if (open) { setSettingsOpen(false, false); renderHistory(); }
    historyPanel.hidden = !open;
    historyButton.setAttribute("aria-expanded", String(open));
    if (open) historyPanel.querySelector("button")?.focus();
  }, { signal: controller.signal });
  root.addEventListener("keydown", event => {
    if ((event as KeyboardEvent).key !== "Escape") return;
    if (!historyPanel.hidden) { closeHistory(); historyButton.focus(); event.stopPropagation(); }
    else if (settingsOpen) { setSettingsOpen(false); event.stopPropagation(); }
    else if (!toolbar.panel.hidden) { setToolbarOpen(false); event.stopPropagation(); }
  }, { signal: controller.signal });
  root.addEventListener("pointerdown", event => {
    const path = event.composedPath();
    if (!path.includes(historyPanel) && !path.includes(historyButton)) closeHistory();
    if (settingsOpen && !path.includes(settingsPanel) && !path.includes(settingsButton) && !path.includes(layout.querySelector(".coterm-toasts")!)) setSettingsOpen(false, false);
    if (!toolbar.panel.hidden && !path.includes(toolbar.panel) && !path.includes(toolbarButton) && !path.includes(uartBar) && !path.includes(toolbar.trash)) setToolbarOpen(false, false);
  }, { signal: controller.signal });

  importButton.addEventListener("click", () => { stopRepeat(); fileInput.click(); }, { signal: controller.signal });
  fileInput.addEventListener("change", async () => {
    const file = fileInput.files?.[0];
    if (!file || importing || disposed) return;
    stopRepeat();
    importing = true; importButton.disabled = true; historyButton.disabled = true;
    uartText.readOnly = true; sendFormat.disabled = true;
    const format = sendFormat.value;
    try {
      if (file.size > WRITE_LIMIT) throw new Error(t("The selected file exceeds the 16 KiB send limit."));
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (disposed) return;
      uartText.value = format === "hex"
        ? Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join(" ")
        : new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      notify(t("Imported {name}. Review the content, then press Enter to send.", { name: file.name }));
      uartText.focus();
    } catch (reason) {
      showError(reason instanceof TypeError ? t("The file is not UTF-8 text. Select HEX to import binary data.") : reason);
    } finally {
      importing = false;
      if (!disposed) { importButton.disabled = false; historyButton.disabled = false; uartText.readOnly = false; sendFormat.disabled = false; fileInput.value = ""; }
    }
  }, { signal: controller.signal });

  exportButton.addEventListener("click", async () => {
    exportButton.disabled = true;
    try {
      if (downloads.size >= 4) { notify(t("Wait before saving another snapshot."), "warning"); return; }
      if (uartView === "hex" && connected) await enqueue(() => refreshHex(true));
      if (disposed) return;
      const text = (uartView === "hex" ? hexEntries : entries)
        .map(item => uartView === "hex" ? item.hex : item.text).join("");
      if (!text) { notify(t("The receive buffer is empty."), "warning"); return; }
      const filename = receiveFilename.value.trim().replace(/[\\/:*?"<>|\u0000-\u001f]/g, "_").replace(/^\.+/, "") || `${prefix}-receive.txt`;
      const url = URL.createObjectURL(new Blob([text], { type: "text/plain;charset=utf-8" }));
      const link = document.createElement("a");
      link.href = url; link.download = filename; root.append(link); link.click(); link.remove();
      // Allow the WebView to consume the URL before releasing the snapshot.
      const timer = window.setTimeout(() => { URL.revokeObjectURL(url); downloads.delete(url); }, 30_000);
      downloads.set(url, timer);
    } catch (reason) { showError(reason); }
    finally { if (!disposed) exportButton.disabled = false; }
  }, { signal: controller.signal });

  // Map only a physical, unmodified Backspace. Paste, API data and other controls stay byte-exact.
  // Capture on the ancestor before xterm's own textarea handler can emit DEL.
  terminal.element?.addEventListener("keydown", event => {
    if (event.target !== terminal.textarea || event.key !== "Backspace" || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey
      || event.isComposing || event.keyCode === 229 || disposed || mode !== "terminal") return;
    event.preventDefault(); event.stopImmediatePropagation();
    void sendText(backspaceKey.value === "del" ? "\x7f" : "\b", "none");
  }, { capture: true, signal: controller.signal });
  // Same text contract as MCP: preserve controls, CR and escape sequences exactly.
  const onData = terminal.onData((text) => sendText(options.input?.(text) ?? text, "none"));
  const stopLocale = api.i18n?.subscribe(() => applyLocale());
  const downloads = new Map<string, number>();
  applyLocale();
  renderMode(displayPreferences.view, false);
  void startEvents().catch(showError);

  return { unmount() {
    disposed = true;
    stopRepeat();
    controller.abort();
    stopLocale?.();
    unsubscribe?.();
    window.clearTimeout(statusTimer);
    window.clearTimeout(renderTimer);
    for (const [url, timer] of downloads) { window.clearTimeout(timer); URL.revokeObjectURL(url); }
    downloads.clear();
    history.length = 0;
    onData.dispose();
    onSelection.dispose();
    window.clearTimeout(selectionTimer);
    toolbar.unmount(); receiveFormats.unmount(); sendFormat.unmount(); toasts.unmount();
    terminalSurface.dispose();
    container.replaceChildren();
  } };
}
