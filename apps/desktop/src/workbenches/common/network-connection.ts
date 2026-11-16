import type { DriverAPI, DriverFrontend } from "../../../../shared/index";
import { mountStreamWorkbench } from "./stream-workbench/index";
import type { Translator } from "./stream-workbench/i18n";

import type { ReceiveNewlineMode } from "./stream-workbench/receive-newlines";

type Proxy = {type:"direct"} | {type:"socks5" | "http";host:string;port:number};
interface NetworkConfig {
  host: string; port: number; connectionMode?: "client" | "server"; username: string | null;
  connectTimeoutMs: number; autoReconnect: boolean; reconnectIntervalMs: number; proxy: Proxy;
  localEcho: "auto" | "on" | "off"; receiveNewlines: ReceiveNewlineMode;
  backspace: "bs" | "del"; terminalType: "xterm" | "xterm-256color" | "vt100";
  mode: "stream" | "shell"; prompt: string | null;
}

function defaults(telnet: boolean): NetworkConfig {
  return {host:"",port:telnet ? 23 : 8080,...(telnet ? {} : {connectionMode:"client" as const}),username:null,connectTimeoutMs:5000,autoReconnect:false,
    reconnectIntervalMs:1000,proxy:{type:"direct"},localEcho:telnet ? "auto" : "on",
    receiveNewlines:telnet ? "none" : "lf",backspace:telnet ? "del" : "bs",terminalType:"xterm",
    mode:telnet ? "shell" : "stream",prompt:"# "};
}

