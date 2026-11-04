import { attachTerminalPreferences, attachTerminalTheme, readTerminalPreferences, readTerminalTheme } from "@coterm/plugin-sdk/components/terminal-theme";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import terminalCSS from "@xterm/xterm/css/xterm.css?inline";
import type { PluginAPI, PluginConfigurationAPI, PluginFrontend, OperationReceipt } from "@coterm/plugin-sdk";
import { createTranslator } from "./i18n";
import { mountToasts, type ToastTone } from "@coterm/plugin-sdk/components/toasts";
import { createFormatSwitch, type Format } from "./format-switch";
import { createToolbar } from "./toolbar";
import { createCommunicationSettings, type SerialConfig } from "./console-settings";
import { createReceiveNewlineNormalizer, type ReceiveNewlineMode } from "./receive-newlines";
import { readSerialDisplayPreferences, saveSerialDisplayPreferences } from "./display-preferences";
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
  textarea { resize: none; font-family: var(--coterm-font-mono, Consolas, monospace); min-height: 68px; }
  [hidden] { display: none !important; }
  .pair { display: grid; grid-template-columns: minmax(0, 1fr) 130px; gap: 12px; }
  @media (max-width: 560px) { .pair { grid-template-columns: minmax(0, 1fr); } }
  .triple { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
  .checks { display: flex; flex-wrap: wrap; gap: 16px; }
  .checks label { display: flex; align-items: center; gap: 7px; color: var(--coterm-text-primary, #cccccc); }
  .checks input { width: auto; }
  .error { color: var(--coterm-danger, #f48771); overflow-wrap: anywhere; margin: 0; }
  .hint { color: var(--coterm-text-muted, #969696); font-size: 11px; overflow-wrap: anywhere; margin: 0; }
  .layout { position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; overflow: hidden; background: var(--coterm-surface, #1e1e1e); }
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
  .bar .spacer { margin-left: auto; }
  .bar button { font-size: 11px; padding: 4px 10px; }
  .bar .connect-button[data-connected="true"], .repeat-button[aria-pressed="true"] {
    border-color: var(--coterm-success, hsl(var(--success, 149 58% 38%)));
    background: color-mix(in srgb, var(--coterm-success, hsl(var(--success, 149 58% 38%))) 26%, var(--coterm-surface-raised, #252526));
  }
  .icon-button { display: inline-grid; place-items: center; min-width: 30px; min-height: 28px; padding: 5px; }
  .icon-button svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 1.5; }
  .format-switch { position: relative; display: inline-grid; grid-template-columns: 1fr 1fr; width: 116px;
    padding: 2px; border: 1px solid var(--coterm-border-input, #454545); border-radius: var(--radius, 6px);
    background: var(--coterm-surface, #1e1e1e); touch-action: pan-y; }
  .format-thumb { position: absolute; inset: 2px auto 2px 2px; width: calc((100% - 4px) / 2);
    border-radius: max(2px, calc(var(--radius, 6px) - 2px)); background: var(--coterm-surface-active, #37373d);
    transition: transform 160ms ease-out; pointer-events: none; }
  .format-switch[data-value="hex"] .format-thumb { transform: translateX(100%); }
  .format-switch button { position: relative; min-height: 25px; padding: 3px 8px; background: transparent; border: 0; border-radius: 4px; font-size: 11px; }
  .format-switch button[aria-pressed="true"] { background: transparent; color: var(--coterm-text-bright, #ffffff); }
  .modes { display: inline-flex; }
  .modes button { border-radius: 0; }
  .modes button:first-child { border-radius: 2px 0 0 2px; }
  .modes button:last-child { border-radius: 0 2px 2px 0; border-left: 0; }
  .panel { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .panel[hidden] { display: none; }
  .receive-area { position: relative; display: flex; flex: 1; min-height: 0; overflow: hidden; isolation: isolate; }
  .settings-panel {
    position: absolute; z-index: 2; inset: 0 0 auto;
    max-height: 100%; overflow: auto; padding: 16px;
    border-bottom: 1px solid var(--coterm-border-input, #454545);
    background: var(--coterm-surface-raised, #252526);
    box-shadow: 0 10px 18px #0003;
    transform: translateY(-102%); opacity: 0; visibility: hidden;
    transition: transform 180ms ease-out, opacity 180ms ease-out, visibility 180ms;
  }
  .settings-panel[data-open="true"] { transform: translateY(0); opacity: 1; visibility: visible; }
  .settings-panel form { margin: 0; }
  .settings-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
  .settings-grid .signal-check { display: flex; flex-direction: row; align-items: center; gap: 7px; align-self: end; min-height: 34px; }
  .signal-check input { width: auto; }
  .settings-grid [aria-disabled="true"] { opacity: .65; cursor: not-allowed; }
  .settings-grid label:has(.receive-filename) { grid-column: span 2; }
  .receive-newlines { grid-column: 1 / -1; }
  .toolbar-item { position: relative; display: inline-flex; flex: 0 0 auto; align-items: center; }
  .toolbar-item[data-control="stats"] { margin-left: auto; }
  .uart-bar[data-customizing="true"] .toolbar-item { cursor: grab; user-select: none;
    outline: 1px dashed var(--coterm-border-input, #454545); outline-offset: 2px; border-radius: 3px; }
  .uart-bar[data-customizing="true"] .toolbar-item:focus-visible { outline: 2px solid var(--coterm-focus, #007fd4); }
  .uart-bar[data-customizing="true"] .toolbar-item > * { transform-origin: 50% 50%;
    animation: serial-toolbar-wiggle var(--wiggle-period, 280ms) ease-in-out var(--wiggle-delay, 0ms) infinite; will-change: transform; }
  .uart-bar[data-customizing="true"] .toolbar-item[data-dragging="true"] > * { animation-play-state: paused; }
  @keyframes serial-toolbar-wiggle {
    0%, 100% { transform: translate3d(-.4px, 0, 0) rotate(-1.4deg); }
    25% { transform: translate3d(0, -.4px, 0) rotate(.5deg); }
    50% { transform: translate3d(.4px, 0, 0) rotate(1.4deg); }
    75% { transform: translate3d(0, .4px, 0) rotate(-.5deg); }
  }
  .toolbar-item[data-dragging="true"] { opacity: .5; cursor: grabbing; }
  .toolbar-item[data-drop-side]::before { content: ""; position: absolute; top: -3px; bottom: -3px; width: 2px;
    background: var(--coterm-focus, #007fd4); pointer-events: none; }
  .toolbar-item[data-drop-side="before"]::before { left: -5px; }
  .toolbar-item[data-drop-side="after"]::before { right: -5px; }
  .toolbar-customization { position: absolute; z-index: 3; inset: var(--toolbar-height, 0px) 0 auto; width: 100%;
    max-height: calc(100% - var(--toolbar-height, 0px)); overflow: auto; padding: 8px 14px; display: flex; flex-wrap: wrap; align-items: center; gap: 8px;
    border-bottom: 1px solid var(--coterm-border, #333333); background: var(--coterm-surface-raised, #252526); }
  .toolbar-customization > .hint { flex: 1 1 180px; }
  .toolbar-edit-actions { display: flex; flex-wrap: wrap; gap: 6px; margin-left: auto; }
  .toolbar-customization button { font-size: 11px; padding: 4px 8px; }
  .toolbar-removed { flex: 1 1 100%; display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .toolbar-removed > span { color: var(--coterm-text-muted, #969696); font-size: 11px; }
  .toolbar-removed button { display: inline-flex; align-items: center; gap: 5px; }
  .toolbar-removed svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-width: 1.5; }
  .toolbar-trash { position: absolute; z-index: 3; bottom: 16px; left: 50%; transform: translateX(-50%);
    display: flex; align-items: center; gap: 10px; width: max-content; max-width: calc(100% - 28px); min-height: 54px; padding: 12px 18px;
    border: 1px dashed var(--coterm-border-input, #454545); border-radius: var(--radius, 8px);
    background: var(--coterm-surface-raised, #252526); box-shadow: 0 4px 12px #0002; }
  .toolbar-trash > * { pointer-events: none; }
  .toolbar-trash svg { flex: 0 0 22px; width: 22px; height: 22px; fill: none; stroke: currentColor; stroke-width: 1.5; }
  .toolbar-trash[data-over="true"] { color: var(--coterm-danger, #e86464); border-color: currentColor; border-style: solid;
    background: color-mix(in srgb, var(--coterm-danger, #e86464) 13%, var(--coterm-surface-raised, #252526)); }
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
    font: var(--coterm-terminal-font-size, 12px)/1.45 var(--coterm-font-mono, Consolas, "Liberation Mono", monospace);
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
    grid-template-columns: minmax(0, 1fr) 34px minmax(190px, auto);
    gap: 8px;
    align-items: stretch;
    border-top: 1px solid var(--coterm-border, #333333);
    padding: 8px 14px;
  }
  .uart-send label { gap: 4px; }
  .uart-send .entry { position: relative; min-width: 0; }
  .uart-send label.entry { height: 100%; }
  .uart-send .entry textarea { height: 100%; border-radius: var(--radius, 6px); font-size: var(--coterm-terminal-font-size, 13px); }
  .file-actions { display: grid; gap: 4px; grid-template-rows: 1fr 1fr; }
  .file-actions button { display: grid; place-items: center; padding: 4px; }
  .file-actions svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 1.5; }
  .send-controls { display: grid; gap: 8px; align-content: center; }
  .send-controls .inline { gap: 8px; }
  .send-controls button { white-space: nowrap; }
  .send-history { position: absolute; z-index: 3; inset: auto 0 calc(100% + 8px); max-height: 240px; overflow: auto;
    border: 1px solid var(--coterm-border-input, #454545); background: var(--coterm-surface-raised, #252526); padding: 6px;
    border-radius: var(--radius, 6px); box-shadow: 0 -6px 18px #0003; }
  .send-history button { display: block; width: 100%; text-align: left; margin-bottom: 4px; font-family: var(--coterm-font-mono, Consolas, monospace);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .send-history p { margin: 8px; }
  .uart-send .inline { display: inline-flex; align-items: center; gap: 6px; }
  .uart-send .inline input { width: auto; }
  .uart-send input[type="number"] { width: 82px; border-radius: var(--radius, 6px); }
  @container serial-console (max-width: 520px) {
    .uart-send { grid-template-columns: minmax(0, 1fr) 34px; }
    .send-controls { grid-column: 1 / -1; display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; }
    .settings-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .bar { padding-inline: 8px; gap: 6px; }
    .uart-bar { padding-inline: 8px; gap: 6px; }
  }
  .layout { container: serial-console / inline-size; }
  @media (prefers-reduced-motion: reduce) {
    .settings-panel, .format-thumb { transition: none; }
    .uart-bar[data-customizing="true"] .toolbar-item > * { animation: none; will-change: auto; }
  }
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
type Config = SerialConfig;
interface Status {
  config: Config; connected: boolean; connectionId: string | null;
  signals: { dtr: boolean; rts: boolean }; rxBytes: number; txBytes: number;
  commandQueue: number;
}
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
const WRITE_LIMIT = 16 * 1024;
const HISTORY_LIMIT = 50;
const HISTORY_BYTES = 64 * 1024;

/** The plugin owns configuration fields; the base never learns UART field semantics. */
export function configure(container: HTMLElement, api: PluginConfigurationAPI): PluginFrontend {
  const t = createTranslator(api.i18n);
  const root = surface(container);
  const form = document.createElement("form");

  const portLabel = caption(t("Port"));
  const port = document.createElement("select");
  let portPlaceholder: string | null = "Loading ports…";
  let portLoadError: string | null = null;

  const showPortPlaceholder = (message: string) => {
    portPlaceholder = message;
    const placeholder = option("", t(message));
    placeholder.disabled = true;
    placeholder.selected = true;
    port.replaceChildren(placeholder);
  };

  port.required = true;
  showPortPlaceholder("Loading ports…");

  const populatePortOptions = (list: PortEntry[]) => {
    const currentVal = port.value;
    port.replaceChildren();
    if (list.length > 0) {
      portPlaceholder = null;
      for (const item of list) {
        const desc = item.portType?.product ? `${item.portName} (${item.portType.product})` : item.portName;
        port.append(option(item.portName, desc));
      }
      if (currentVal && list.some(item => item.portName === currentVal)) {
        port.value = currentVal;
      }
    } else {
      showPortPlaceholder("No serial ports detected");
    }
  };

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
    if (portPlaceholder && port.options[0]) {
      port.options[0].textContent = next(portPlaceholder);
    }
    if (portLoadError) error.textContent = next("Could not load serial ports: {reason}", {reason:portLoadError});

    submit.textContent = next("Open serial port");
  });

  let disposed = false;
  let busy = false;
  let loading = false;
  let pendingPorts: Promise<void> | undefined;
  let openingPicker = false;
  let pickerVersion = 0;
  const controller = new AbortController();

  const syncActions = () => {
    // Keep the native picker usable while its options refresh asynchronously.
    port.disabled = busy;
    submit.disabled = loading || busy || !port.value;
    form.setAttribute("aria-busy", String(loading || busy));
  };
  const readPorts = async () => {
    if (disposed || loading || busy) return;
    loading = true;
    portLoadError = null;
    error.textContent = "";
    if (!port.value) showPortPlaceholder("Loading ports…");
    syncActions();
    try {
      if (!api.invoke) throw new Error(t("Serial port discovery is unavailable."));
      const info = output(await api.invoke<{ ports?: PortEntry[] }>("serial.plugin_info", {}));
      if (disposed) return;
      if (!Array.isArray(info.ports)) {
        throw new Error(t("Serial backend does not support port discovery. Rebuild the plugin."));
      }
      populatePortOptions(info.ports);
    } catch (reason) {
      if (disposed) return;
      portLoadError = String(reason);
      showPortPlaceholder("Could not load serial ports");
      error.textContent = t("Could not load serial ports: {reason}", {reason:portLoadError});
    } finally {
      if (!disposed) { loading = false; syncActions(); }
    }
  };
  const loadPorts = () => {
    if (!pendingPorts) pendingPorts = readPorts().finally(() => { pendingPorts = undefined; });
    return pendingPorts;
  };
  const refreshForPicker = (event: Event) => {
    if (typeof port.showPicker !== "function") { void loadPorts(); return; }
    // Native menus snapshot their options on opening; populate before opening.
    event.preventDefault();
    if (disposed || busy || openingPicker) return;
    port.focus({preventScroll:true});
    const version = ++pickerVersion;
    openingPicker = true;
    void loadPorts().then(() => {
      if (disposed || busy || portLoadError || !port.value || version !== pickerVersion || root.activeElement !== port) return;
      try { port.showPicker(); }
      catch { /* Slow reads can outlive user activation; keep the refreshed list for the next click. */ }
    }).finally(() => { openingPicker = false; });
  };
  port.addEventListener("pointerdown", event => {
    if (event.button === 0 && event.target === port) refreshForPicker(event);
  }, {signal:controller.signal});
  port.addEventListener("keydown", event => {
    if (event.key === "Escape") pickerVersion++;
    if (!event.repeat && !event.ctrlKey && !event.metaKey && (
      event.key === " " || event.key === "Enter" || (event.key === "F4" && !event.altKey) ||
      (event.key === "ArrowDown" && event.altKey)
    )) refreshForPicker(event);
  }, {signal:controller.signal});
  port.addEventListener("blur", () => { pickerVersion++; }, {signal:controller.signal});
  port.addEventListener("change", syncActions, {signal:controller.signal});
  void loadPorts();

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (busy || loading || disposed) return;
    const name = port.value.trim();
    if (!name) { error.textContent = t("A serial port name is required."); return; }
    busy = true;
    syncActions();
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
      if (!disposed) { busy = false; syncActions(); }
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

interface LogEntry { time: number; bytes: number; text: string; hex: string; boundary?: boolean }

export function mount(container: HTMLElement, api: PluginAPI): PluginFrontend {
  const displayPreferences = readSerialDisplayPreferences(api.instanceConfigRef, api.pluginInstanceId);
  const root = surface(container, true);
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
  settingsButton.setAttribute("aria-controls", "serial-settings");
  const toolbarButton = document.createElement("button");
  toolbarButton.type = "button"; toolbarButton.className = "toolbar-toggle icon-button";
  toolbarButton.setAttribute("aria-expanded", "false"); toolbarButton.setAttribute("aria-controls", "serial-toolbar-settings");
  toolbarButton.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h7m4 0h5M4 17h2m4 0h10M11 4v6M6 14v6"/></svg>';
  bar.append(modes, spacer, connect, settingsButton, toolbarButton);
  const receiveArea = document.createElement("div");
  receiveArea.className = "receive-area";
  const settingsPanel = document.createElement("section");
  settingsPanel.id = "serial-settings";
  settingsPanel.className = "settings-panel";
  settingsPanel.tabIndex = -1;
  settingsPanel.inert = true;
  settingsPanel.setAttribute("aria-hidden", "true");
  const communication = createCommunicationSettings(createTranslator(api.i18n),
    message => notify(message, "warning"), async (name, value) => {
      if (disposed || !connected || working) return;
      const targetConnection = connectionId;
      try {
        const result = await invoke<Signals>("serial.signals", { [name]: value });
        if (!disposed && connected && connectionId === targetConnection) communication.signals(result);
      } catch (reason) { showError(reason); }
    });
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
  communication.element.append(receiveNewlines);
  let receiveNormalizer = createReceiveNewlineNormalizer(receiveNewlineMode);
  const backspaceLabel = caption(createTranslator(api.i18n)("Backspace key"));
  const backspaceLabelText = backspaceLabel.firstElementChild as HTMLSpanElement;
  const backspaceKey = document.createElement("select");
  backspaceKey.name = "backspaceKey";
  backspaceKey.append(option("bs", "BS (0x08)"), option("del", "DEL (0x7F)"));
  backspaceKey.value = displayPreferences.backspace;
  backspaceLabel.append(backspaceKey);
  communication.element.append(backspaceLabel);

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
  receiveFilename.value = "serial-receive.txt";
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
  ], createTranslator(api.i18n));
  toolbar.panel.id = "serial-toolbar-settings";

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
  historyButton.setAttribute("aria-controls", "serial-send-history");
  historyButton.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 12a9 9 0 1 0 2.6-6.4L3 8M3 3v5h5M12 7v5l4 2"/></svg>';
  const historyPanel = document.createElement("div");
  historyPanel.id = "serial-send-history";
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
  uartEnding.value = "crlf";
  uartEndingLabel.append(uartEnding);
  communication.element.append(uartEndingLabel, receiveFilenameLabel);
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
  const stopTheme = attachTerminalTheme(terminalContainer, (theme) => { terminal.options.theme = theme; });
  const stopPreferences = attachTerminalPreferences(terminalContainer, (preferences) => {
    Object.assign(terminal.options, preferences);
    resize();
  });

  let disposed = false;
  let connected = false;
  let currentConfig: Config | null = null;
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
  let uartView: "text" | "hex" = displayPreferences.receiveFormat;
  const saveDisplayPreferences = () => {
    saveSerialDisplayPreferences(api.instanceConfigRef, {
      view: mode, receiveFormat: uartView, timestamps: timestamps.checked, autoScroll: autoscroll.checked,
      receiveNewlines: receiveNewlineMode, backspace: backspaceKey.value === "del" ? "del" : "bs",
    }, api.pluginInstanceId);
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
  // 语言切换只替换翻译函数；已渲染的日志文本保持原文。
  let t = createTranslator(api.i18n);
  const searchView = mountTerminalSearch(bar, terminal, searchAddon, (message, values) => t(message, values));
  const stopShortcuts = attachTerminalShortcuts(terminal, () => {
    notify(t("Could not copy terminal selection. Try copying again."), "error");
  }, searchView.open);

  const invoke = async <T = unknown>(operation: string, args: unknown): Promise<T> => output(await api.invoke<T>(operation, args));
  const notify = (message: string, tone: ToastTone = "info") => { if (!disposed) toasts.show({ message, tone }); };
  const showError = (reason: unknown) => notify(reason instanceof Error ? reason.message : String(reason), "error");
  const renderConnection = () => {
    connect.dataset.connected = String(connected);
    connect.textContent = working ? t("Working…") : connected ? t("Connected") : t("Connect");
    connect.title = connected ? t("Disconnect") : t("Connect");
    connect.setAttribute("aria-label", connect.title);
    connect.setAttribute("aria-pressed", String(connected));
    connect.disabled = working;
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
    uartMode.textContent = tr("UART console");
    renderConnection();
    settingsButton.textContent = tr("Settings");
    settingsPanel.setAttribute("aria-label", tr("Serial settings"));
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

  const setSignals = (next: Signals) => communication.signals(next);

  const setConnected = (next: boolean) => {
    connected = next;
    if (currentConfig) communication.update(currentConfig, next);
    renderConnection();
    repeatButton.disabled = !next || working;
    if (!next) { inputGeneration++; stopRepeat(t("Repeat stopped: the port is disconnected.")); }
    communication.setBusy(working);
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
    currentConfig = status.config;
    if (connectionId !== status.connectionId) {
      connectionId = status.connectionId;
      cursor = 0;
      statsRx = 0;
      statsTx = 0;
      if (connectionId) clearLog();
    }
    setSignals(status.signals ?? { dtr: status.config.dtr, rts: status.config.rts });
    commandQueue = status.commandQueue ?? 0;
    if (commandQueue > 0) stopRepeat(t("MCP command queue is busy. Wait for it to finish before sending."));
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

  const sendInput = (operation: "serial.write" | "serial.write_hex", args: {text: string; lineEnding: string} | {hex: string}, bytes: number): Promise<boolean> => {
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
        // 插件可能补上行结束符，计数以插件回报的实际字节为准。
        const written = await invoke<{ bytes: number }>(operation, args);
        if (disposed || target !== connectionId) return false;
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
    return sendInput("serial.write", {text, lineEnding}, new TextEncoder().encode(text).length + endingBytes);
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
      if (connected) {
        await invoke("serial.disconnect", {});
        setConnected(false);
      } else {
        let parameters: ReturnType<typeof communication.prepare>;
        try { parameters = communication.prepare(); }
        catch (reason) { notify(reason instanceof Error ? reason.message : String(reason), "warning"); return; }
        if (parameters) {
          await invoke("serial.configure", parameters);
          if (disposed) return;
          await enqueue(refreshSnapshot);
          if (currentConfig) communication.update(currentConfig, false, true);
        }
        if (disposed) return;
        await invoke("serial.connect", {});
        if (disposed) return;
        await enqueue(refreshSnapshot);
        if (!settingsOpen) { if (mode === "terminal") terminal.focus(); else uartText.focus(); }
      }
    } catch (reason) { showError(reason); }
    finally { if (!disposed) { working = false; connect.disabled = false; setConnected(connected); } }
  };

  const startEvents = async () => {
    const cancel = await api.subscribeEvents((event) => enqueue(async () => {
      if (event.name === "runtime.closed") { setConnected(false); notify(t("Plugin connection closed."), "warning"); return; }
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

  const setMode = (next: "terminal" | "uart", remember = true) => {
    if (mode !== next) { stopRepeat(); inputGeneration++; closeHistory(); }
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

  terminalMode.addEventListener("click", () => setMode("terminal"), { signal: controller.signal });
  uartMode.addEventListener("click", () => setMode("uart"), { signal: controller.signal });

  connect.addEventListener("click", doConnect, { signal: controller.signal });

  const uartInput = (): (() => Promise<boolean>) | null => {
    if (!uartText.value) return null;
    try {
      if (sendFormat.value === "hex") {
        const bytes = fromHex(uartText.value);
        const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
        return () => sendInput("serial.write_hex", {hex}, bytes.length);
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
  uartEnding.addEventListener("change", () => stopRepeat(), { signal: controller.signal });
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
      const filename = receiveFilename.value.trim().replace(/[\\/:*?"<>|\u0000-\u001f]/g, "_").replace(/^\.+/, "") || "serial-receive.txt";
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
  const onData = terminal.onData((text) => sendText(text, "none"));
  const observer = new ResizeObserver(() => {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(resize, 100);
  });
  observer.observe(terminalContainer);
  const stopLocale = api.i18n?.subscribe(() => applyLocale());
  const downloads = new Map<string, number>();
  applyLocale();
  setMode(displayPreferences.view, false);
  void startEvents().catch(showError);

  return { unmount() {
    disposed = true;
    stopRepeat();
    controller.abort();
    stopLocale?.();
    unsubscribe?.();
    window.clearTimeout(resizeTimer);
    window.clearTimeout(renderTimer);
    for (const [url, timer] of downloads) { window.clearTimeout(timer); URL.revokeObjectURL(url); }
    downloads.clear();
    history.length = 0;
    observer.disconnect();
    onData.dispose();
    stopShortcuts();
    toolbar.unmount(); receiveFormats.unmount(); sendFormat.unmount(); toasts.unmount();
    searchView.dispose();
    stopTheme(); stopPreferences();
    terminal.dispose();
    container.replaceChildren();
  } };
}
