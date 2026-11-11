import type { DriverAPI, DriverConfigurationAPI, DriverFrontend } from "../index";
import { mountStreamWorkbench } from "./stream-workbench";
import { createTranslator, type Translator } from "./stream-workbench/i18n";
import { caption, option, surface } from "./stream-workbench/styles";
import type { ReceiveNewlineMode } from "./stream-workbench/receive-newlines";

type Proxy = {type:"direct"} | {type:"socks5" | "http";host:string;port:number};
interface NetworkConfig {
  host: string; port: number; username: string | null;
  connectTimeoutMs: number; autoReconnect: boolean; reconnectIntervalMs: number; proxy: Proxy;
  localEcho: "auto" | "on" | "off"; receiveNewlines: ReceiveNewlineMode;
  backspace: "bs" | "del"; terminalType: "xterm" | "xterm-256color" | "vt100";
  mode: "stream" | "shell"; prompt: string | null;
}

function defaults(telnet: boolean): NetworkConfig {
  return {host:"",port:telnet ? 23 : 8080,username:null,connectTimeoutMs:5000,autoReconnect:false,
    reconnectIntervalMs:1000,proxy:{type:"direct"},localEcho:telnet ? "auto" : "on",
    receiveNewlines:telnet ? "none" : "lf",backspace:telnet ? "del" : "bs",terminalType:"xterm",
    mode:telnet ? "shell" : "stream",prompt:"# "};
}

/** Build a bounded connection draft. Read returns a validated config or throws a visible validation error. */
function createSettings(telnet: boolean, translate: Translator) {
  let t = translate, dirty = false, locked = false, connected = false;
  let initial = defaults(telnet);
  const element = document.createElement("form"); element.className = "communication-settings settings-grid";
  const labels: [HTMLElement,string][] = [], choices: [HTMLOptionElement,string][] = [];
  const field = (name: string, input: HTMLInputElement | HTMLSelectElement) => {
    const label = caption(t(name)); labels.push([label.firstElementChild as HTMLElement,name]);
    label.append(input); element.append(label); return label;
  };
  const text = (name: string, max: number) => {
    const input = document.createElement("input"); input.name = name; input.maxLength = max; return input;
  };
  const number = (name: string, min: number, max: number) => {
    const input = text(name, 8); input.type = "number"; input.min = String(min); input.max = String(max);
    input.required = true; input.step = "1"; return input;
  };
  const select = (name: string, values: [string,string][]) => {
    const input = document.createElement("select"); input.name = name;
    for (const [value,label] of values) { const choice = option(value,t(label)); choices.push([choice,label]); input.append(choice); }
    return input;
  };
  const host = text("host",253); host.required = true; host.placeholder = "192.168.1.100";
  const port = number("port",1,65535), username = text("username",128);
  const timeout = number("connectTimeoutMs",100,60000), interval = number("reconnectIntervalMs",250,60000);
  const reconnect = text("autoReconnect",0); reconnect.type = "checkbox";
  field("Remote host",host); field("Port",port);
  if (telnet) field("Login username (optional)",username);
  field("Connection timeout (ms)",timeout);
  const reconnectLabel = field("Reconnect after disconnection",reconnect); reconnectLabel.className = "signal-check";
  field("Reconnect interval (ms)",interval);
  const proxy = select("proxyType",[["direct","Direct"],["socks5","SOCKS5 (no authentication)"],["http","HTTP CONNECT (no authentication)"]]);
  const proxyHost = text("proxyHost",253), proxyPort = number("proxyPort",1,65535);
  field("Proxy",proxy); const proxyHostLabel = field("Proxy host",proxyHost), proxyPortLabel = field("Proxy port",proxyPort);
  const echo = select("localEcho",[["auto","Automatic"],["on","On"],["off","Off"]]);
  const newlines = select("receiveNewlines",[["none","None"],["lf","Complete LF with CR"],["cr","Complete CR with LF"],["both","Complete CR and LF"]]);
  const backspace = select("backspace",[["bs","BS (0x08)"],["del","DEL (0x7F)"]]);
  const terminal = select("terminalType",[["xterm","xterm"],["xterm-256color","xterm-256color"],["vt100","vt100"]]);
  field("Local echo",echo); field("Receive newline completion",newlines); field("Backspace key",backspace);
  if (telnet) field("Terminal type",terminal);
  const prompt = text("prompt",256); const promptLabel = field("Shell prompt",prompt);
  const hint = document.createElement("p"); hint.className = "hint"; hint.style.gridColumn = "1 / -1"; element.append(hint);
  const refresh = () => {
    for (const [node,label] of labels) node.textContent = t(label);
    for (const [node,label] of choices) node.textContent = t(label);
    proxyHostLabel.hidden = proxyPortLabel.hidden = proxy.value === "direct";
    proxyHost.required = proxyPort.required = proxy.value !== "direct";
    proxyHost.disabled = proxyPort.disabled = locked || proxy.value === "direct";
    interval.disabled = locked || !reconnect.checked;
    promptLabel.hidden = initial.mode !== "shell"; prompt.disabled = locked || initial.mode !== "shell";
    hint.textContent = t(telnet
      ? "Telnet is unencrypted. Enter passwords only in the terminal. Use the complete visible Shell prompt; trailing spaces are ignored. Settings apply on the next connection."
      : "Raw TCP sends exact bytes. Shell reads require terminal mode and a complete visible prompt. Settings apply on the next connection; reconnect never replays input.");
  };
  const write = (config: NetworkConfig) => {
    host.value = config.host; port.value = String(config.port); username.value = config.username ?? "";
    timeout.value = String(config.connectTimeoutMs); reconnect.checked = config.autoReconnect; interval.value = String(config.reconnectIntervalMs);
    proxy.value = config.proxy.type; proxyHost.value = config.proxy.type === "direct" ? "" : config.proxy.host;
    proxyPort.value = String(config.proxy.type === "direct" ? 1080 : config.proxy.port);
    echo.value = config.localEcho; newlines.value = config.receiveNewlines; backspace.value = config.backspace;
    terminal.value = config.terminalType; prompt.value = config.prompt ?? "# "; refresh();
  };
  const controls = Array.from(element.querySelectorAll<HTMLInputElement | HTMLSelectElement>("input,select"));
  element.addEventListener("submit",event => event.preventDefault());
  element.addEventListener("input",() => { if (!locked) dirty = true; refresh(); });
  element.addEventListener("change",() => { if (!locked) dirty = true; refresh(); });
  const read = (): NetworkConfig => {
    if (!element.reportValidity()) throw new Error(t("Check the connection parameters."));
    const validHost = (value: string) => /^[a-z0-9._:%-]{1,253}$/i.test(value);
    if (!validHost(host.value.trim()) || (proxy.value !== "direct" && !validHost(proxyHost.value.trim()))) {
      throw new Error(t("Enter a hostname or IP address without a scheme, path or port."));
    }
    if (initial.mode === "shell" && (!prompt.value.trim() || /[\x00-\x1f\x7f]/.test(prompt.value) || new TextEncoder().encode(prompt.value).length > 256)) {
      throw new Error(t("Enter a literal Shell prompt of 1–256 UTF-8 bytes."));
    }
    if (/[\x00-\x1f\x7f]/.test(username.value) || new TextEncoder().encode(username.value).length > 128) {
      throw new Error(t("Check the connection parameters."));
    }
    return {...initial,host:host.value.trim(),port:Number(port.value),username:telnet && username.value ? username.value : null,
      connectTimeoutMs:Number(timeout.value),autoReconnect:reconnect.checked,reconnectIntervalMs:Number(interval.value),
      proxy:proxy.value === "direct" ? {type:"direct"} : {type:proxy.value as "socks5" | "http",host:proxyHost.value.trim(),port:Number(proxyPort.value)},
      localEcho:echo.value as NetworkConfig["localEcho"],receiveNewlines:newlines.value as ReceiveNewlineMode,
      backspace:backspace.value as "bs" | "del",terminalType:terminal.value as NetworkConfig["terminalType"],prompt:prompt.value};
  };
  write(initial);
  return {element,read,
    update(config: NetworkConfig, isConnected: boolean, applied = false) {
      connected = isConnected;
      initial = config;
      if (!dirty || applied || connected) { write(config); dirty = false; }
      this.setBusy(connected); refresh();
    },
    prepare(): Partial<NetworkConfig> | null { return dirty ? read() : null; },
    locale(next: Translator) { t = next; refresh(); },
    setBusy(value: boolean) { locked = value || connected; for (const control of controls) control.disabled = locked; refresh(); },
  };
}