/** Build a bounded connection draft. Read returns a validated config or throws a visible validation error. */
function createSettings(element: HTMLFormElement, telnet: boolean, translate: Translator) {
  let t = translate, dirty = false, locked = false, connected = false;
  let initial = defaults(telnet);
  let previousConnectionMode: NonNullable<NetworkConfig["connectionMode"]> = "client";
  const input = <T extends HTMLInputElement | HTMLSelectElement>(name: string): T => {
    const field=element.elements.namedItem(name);
    if (!(field instanceof HTMLInputElement || field instanceof HTMLSelectElement)) throw new Error(`Missing setting ${name}`);
    return field as T;
  };
  const host=input<HTMLInputElement>('host'),port=input<HTMLInputElement>('port');
  const connectionMode = telnet ? null : input<HTMLSelectElement>('connectionMode');
  const hostField = host.closest<HTMLElement>('[data-field]')!;
  const username=input<HTMLInputElement>('username');
  const timeout=input<HTMLInputElement>('connectTimeoutMs'), interval=input<HTMLInputElement>('reconnectIntervalMs');
  const reconnect=input<HTMLInputElement>('autoReconnect');
  const timeoutField=timeout.closest<HTMLElement>('[data-field]')!, reconnectField=reconnect.closest<HTMLElement>('[data-field]')!;
  const intervalField=interval.closest<HTMLElement>('[data-field]')!;
  const proxy=input<HTMLSelectElement>('proxy.type'),proxyHost=input<HTMLInputElement>('proxy.host'),proxyPort=input<HTMLInputElement>('proxy.port');
  const proxyHostLabel=proxyHost.closest<HTMLElement>('[data-field]')!,proxyPortLabel=proxyPort.closest<HTMLElement>('[data-field]')!;
  const proxyField=proxy.closest<HTMLElement>('[data-field]')!;
  const echo=input<HTMLSelectElement>('localEcho'),newlines=input<HTMLSelectElement>('receiveNewlines'),backspace=input<HTMLSelectElement>('backspace');
  const terminal=input<HTMLSelectElement>('terminalType');

  const prompt=input<HTMLInputElement>('prompt');
  const promptLabel=prompt.closest<HTMLElement>('[data-field]')!;
  const help: [HTMLElement, string][] = [
    [host, telnet ? "Telnet is unencrypted. Enter passwords only in the terminal." : "Raw TCP sends exact bytes."],
    [prompt, "Use the complete visible Shell prompt; trailing spaces are ignored. Shell reads require terminal mode."],
    [reconnect, "Reconnecting never replays input."],
  ];
  if (telnet) help.push([username, "Telnet is unencrypted. Enter passwords only in the terminal."]);
  const refresh = () => {
    const server = connectionMode?.value === "server";
    hostField.querySelector('label')!.textContent = t(server ? "Bind address" : "Remote host");
    timeoutField.hidden = server;
    reconnectField.hidden = server;
    intervalField.hidden = server;
    proxyField.hidden = server;
    proxyHostLabel.hidden = proxyPortLabel.hidden = server || proxy.value === "direct";
    host.title = server ? t("Choose a local IP address to accept one TCP client at a time.")
      : t(telnet ? "Telnet is unencrypted. Enter passwords only in the terminal." : "Raw TCP sends exact bytes.");
    host.setAttribute("aria-description", host.title);
    timeout.disabled = locked || server;
    reconnect.disabled = locked || server;
    proxy.disabled = locked || server;
    proxyHost.required = proxyPort.required = proxy.value !== "direct";
    proxyHost.disabled = proxyPort.disabled = locked || server || proxy.value === "direct";
    interval.disabled = locked || server || !reconnect.checked;
    promptLabel.hidden = initial.mode !== "shell"; prompt.disabled = locked || initial.mode !== "shell";
    element.title = t("Settings apply on the next connection.");
    for (const [control, message] of help) {
      control.title = `${t(message)}\n${element.title}`;
      control.setAttribute("aria-description", control.title);
    }
  };
  const write = (config: NetworkConfig) => {
    host.value = config.host; port.value = String(config.port); username.value = config.username ?? "";
    if (connectionMode) {
      connectionMode.value = config.connectionMode ?? "client";
      previousConnectionMode = connectionMode.value as NonNullable<NetworkConfig["connectionMode"]>;
    }
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
  connectionMode?.addEventListener("change", () => {
    const nextMode = connectionMode.value as NonNullable<NetworkConfig["connectionMode"]>;
    if (nextMode !== previousConnectionMode) host.value = nextMode === "server" ? "0.0.0.0" : "";
    previousConnectionMode = nextMode;
    refresh();
  });
  const read = (): NetworkConfig => {
    if (!element.reportValidity()) throw new Error(t("Check the connection parameters."));
    const validHost = (value: string) => /^[a-z0-9._:%-]{1,253}$/i.test(value);
    const server = connectionMode?.value === "server";
    if (!validHost(host.value.trim()) || (!server && proxy.value !== "direct" && !validHost(proxyHost.value.trim()))) {
      throw new Error(t("Enter a hostname or IP address without a scheme, path or port."));
    }
    const bindAddress = host.value.trim();
    const ipv4 = /^(?:\d{1,3}\.){3}\d{1,3}$/.test(bindAddress)
      && bindAddress.split(".").every(part => Number(part) <= 255);
    const ipv6Candidate = bindAddress.includes(":") && /^[0-9a-f:]+$/i.test(bindAddress);
    if (server && !ipv4 && !ipv6Candidate) {
      throw new Error(t("Enter an IPv4 or IPv6 bind address."));
    }
    if (initial.mode === "shell" && (!prompt.value.trim() || /[\x00-\x1f\x7f]/.test(prompt.value) || new TextEncoder().encode(prompt.value).length > 256)) {
      throw new Error(t("Enter a literal Shell prompt of 1–256 UTF-8 bytes."));
    }
    if (/[\x00-\x1f\x7f]/.test(username.value) || new TextEncoder().encode(username.value).length > 128) {
      throw new Error(t("Check the connection parameters."));
    }
    return {...initial,host:host.value.trim(),port:Number(port.value),...(connectionMode ? {connectionMode:connectionMode.value as NonNullable<NetworkConfig["connectionMode"]>} : {}),username:telnet && username.value ? username.value : null,
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

/** Mount the shared stream workbench with network epochs, echo negotiation and status recovery. */
export function mountNetwork(container: HTMLElement, api: DriverAPI, telnet: boolean): DriverFrontend {
  return mountStreamWorkbench<NetworkConfig>(container,api,{
    namespace:telnet ? "telnet" : "tcp",settingsLabel:telnet ? "Telnet settings" : "Raw TCP settings",
    byteLabel:"Byte console",terminalOnly:telnet,bindConnection:true,pollStatusMs:2000,
    createSettings(form, t) { return createSettings(form,telnet,t); },
    input:telnet ? text => text === "\r" ? "\r\n" : text : undefined,
    echo:status => status.config.localEcho === "on" || (status.config.localEcho === "auto" && !status.remoteEcho),
    display:config => ({receiveNewlines:config.receiveNewlines,backspace:config.backspace}),
  });
}
