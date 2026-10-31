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
  button, input, select, textarea { font: inherit; color: inherit; }
  form input, form select, form button { border-radius: var(--radius, 8px); }
  button {
    border: 1px solid var(--coterm-border-input, #454545);
    padding: 5px 12px;
    background: var(--coterm-surface-sidebar, #333333);
    cursor: pointer;
  }
  button:hover { background: var(--coterm-surface-hover, #414141); }
  button:disabled { opacity: .5; cursor: default; }
  button[aria-pressed="true"] {
    border-color: var(--coterm-focus, #007fd4);
    background: var(--coterm-surface-active, #37373d);
  }
  button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible {
    outline: 1px solid var(--coterm-focus, #007fd4);
  }
  .primary { background: var(--coterm-accent, #0e639c); color: var(--coterm-accent-foreground, #ffffff); }
  .primary:hover { background: var(--coterm-accent-hover, #1177bb); }
  form { display: grid; gap: 14px; margin-bottom: 20px; }
  label { display: grid; gap: 6px; color: var(--coterm-text-subtle, #aaaaaa); }
  input, select, textarea {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--coterm-border-input, #454545);
    background: var(--coterm-surface, #1e1e1e);
    color: var(--coterm-text-primary, #cccccc);
    padding: 7px 9px;
  }
  textarea { resize: vertical; font-family: var(--coterm-font-mono, Consolas, monospace); min-height: 34px; }
  .pair { display: grid; grid-template-columns: minmax(0, 1fr) 130px; gap: 12px; }
  .triple { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
  .checks { display: flex; flex-wrap: wrap; gap: 16px; }
  .checks label { display: flex; align-items: center; gap: 7px; color: var(--coterm-text-primary, #cccccc); }
  .checks input { width: auto; }
  .error { color: var(--coterm-danger, #f48771); overflow-wrap: anywhere; margin: 0; }
  .hint { color: var(--coterm-text-muted, #969696); font-size: 11px; overflow-wrap: anywhere; margin: 0; }
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
  .modes { display: inline-flex; }
  .modes button { border-radius: 0; }
  .modes button:first-child { border-radius: 2px 0 0 2px; }
  .modes button:last-child { border-radius: 0 2px 2px 0; border-left: 0; }
  .panel { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .panel[hidden] { display: none; }
  .terminal { flex: 1; min-height: 0; overflow: hidden; padding: 10px 4px 0 10px; }
  .uart { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .uart-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--coterm-border, #333333);
    font-size: 11px;
    color: var(--coterm-text-subtle, #aaaaaa);
  }
  .uart-bar .stats { margin-left: auto; color: var(--coterm-text-muted, #969696); font-variant-numeric: tabular-nums; }
  .uart-bar label { display: inline-flex; align-items: center; gap: 5px; color: var(--coterm-text-subtle, #aaaaaa); }
  .uart-bar input { width: auto; }
  .uart-bar button { font-size: 11px; padding: 3px 9px; }
  .uart-output {
    flex: 1;
    min-height: 0;
    margin: 0;
    overflow: auto;
    padding: 8px 14px;
    background: var(--coterm-surface, #1e1e1e);
    color: var(--coterm-text-primary, #cccccc);
    font: 12px/1.45 var(--coterm-font-mono, Consolas, "Liberation Mono", monospace);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    tab-size: 4;
  }
  .uart-output:empty::before {
    content: attr(data-empty);
    color: var(--coterm-text-disabled, #6c6c6c);
  }
  .uart-send {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto auto auto;
    gap: 8px;
    align-items: end;
    border-top: 1px solid var(--coterm-border, #333333);
    padding: 8px 14px;
  }
  .uart-send label { gap: 4px; }
  .uart-send .inline { display: inline-flex; align-items: center; gap: 6px; }
  .uart-send .inline input { width: auto; }
  .uart-send input[type="number"] { width: 82px; }
  .signals {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-top: 1px solid var(--coterm-border, #333333);
    font-size: 11px;
    color: var(--coterm-text-muted, #969696);
  }
  .notice { padding: 7px 14px; font-size: 12px; color: var(--coterm-text-subtle, #aaaaaa); overflow-wrap: anywhere; }
  .notice:empty { display: none; }
`;

function surface(container: HTMLElement, withTerminalStyle = false): ShadowRoot {
  // A new host element keeps React StrictMode remounts independent.
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

function option(value: string, label: string): HTMLOptionElement {
  const element = document.createElement("option");
  element.value = value;
  element.textContent = label;
  return element;
}

function caption(text: string): HTMLLabelElement {
  const label = document.createElement("label");
  const span = document.createElement("span");
  span.textContent = text;
  label.append(span);
  return label;
}

/** 只改标签里的文案节点，语言切换不重建控件。 */
function retitle(label: HTMLLabelElement, text: string) {
  const span = label.querySelector("span");
  if (span) span.textContent = text;
  else label.prepend(document.createTextNode(text));
}

interface PortEntry { portName: string; portType: { kind: string; product?: string | null } }
interface Config {
  port: string; baudRate: number; dataBits: number; stopBits: number;
  parity: "none" | "odd" | "even"; dtr: boolean; rts: boolean;
}
interface Status {
  config: Config; connected: boolean; connectionId: string | null;
  signals: { dtr: boolean; rts: boolean }; rxBytes: number; txBytes: number;
  commandQueue: number;
}
interface Probe { config: Config; connected: boolean; ports: PortEntry[] }
interface Output { text: string; bytes: number; cursor: number; gap: boolean; closed: boolean; more: boolean; connectionId: string }
interface HexOutput { hex: string; bytes: number; cursor: number; gap: boolean; closed: boolean; more: boolean; connectionId: string }
interface PushedOutput { text: string; bytes: number; cursor: number; connectionId: string }
interface Signals { dtr: boolean; rts: boolean }

const BAUD_RATES = [300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];
/** 回放缓冲是 256 KiB；调试台保留同样大小的可读日志，超出后丢弃最老的块。 */
const LOG_LIMIT = 256 * 1024;
/** 单次渲染最多保留的字符数，避免 HEX 视图把整段缓冲塞进 DOM。 */
const RENDER_LIMIT = 400_000;
const REPEAT_MIN_MS = 20;
const REPEAT_MAX_MS = 60_000;

/** The plugin owns configuration fields; the base never learns UART field semantics. */
export function configure(container: HTMLElement, api: PluginConfigurationAPI): PluginFrontend {
  const t = createTranslator(api.i18n);
  const root = surface(container);
  const form = document.createElement("form");

  const portLabel = caption(t("Port"));
  const port = document.createElement("input");
  port.type = "text";
  port.required = true;
  port.placeholder = "COM6";
  port.spellcheck = false;
  portLabel.append(port);

  const baudLabel = caption(t("Baud rate"));
  const baud = document.createElement("input");
  baud.type = "number";
  baud.value = "115200";
  baud.min = "1";
  const baudList = document.createElement("datalist");
  baudList.id = `coterm-serial-baud-${Math.random().toString(36).slice(2, 9)}`;
  for (const rate of BAUD_RATES) baudList.append(option(String(rate), String(rate)));
  baud.setAttribute("list", baudList.id);
  baudLabel.append(baud, baudList);
  const portRow = document.createElement("div");
  portRow.className = "pair";
  portRow.append(portLabel, baudLabel);

  const dataBits = document.createElement("select");
  for (const bits of ["5", "6", "7", "8"]) dataBits.append(option(bits, bits));
  dataBits.value = "8";
  const dataBitsLabel = caption(t("Data bits"));
  dataBitsLabel.append(dataBits);

  const stopBits = document.createElement("select");
  stopBits.append(option("1", "1"), option("2", "2"));
  const stopBitsLabel = caption(t("Stop bits"));
  stopBitsLabel.append(stopBits);

  const parity = document.createElement("select");
  parity.append(option("none", t("None")), option("odd", t("Odd")), option("even", t("Even")));
  const parityLabel = caption(t("Parity"));
  parityLabel.append(parity);
  const framing = document.createElement("div");
  framing.className = "triple";
  framing.append(dataBitsLabel, stopBitsLabel, parityLabel);

  const dtrLabel = document.createElement("label");
  const dtr = document.createElement("input");
  dtr.type = "checkbox";
  dtr.checked = true;
  const dtrLabelText = document.createElement("span");
  dtrLabelText.textContent = t("DTR high on open");
  dtrLabel.append(dtr, dtrLabelText);
  const rtsLabel = document.createElement("label");
  const rts = document.createElement("input");
  rts.type = "checkbox";
  rts.checked = true;
  const rtsLabelText = document.createElement("span");
  rtsLabelText.textContent = t("RTS high on open");
  rtsLabel.append(rts, rtsLabelText);
  const checks = document.createElement("div");
  checks.className = "checks";
  checks.append(dtrLabel, rtsLabel);

  const hint = document.createElement("p");
  hint.className = "hint";
  hint.textContent = t("Port names come from the operating system, e.g. COM6 or /dev/ttyUSB0.");
  const lineHint = document.createElement("p");
  lineHint.className = "hint";
  lineHint.textContent = t("Some boards reset when DTR changes; clear it if the device reboots unexpectedly.");
  const error = document.createElement("p");
  error.className = "error";
  error.setAttribute("role", "alert");
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "primary";
  submit.textContent = t("Open serial port");
  form.append(portRow, framing, checks, hint, lineHint, error, submit);
  root.append(form);

  const stopLocale = api.i18n?.subscribe(() => {
    const next = createTranslator(api.i18n);
    retitle(portLabel, next("Port"));
    retitle(baudLabel, next("Baud rate"));
    retitle(dataBitsLabel, next("Data bits"));
    retitle(stopBitsLabel, next("Stop bits"));
    retitle(parityLabel, next("Parity"));
    parity.options[0].textContent = next("None");
    parity.options[1].textContent = next("Odd");
    parity.options[2].textContent = next("Even");
    dtrLabelText.textContent = next("DTR high on open");
    rtsLabelText.textContent = next("RTS high on open");
    hint.textContent = next("Port names come from the operating system, e.g. COM6 or /dev/ttyUSB0.");
    lineHint.textContent = next("Some boards reset when DTR changes; clear it if the device reboots unexpectedly.");
    submit.textContent = next("Open serial port");
  });

  let disposed = false;
  let busy = false;
  const controller = new AbortController();

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (busy || disposed) return;
    const name = port.value.trim();
    if (!name) { error.textContent = t("A serial port name is required."); return; }
    busy = true;
    submit.disabled = true;
    try {
      await api.submit({
        port: name,
        baudRate: Number(baud.value),
        dataBits: Number(dataBits.value),
        stopBits: Number(stopBits.value),
        parity: parity.value,
        dtr: dtr.checked,
        rts: rts.checked,
      });
    } catch (reason) {
      if (!disposed) error.textContent = String(reason);
    } finally {
      if (!disposed) { busy = false; submit.disabled = false; }
    }
  }, { signal: controller.signal });

  return { unmount() { disposed = true; stopLocale?.(); controller.abort(); container.replaceChildren(); } };
}

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

interface LogEntry { time: number; bytes: number; text: string; hex: string }

export function mount(container: HTMLElement, api: PluginAPI): PluginFrontend {
  const root = surface(container, true);
  const layout = document.createElement("div");
  layout.className = "layout";
  const bar = document.createElement("div");
  bar.className = "bar";
  const label = document.createElement("strong");
  label.textContent = "Serial";
  const state = document.createElement("span");
  state.className = "state";
  state.textContent = createTranslator(api.i18n)("Loading…");

  const modes = document.createElement("div");
  modes.className = "modes";
  modes.setAttribute("role", "group");
  const terminalMode = document.createElement("button");
  terminalMode.type = "button";
  const uartMode = document.createElement("button");
  uartMode.type = "button";
  modes.append(terminalMode, uartMode);

  const ports = document.createElement("button");
  ports.type = "button";
  ports.disabled = true;
  const connect = document.createElement("button");
  connect.type = "button";
  connect.disabled = true;
  const spacer = document.createElement("span");
  spacer.className = "spacer";
  bar.append(label, state, modes, spacer, ports, connect);

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
  const viewText = document.createElement("button");
  viewText.type = "button";
  const viewHex = document.createElement("button");
  viewHex.type = "button";
  const timestampsLabel = document.createElement("label");
  const timestamps = document.createElement("input");
  timestamps.type = "checkbox";
  const timestampsText = document.createElement("span");
  timestampsLabel.append(timestamps, timestampsText);
  const autoscrollLabel = document.createElement("label");
  const autoscroll = document.createElement("input");
  autoscroll.type = "checkbox";
  autoscroll.checked = true;
  const autoscrollText = document.createElement("span");
  autoscrollLabel.append(autoscroll, autoscrollText);
  const pause = document.createElement("button");
  pause.type = "button";
  const clear = document.createElement("button");
  clear.type = "button";
  const stats = document.createElement("span");
  stats.className = "stats";
  uartBar.append(viewText, viewHex, timestampsLabel, autoscrollLabel, pause, clear, stats);

  const uartOutput = document.createElement("pre");
  uartOutput.className = "uart-output";
  uartOutput.tabIndex = 0;

  const uartSend = document.createElement("div");
  uartSend.className = "uart-send";
  const uartEntry = document.createElement("label");
  const uartEntryText = document.createElement("span");
  uartEntryText.textContent = createTranslator(api.i18n)("Send");
  const uartText = document.createElement("textarea");
  uartText.rows = 1;
  uartEntry.append(uartEntryText, uartText);
  const uartHexLabel = document.createElement("label");
  uartHexLabel.className = "inline";
  const uartHex = document.createElement("input");
  uartHex.type = "checkbox";
  const uartHexText = document.createElement("span");
  uartHexLabel.append(uartHex, uartHexText);
  const uartEndingLabel = caption(createTranslator(api.i18n)("Line ending"));
  const uartEndingLabelText = uartEndingLabel.firstElementChild as HTMLSpanElement;
  const uartEnding = document.createElement("select");
  uartEnding.append(option("none", "None"), option("cr", "CR"), option("lf", "LF"), option("crlf", "CRLF"));
  uartEnding.value = "crlf";
  uartEndingLabel.append(uartEnding);
  const uartSendButton = document.createElement("button");
  uartSendButton.type = "button";
  uartSendButton.className = "primary";
  uartSendButton.disabled = true;
  const repeatLabel = caption(createTranslator(api.i18n)("Repeat"));
  const repeatLabelText = repeatLabel.firstElementChild as HTMLSpanElement;
  const repeatRow = document.createElement("div");
  repeatRow.className = "inline";
  const repeatMs = document.createElement("input");
  repeatMs.type = "number";
  repeatMs.min = String(REPEAT_MIN_MS);
  repeatMs.max = String(REPEAT_MAX_MS);
  repeatMs.step = "10";
  repeatMs.value = "1000";
  const repeatButton = document.createElement("button");
  repeatButton.type = "button";
  repeatButton.disabled = true;
  repeatRow.append(repeatMs, repeatButton);
  repeatLabel.append(repeatRow);
  uartSend.append(uartEntry, uartHexLabel, uartEndingLabel, uartSendButton, repeatLabel);

  const signalsRow = document.createElement("div");
  signalsRow.className = "signals";
  const dtrButton = document.createElement("button");
  dtrButton.type = "button";
  dtrButton.disabled = true;
  const rtsButton = document.createElement("button");
  rtsButton.type = "button";
  rtsButton.disabled = true;
  signalsRow.append(dtrButton, rtsButton);

  uartPanel.append(uartBar, uartOutput, uartSend, signalsRow);

  const notice = document.createElement("div");
  notice.className = "notice";
  notice.setAttribute("role", "status");
  layout.append(bar, terminalPanel, uartPanel, notice);
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
  let cursor = 0;
  let connectionId: string | null = null;
  let unsubscribe: (() => void) | undefined;
  let updates = Promise.resolve();
  let resizeTimer = 0;
  let queuedBytes = 0;
  let inputQueue = Promise.resolve();
  let commandQueue = 0;
  let mode: "terminal" | "uart" = "terminal";
  let uartView: "text" | "hex" = "text";
  let paused = false;
  let pendingWhilePaused = 0;
  let renderTimer = 0;
  let repeatTimer = 0;
  let signals: Signals = { dtr: true, rts: true };
  let statsRx = 0;
  let statsTx = 0;
  const entries: LogEntry[] = [];
  let logBytes = 0;
  let hexOffset = 0;
  let hexCursor = 0;
  const hexEntries: LogEntry[] = [];
  let hexLogBytes = 0;
  const controller = new AbortController();
  // 语言切换只替换翻译函数；已渲染的日志文本保持原文。
  let t = createTranslator(api.i18n);
  const searchView = mountTerminalSearch(bar, terminal, searchAddon, (message, values) => t(message, values));
  const stopShortcuts = attachTerminalShortcuts(terminal, () => {
    if (!disposed) notice.textContent = t("Could not copy terminal selection. Try copying again.");
  }, searchView.open);

  const invoke = async <T = unknown>(operation: string, args: unknown): Promise<T> => output(await api.invoke<T>(operation, args));
  const showError = (reason: unknown) => { if (!disposed) notice.textContent = String(reason); };

  /** 静态标签集中在这里刷新，语言切换不重建实例或连接。 */
  const applyLocale = () => {
    const tr = createTranslator(api.i18n);
    t = tr;
    searchView.refreshLocale();
    label.textContent = tr("Serial");
    terminalMode.textContent = tr("Terminal");
    uartMode.textContent = tr("UART console");
    ports.textContent = tr("Ports");
    connect.textContent = connected ? tr("Disconnect") : tr("Connect");
    state.textContent = connected ? tr("Connected") : tr("Disconnected");
    viewText.textContent = tr("ASCII");
    viewHex.textContent = tr("HEX");
    timestampsText.textContent = tr("Show timestamps");
    autoscrollText.textContent = tr("Auto-scroll");
    pause.textContent = paused ? tr("Resume") : tr("Pause");
    clear.textContent = tr("Clear");
    uartEntryText.textContent = tr("Send");
    uartText.placeholder = tr("Send text, then Send (Enter sends, Shift+Enter adds a line)");
    uartHexText.textContent = tr("Input is HEX");
    uartEndingLabelText.textContent = tr("Line ending");
    uartSendButton.textContent = tr("Send");
    repeatLabelText.textContent = tr("Repeat");
    repeatButton.textContent = repeatTimer ? tr("Stop repeating") : tr("Start repeating");
    uartOutput.dataset.empty = tr("Receive");
    dtrButton.textContent = `${tr("DTR")} ${signals.dtr ? tr("High") : tr("Low")}`;
    rtsButton.textContent = `${tr("RTS")} ${signals.rts ? tr("High") : tr("Low")}`;
    dtrButton.setAttribute("aria-pressed", String(signals.dtr));
    rtsButton.setAttribute("aria-pressed", String(signals.rts));
    renderStats();
    renderUart(true);
  };

  const renderStats = () => {
    stats.textContent = t("RX {rx} B · TX {tx} B", { rx: statsRx, tx: statsTx });
  };

  const setSignals = (next: Signals) => {
    signals = next;
    dtrButton.textContent = `${t("DTR")} ${next.dtr ? t("High") : t("Low")}`;
    rtsButton.textContent = `${t("RTS")} ${next.rts ? t("High") : t("Low")}`;
    dtrButton.setAttribute("aria-pressed", String(next.dtr));
    rtsButton.setAttribute("aria-pressed", String(next.rts));
  };

  const setConnected = (next: boolean) => {
    connected = next;
    state.textContent = next ? t("Connected") : t("Disconnected");
    connect.textContent = next ? t("Disconnect") : t("Connect");
    uartSendButton.disabled = !next || working;
    ports.disabled = working;
    dtrButton.disabled = !next || working;
    rtsButton.disabled = !next || working;
    if (!next) stopRepeat(t("Repeat stopped: the port is disconnected."));
    else repeatButton.disabled = working;
  };

  const resize = () => {
    if (disposed || terminalPanel.hidden) return;
    if (terminalContainer.clientWidth === 0 || terminalContainer.clientHeight === 0) return;
    fit.fit();
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
    for (const item of uartView === "hex" ? hexEntries : entries) {
      const prefix = timestamps.checked ? `[${stamp(item.time)}] ` : "";
      text += prefix + (uartView === "hex" ? item.hex : item.text);
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
    });
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
    await new Promise<void>((resolve) => terminal.write(data, resolve));
  };

  const refreshHex = async () => {
    // Merely mounting a terminal or using text mode never invokes the byte interface.
    for (let batch = 0; batch < 257 && !disposed && mode === "uart" && uartView === "hex" && connectionId; batch++) {
      const data = await invoke<HexOutput>("serial.read_hex", { cursor: hexCursor });
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
    const status = await invoke<Status>("serial.status", {});
    if (disposed) return;
    label.textContent = `${status.config.port}@${status.config.baudRate}`;
    if (connectionId !== status.connectionId) {
      connectionId = status.connectionId;
      cursor = 0;
      statsRx = 0;
      statsTx = 0;
      if (connectionId) clearLog();
    }
    setSignals(status.signals ?? { dtr: status.config.dtr, rts: status.config.rts });
    commandQueue = status.commandQueue ?? 0;
    statsTx = status.txBytes;
    renderStats();
    setConnected(status.connected);
    connect.disabled = working;
    if (!connectionId) return;
    // Replay only on mount, connection change or a reported gap; never idle polling.
    for (let batch = 0; batch < 257 && !disposed; batch++) {
      const data = await invoke<Output>("serial.read", { cursor });
      if (disposed || data.connectionId !== connectionId) return;
      if (data.gap) {
        terminal.reset();
        terminal.write(`\r\n[${t("Output gap: older data expired from the replay buffer")}]\r\n`);
      }
      await write(data.text);
      appendOutput(data.text, data.bytes);
      cursor = data.cursor;
      if (!data.more) break;
      if (batch === 256) notice.textContent = t("Replay limit reached; waiting for the next output event.");
    }
    await refreshHex();
    if (connected) resize();
  };

  const enqueue = (action: () => Promise<void>) => {
    const next = updates.then(async () => { if (!disposed) await action(); });
    updates = next.catch(showError);
    return updates;
  };

  const sendInput = (operation: "serial.write" | "serial.write_hex", args: {text: string; lineEnding: string} | {hex: string}, bytes: number) => {
    if (disposed || !connected || working) return;
    if (commandQueue > 0) {
      notice.textContent = t("MCP command queue is busy. Wait for it to finish before sending.");
      return;
    }
    if (queuedBytes + bytes > 16 * 1024) {
      notice.textContent = t("Input queue is full. Wait before sending more.");
      return;
    }
    queuedBytes += bytes;
    inputQueue = inputQueue.then(async () => {
      if (!disposed && connected && (operation !== "serial.write_hex" || (mode === "uart" && uartHex.checked))) {
        // 插件可能补上行结束符，计数以插件回报的实际字节为准。
        const written = await invoke<{ bytes: number }>(operation, args);
        statsTx += written.bytes;
        renderStats();
      }
    }).catch(showError).finally(() => { queuedBytes -= bytes; });
  };

  const sendText = (text: string, lineEnding: string) => {
    const endingBytes = lineEnding === "crlf" ? 2 : lineEnding === "none" ? 0 : 1;
    sendInput("serial.write", {text, lineEnding}, new TextEncoder().encode(text).length + endingBytes);
  };

  const stopRepeat = (reason?: string) => {
    if (!repeatTimer) return;
    window.clearInterval(repeatTimer);
    repeatTimer = 0;
    repeatButton.textContent = t("Start repeating");
    repeatButton.setAttribute("aria-pressed", "false");
    if (reason && !disposed) notice.textContent = reason;
  };

  const startEvents = async () => {
    const cancel = await api.subscribeEvents((event) => enqueue(async () => {
      if (event.name === "runtime.closed") { setConnected(false); notice.textContent = t("Plugin connection closed."); return; }
      if (event.name === "runtime.gap" || event.name === "serial.status") { await refreshSnapshot(); return; }
      if (event.name === "serial.command") {
        const progress = event.payload as { connectionId: string; txBytes: number; finished: boolean };
        if (progress.connectionId !== connectionId) return;
        statsTx = progress.txBytes;
        renderStats();
        if (progress.finished) await refreshSnapshot();
        return;
      }
      if (event.name !== "serial.output") return;
      const data = event.payload as PushedOutput;
      if (typeof data.text !== "string" || typeof data.bytes !== "number" || typeof data.connectionId !== "string" || !Number.isSafeInteger(data.cursor) || data.cursor < 0) {
        throw new Error("Invalid serial output event");
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
  };

  const setMode = (next: "terminal" | "uart") => {
    if (mode !== next) stopRepeat();
    mode = next;
    terminalMode.setAttribute("aria-pressed", String(next === "terminal"));
    uartMode.setAttribute("aria-pressed", String(next === "uart"));
    terminalPanel.hidden = next !== "terminal";
    uartPanel.hidden = next !== "uart";
    if (next === "uart") {
      searchView.close(false);
      renderUart(true);
      void enqueue(refreshHex);
      uartText.focus();
    } else {
      requestAnimationFrame(() => {
        if (disposed) return;
        resize();
        terminal.refresh(0, terminal.rows - 1);
        terminal.focus();
      });
    }
  };

  terminalMode.addEventListener("click", () => setMode("terminal"), { signal: controller.signal });
  uartMode.addEventListener("click", () => setMode("uart"), { signal: controller.signal });

  connect.addEventListener("click", async () => {
    if (working || disposed) return;
    working = true;
    connect.disabled = true;
    uartSendButton.disabled = true;
    repeatButton.disabled = true;
    notice.textContent = "";
    try {
      if (connected) {
        await invoke("serial.disconnect", {});
        setConnected(false);
      } else {
        await invoke("serial.connect", {});
        if (disposed) return;
        await enqueue(refreshSnapshot);
        if (mode === "terminal") terminal.focus(); else uartText.focus();
      }
    } catch (reason) { showError(reason); }
    finally { if (!disposed) { working = false; connect.disabled = false; setConnected(connected); } }
  }, { signal: controller.signal });

  const uartInput = (): (() => void) | null => {
    if (!uartText.value) return null;
    try {
      if (uartHex.checked) {
        const bytes = fromHex(uartText.value);
        const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
        return () => sendInput("serial.write_hex", {hex}, bytes.length);
      }
      const text = uartText.value;
      const ending = uartEnding.value;
      return () => sendText(text, ending);
    } catch (reason) {
      showError(reason);
      return null;
    }
  };

  const sendUart = () => {
    if (mode === "uart") uartInput()?.();
  };
  uartHex.addEventListener("change", () => { stopRepeat(); uartEnding.disabled = uartHex.checked; }, { signal: controller.signal });
  uartSendButton.addEventListener("click", sendUart, { signal: controller.signal });
  uartText.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); sendUart(); }
  }, { signal: controller.signal });

  repeatButton.addEventListener("click", () => {
    if (disposed) return;
    if (repeatTimer) { stopRepeat(); return; }
    const interval = Number(repeatMs.value);
    if (!Number.isFinite(interval) || interval < REPEAT_MIN_MS || interval > REPEAT_MAX_MS) {
      notice.textContent = t("Repeat interval must be between 20 and 60000 ms.");
      return;
    }
    // 重复发送复用同一条输入队列，不绕过节流和连接检查。
    const send = uartInput();
    if (!send) return;
    notice.textContent = t("Repeat every {ms} ms", { ms: interval });
    repeatTimer = window.setInterval(() => {
      if (!connected) { stopRepeat(t("Repeat stopped: the port is disconnected.")); return; }
      send();
    }, interval);
    repeatButton.textContent = t("Stop repeating");
    repeatButton.setAttribute("aria-pressed", "true");
  }, { signal: controller.signal });

  viewText.addEventListener("click", () => { uartView = "text"; updateViewButtons(); renderUart(true); }, { signal: controller.signal });
  viewHex.addEventListener("click", () => { uartView = "hex"; updateViewButtons(); void enqueue(refreshHex); }, { signal: controller.signal });
  const updateViewButtons = () => {
    viewText.setAttribute("aria-pressed", String(uartView === "text"));
    viewHex.setAttribute("aria-pressed", String(uartView === "hex"));
  };
  timestamps.addEventListener("change", () => renderUart(true), { signal: controller.signal });
  autoscroll.addEventListener("change", () => { if (autoscroll.checked) renderUart(true); }, { signal: controller.signal });
  clear.addEventListener("click", () => {
    clearLog();
    if (paused) { paused = false; pause.textContent = t("Pause"); }
    pendingWhilePaused = 0;
    notice.textContent = "";
  }, { signal: controller.signal });
  pause.addEventListener("click", () => {
    paused = !paused;
    pause.textContent = paused ? t("Resume") : t("Pause");
    pause.setAttribute("aria-pressed", String(paused));
    if (paused) {
      pendingWhilePaused = 0;
    } else {
      if (pendingWhilePaused > 0) notice.textContent = t("The device sent {bytes} bytes while paused.", { bytes: pendingWhilePaused });
      pendingWhilePaused = 0;
      renderUart(true);
    }
  }, { signal: controller.signal });

  const toggleSignal = async (name: "dtr" | "rts") => {
    if (!connected || working || disposed) return;
    const next = { ...signals, [name]: !signals[name] };
    dtrButton.disabled = true;
    rtsButton.disabled = true;
    try {
      const result = await invoke<Signals>("serial.signals", { [name]: next[name] });
      if (!disposed) setSignals(result);
    } catch (reason) { showError(reason); }
    finally { if (!disposed) { dtrButton.disabled = !connected; rtsButton.disabled = !connected; } }
  };
  dtrButton.addEventListener("click", () => void toggleSignal("dtr"), { signal: controller.signal });
  rtsButton.addEventListener("click", () => void toggleSignal("rts"), { signal: controller.signal });

  // 端口枚举是只读探测，不触碰已经打开的连接。
  ports.addEventListener("click", async () => {
    if (disposed) return;
    ports.disabled = true;
    try {
      const probe = await invoke<Probe>("serial.probe", {});
      if (disposed) return;
      notice.textContent = probe.ports.length
        ? probe.ports.map((item) => `${item.portName} (${item.portType.kind}${item.portType.product ? `: ${item.portType.product}` : ""})`).join(", ")
        : t("No serial ports were reported by the operating system.");
    } catch (reason) { showError(reason); }
    finally { if (!disposed) ports.disabled = !connected && working; }
  }, { signal: controller.signal });

  // Same text contract as MCP: preserve controls, CR and escape sequences exactly.
  const onData = terminal.onData((text) => sendText(text, "none"));
  const observer = new ResizeObserver(() => {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(resize, 100);
  });
  observer.observe(terminalContainer);
  const stopLocale = api.i18n?.subscribe(() => applyLocale());
  applyLocale();
  setMode("terminal");
  void startEvents().catch(showError);

  return { unmount() {
    disposed = true;
    stopRepeat();
    controller.abort();
    stopLocale?.();
    unsubscribe?.();
    window.clearTimeout(resizeTimer);
    window.clearTimeout(renderTimer);
    observer.disconnect();
    onData.dispose();
    stopShortcuts();
    searchView.dispose();
    stopTheme();
    terminal.dispose();
    container.replaceChildren();
  } };
}