/** Mount a network configuration form. Submission creates an instance; mounting does not open a socket. */
export function configureNetwork(container: HTMLElement, api: DriverConfigurationAPI, telnet: boolean): DriverFrontend {
  const root = surface(container), settings = createSettings(telnet,createTranslator(api.i18n));
  settings.element.classList.add("network-configuration");
  const error = document.createElement("p"); error.className = "error"; error.setAttribute("role","alert");
  const submit = document.createElement("button"); submit.type = "submit"; submit.className = "primary";
  const locale = () => { const t = createTranslator(api.i18n); settings.locale(t); submit.textContent = t("Create instance"); };
  settings.element.append(error,submit); root.append(settings.element); locale();
  let disposed = false, busy = false; const controller = new AbortController();
  const stopLocale = api.i18n?.subscribe(locale);
  settings.element.addEventListener("submit", async event => {
    event.preventDefault(); if (busy || disposed) return; error.textContent = "";
    try {
      const config = settings.read(); busy = true; settings.setBusy(true); submit.disabled = true;
      await api.submit(config);
    } catch (reason) { if (!disposed) error.textContent = reason instanceof Error ? reason.message : String(reason); }
    finally { if (!disposed) { busy = false; settings.setBusy(false); submit.disabled = false; } }
  },{signal:controller.signal});
  return {unmount() { disposed = true; controller.abort(); stopLocale?.(); container.replaceChildren(); }};
}

/** Mount the shared stream workbench with network epochs, echo negotiation and status recovery. */
export function mountNetwork(container: HTMLElement, api: DriverAPI, telnet: boolean): DriverFrontend {
  return mountStreamWorkbench<NetworkConfig>(container,api,{
    namespace:telnet ? "telnet" : "tcp",settingsLabel:telnet ? "Telnet settings" : "Raw TCP settings",
    byteLabel:"Byte console",terminalOnly:telnet,bindConnection:true,pollStatusMs:2000,
    createSettings(t) { return createSettings(telnet,t); },
    input:telnet ? text => text === "\r" ? "\r\n" : text : undefined,
    echo:status => status.config.localEcho === "on" || (status.config.localEcho === "auto" && !status.remoteEcho),
    display:config => ({receiveNewlines:config.receiveNewlines,backspace:config.backspace}),
  });
}
