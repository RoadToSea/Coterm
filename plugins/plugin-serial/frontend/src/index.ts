import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import terminalCSS from "@xterm/xterm/css/xterm.css?inline";
import type { PluginAPI, PluginConfigurationAPI, PluginFrontend, OperationReceipt } from "@coterm/plugin-sdk";

const css = `
  :host { display:block; height:100%; min-height:0; color:#ccc; font:13px "Segoe UI",sans-serif; }
  * { box-sizing:border-box; }
  button,input,select,textarea { font:inherit; color:inherit; }
  button { border:1px solid #454545; padding:5px 12px; background:#333; cursor:pointer; }
  button:hover { background:#414141; } button:disabled { opacity:.5; cursor:default; }
  button:focus-visible,input:focus-visible,select:focus-visible,textarea:focus-visible { outline:1px solid #007fd4; }
  .primary { background:#0e639c; color:#fff; } .primary:hover { background:#1177bb; }
  form { display:grid; gap:14px; margin-bottom:20px; }
  label { display:grid; gap:6px; color:#aaa; }
  input,select,textarea { width:100%; min-width:0; border:1px solid #454545; background:#1e1e1e; padding:7px 9px; }
  textarea { resize:vertical; font-family:Consolas,monospace; min-height:34px; }
  .pair { display:grid; grid-template-columns:minmax(0,1fr) 110px; gap:12px; }
  .triple { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:12px; }
  .error { color:#f48771; overflow-wrap:anywhere; margin:0; }
  .hint { color:#969696; font-size:11px; overflow-wrap:anywhere; margin:0; }
  .layout { display:flex; flex-direction:column; height:100%; min-height:0; background:#1e1e1e; }
  .bar { display:flex; flex-wrap:wrap; align-items:center; gap:12px; min-height:42px; border-bottom:1px solid #333; padding:6px 14px; }
  .bar strong { font-weight:500; } .bar span { font-size:11px; color:#969696; }
  .bar button { margin-left:auto; font-size:11px; }
  .terminal { flex:1; min-height:0; overflow:hidden; padding:10px 4px 0 10px; }
  .send { display:grid; grid-template-columns:minmax(0,1fr) auto auto; gap:8px; align-items:end;
          border-top:1px solid #333; padding:8px 14px; }
  .send label { gap:4px; }
  .send .line-end { width:auto; }
  .notice { padding:7px 14px; font-size:12px; color:#aaa; overflow-wrap:anywhere; }
  .notice:empty { display:none; }
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
  label.textContent = text;
  return label;
}

function field(caption_text: string, value = "", type = "text"): { label: HTMLLabelElement; input: HTMLInputElement } {
  const label = caption(caption_text);
  const input = document.createElement("input");
  input.type = type;
  input.value = value;
  label.append(input);
  return { label, input };
}

interface PortEntry { portName: string; portType: { kind: string; product?: string | null } }
interface Config {
  port: string; baudRate: number; dataBits: number; stopBits: number;
  parity: "none" | "odd" | "even";
}
interface Status { config: Config; connected: boolean; connectionId: string | null }
interface Probe { config: Config; connected: boolean; ports: PortEntry[] }
interface Output { data: string; cursor: string; gap: boolean; closed: boolean; more: boolean; connectionId: string }
interface PushedOutput { data: string; cursor: string; connectionId: string }

const BAUD_RATES = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

/** The plugin owns configuration fields; the base never learns UART field semantics. */
export function configure(container: HTMLElement, api: PluginConfigurationAPI): PluginFrontend {
  const root = surface(container);
  const form = document.createElement("form");
  const port = document.createElement("select");
  const portLabel = caption("Port");
  portLabel.append(port);
  const refresh = document.createElement("button");
  refresh.type = "button";
  refresh.textContent = "Add another";
  const portRow = document.createElement("div");
  portRow.className = "pair";
  portRow.append(portLabel, refresh);

  const baud = document.createElement("select");
  for (const rate of BAUD_RATES) baud.append(option(String(rate), String(rate)));
  baud.value = "115200";
  const baudLabel = caption("Baud rate");
  baudLabel.append(baud);

  const dataBits = document.createElement("select");
  for (const bits of ["5", "6", "7", "8"]) dataBits.append(option(bits, bits));
  dataBits.value = "8";
  const dataBitsLabel = caption("Data bits");
  dataBitsLabel.append(dataBits);

  const stopBits = document.createElement("select");
  stopBits.append(option("1", "1"), option("2", "2"));
  const stopBitsLabel = caption("Stop bits");
  stopBitsLabel.append(stopBits);

  const parity = document.createElement("select");
  parity.append(option("none", "None"), option("odd", "Odd"), option("even", "Even"));
  const parityLabel = caption("Parity");
  parityLabel.append(parity);

  const framing = document.createElement("div");
  framing.className = "triple";
  framing.append(dataBitsLabel, stopBitsLabel, parityLabel);

  const hint = document.createElement("p");
  hint.className = "hint";
  hint.textContent = "Port names come from the operating system, e.g. COM6 or /dev/ttyUSB0. "
    + "The connected list appears in the instance panel after the port is opened.";
  const error = document.createElement("p");
  error.className = "error";
  error.setAttribute("role", "alert");
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "primary";
  submit.textContent = "Open serial port";
  form.append(portRow, baudLabel, framing, hint, error, submit);
  root.append(form);

  let disposed = false;
  let busy = false;
  const controller = new AbortController();

  // 配置阶段没有实例，插件级枚举需要实例作用域，所以这里只接受手输。
  const addPort = (name: string) => {
    if (!name || Array.from(port.options).some((item) => item.value === name)) return;
    port.append(option(name, name));
    port.value = name;
  };
  addPort("COM6");

  refresh.addEventListener("click", () => {
    const name = window.prompt("Serial port name (for example COM6 or /dev/ttyUSB0)");
    if (name) addPort(name.trim());
  }, { signal: controller.signal });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (busy || disposed) return;
    const name = port.value.trim();
    if (!name) { error.textContent = "A serial port name is required."; return; }
    busy = true;
    submit.disabled = true;
    try {
      await api.submit({
        port: name,
        baudRate: Number(baud.value),
        dataBits: Number(dataBits.value),
        stopBits: Number(stopBits.value),
        parity: parity.value,
      });
    } catch (reason) {
      if (!disposed) error.textContent = String(reason);
    } finally {
      if (!disposed) { busy = false; submit.disabled = false; }
    }
  }, { signal: controller.signal });

  return { unmount() { disposed = true; controller.abort(); container.replaceChildren(); } };
}

function output<T>(receipt: OperationReceipt<T>): T {
  if (receipt.status !== "completed" || receipt.error || receipt.output === null) {
    throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
  }
  return receipt.output;
}

function base64(data: Uint8Array): string {
  let binary = "";
  for (const byte of data) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** HEX 输入是插件自己的便利功能；基座只见 base64。 */
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

export function mount(container: HTMLElement, api: PluginAPI): PluginFrontend {
  const root = surface(container, true);
  const layout = document.createElement("div");
  layout.className = "layout";
  const bar = document.createElement("div");
  bar.className = "bar";
  const label = document.createElement("strong");
  label.textContent = "Serial";
  const state = document.createElement("span");
  state.textContent = "Loading…";
  const ports = document.createElement("button");
  ports.textContent = "Ports";
  ports.disabled = true;
  const connect = document.createElement("button");
  connect.textContent = "Connect";
  connect.disabled = true;
  bar.append(label, state, ports, connect);
  const terminalContainer = document.createElement("div");
  terminalContainer.className = "terminal";

  const send = document.createElement("div");
  send.className = "send";
  const entry = caption("Send");
  const text = document.createElement("textarea");
  text.rows = 1;
  text.placeholder = "Type text, then Send (Enter sends, Shift+Enter adds a line)";
  entry.append(text);
  const ending = document.createElement("select");
  ending.className = "line-end";
  ending.append(option("cr", "CR"), option("crlf", "CRLF"), option("lf", "LF"), option("none", "None"));
  ending.value = "cr";
  const endingLabel = caption("Line ending");
  endingLabel.append(ending);
  const hex = field("Input is HEX");
  const hexInput = hex.input;
  hexInput.type = "checkbox";
  hexInput.required = false;
  const sendButton = document.createElement("button");
  sendButton.textContent = "Send";
  sendButton.disabled = true;
  send.append(entry, endingLabel, hex.label, sendButton);

  const notice = document.createElement("div");
  notice.className = "notice";
  notice.setAttribute("role", "status");
  layout.append(bar, terminalContainer, send, notice);
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
    sendButton.disabled = !next || working;
    ports.disabled = working;
  };
  const resize = () => {
    if (disposed || terminalContainer.clientWidth === 0 || terminalContainer.clientHeight === 0) return;
    fit.fit();
  };

  const write = async (data: string) => {
    if (!data || disposed) return;
    const decoded = atob(data);
    await new Promise<void>((resolve) => terminal.write(Uint8Array.from(decoded, (char) => char.charCodeAt(0)), resolve));
  };
  const refreshSnapshot = async () => {
    const status = await invoke<Status>("serial.status", {});
    if (disposed) return;
    label.textContent = `${status.config.port}@${status.config.baudRate}`;
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
      const data = await invoke<Output>("serial.read", { cursor });
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
  const sendBytes = (payload: Uint8Array, lineEnding: string) => {
    if (disposed || !connected || working) return;
    if (queuedBytes + payload.length > 16 * 1024) {
      notice.textContent = "Input queue is full. Wait before sending more.";
      return;
    }
    queuedBytes += payload.length;
    inputQueue = inputQueue.then(async () => {
      if (!disposed && connected) {
        await invoke("serial.write", { data: base64(payload), lineEnding });
      }
    }).catch(showError).finally(() => { queuedBytes -= payload.length; });
  };
  const startEvents = async () => {
    const cancel = await api.subscribeEvents((event) => enqueue(async () => {
      if (event.name === "runtime.closed") { setConnected(false); notice.textContent = "Plugin connection closed."; return; }
      if (event.name === "runtime.gap" || event.name === "serial.status") { await refreshSnapshot(); return; }
      if (event.name !== "serial.output") return;
      const data = event.payload as PushedOutput;
      if (typeof data.data !== "string" || typeof data.connectionId !== "string" || typeof data.cursor !== "string" || !/^\d+$/.test(data.cursor)) {
        throw new Error("Invalid serial output event");
      }
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
    sendButton.disabled = true;
    notice.textContent = "";
    try {
      if (connected) {
        await invoke("serial.disconnect", {});
        setConnected(false);
      } else {
        await invoke("serial.connect", {});
        if (disposed) return;
        await enqueue(refreshSnapshot);
        terminal.focus();
      }
    } catch (reason) { showError(reason); }
    finally { if (!disposed) { working = false; connect.disabled = false; sendButton.disabled = !connected; } }
  }, { signal: controller.signal });

  const sendText = () => {
    if (!text.value) return;
    let payload: Uint8Array;
    try {
      payload = hexInput.checked ? fromHex(text.value) : new TextEncoder().encode(text.value);
    } catch (reason) { showError(reason); return; }
    sendBytes(payload, ending.value);
    text.value = "";
  };
  sendButton.addEventListener("click", sendText, { signal: controller.signal });
  text.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); sendText(); }
  }, { signal: controller.signal });

  // 端口枚举是只读探测，不触碰已经打开的连接。
  ports.addEventListener("click", async () => {
    if (disposed) return;
    ports.disabled = true;
    try {
      const probe = await invoke<Probe>("serial.probe", {});
      if (disposed) return;
      notice.textContent = probe.ports.length
        ? probe.ports.map((item) => `${item.portName} (${item.portType.kind}${item.portType.product ? `: ${item.portType.product}` : ""})`).join(", ")
        : "No serial ports were reported by the operating system.";
    } catch (reason) { showError(reason); }
    finally { if (!disposed) ports.disabled = !connected; }
  }, { signal: controller.signal });

  // 终端里的按键按原样发送，不附加行结束符：这里的输入是原始字节。
  const onData = terminal.onData((data) => sendBytes(new TextEncoder().encode(data), "none"));
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
