import type { Translator } from "./i18n";

export interface SerialConfig {
  port: string; baudRate: number; dataBits: number; stopBits: number;
  parity: "none" | "odd" | "even"; dtr: boolean; rts: boolean;
  mode?: "stream" | "shell"; prompt?: string | null;
}
export type SerialParameters = Omit<SerialConfig, "port">;

/** A local draft: framing is applied only by the next explicit connection action. */
export function createCommunicationSettings(translate: Translator, warn: (message: string) => void,
  signal: (name: "dtr" | "rts", value: boolean) => Promise<void>) {
  let t = translate;
  let connected = false, busy = false, dirty = false, signalBusy = false;
  let initial: SerialParameters | null = null;
  let levels = { dtr: false, rts: false };
  const form = document.createElement("form");
  form.className = "communication-settings settings-grid";
  const labels: [HTMLSpanElement, string][] = [];
  const field = (name: string, control: HTMLElement) => {
    const label = document.createElement("label"), text = document.createElement("span");
    labels.push([text, name]); label.append(text, control); form.append(label);
    return label;
  };
  const select = (name: string, values: string[]) => {
    const input = document.createElement("select"); input.name = name;
    for (const value of values) input.add(new Option(value, value));
    return input;
  };
  const baud = document.createElement("input");
  baud.type = "number"; baud.min = "1"; baud.max = "4294967295"; baud.required = true; baud.name = "baudRate";
  const data = select("dataBits", ["5", "6", "7", "8"]);
  const stop = select("stopBits", ["1", "2"]);
  const parity = select("parity", ["none", "odd", "even"]);
  const prompt = document.createElement("input"); prompt.name = "prompt"; prompt.maxLength = 256; prompt.value = "# ";
  field("Baud rate", baud); field("Data bits", data); field("Stop bits", stop); field("Parity", parity);
  const promptLabel = field("Shell prompt", prompt);
  const dtr = document.createElement("input"); dtr.type = "checkbox"; dtr.name = "dtr";
  const rts = document.createElement("input"); rts.type = "checkbox"; rts.name = "rts";
  for (const [control, name] of [[dtr, "DTR"], [rts, "RTS"]] as const) {
    const label = document.createElement("label"), text = document.createElement("span");
    label.className = "signal-check"; labels.push([text, name]); label.append(control, text); form.append(label);
  }
  const framing = [baud, data, stop, parity, prompt];
  const refresh = () => {
    for (const [text, name] of labels) text.textContent = t(name);
    for (const [index, name] of ["None", "Odd", "Even"].entries()) parity.options[index].textContent = t(name);
    promptLabel.hidden = initial?.mode !== "shell";
    prompt.disabled = initial?.mode !== "shell"; prompt.readOnly = connected || busy;
    for (const control of framing) control.setAttribute("aria-disabled", String(connected || busy));
    baud.readOnly = connected || busy;
    dtr.disabled = busy || signalBusy; rts.disabled = busy || signalBusy;
  };
  const locked = (event: Event) => {
    if (!connected && !busy) return;
    if (event instanceof KeyboardEvent && ["Tab", "Escape"].includes(event.key)) return;
    event.preventDefault();
    warn(t("Disconnect before changing communication parameters. Changes apply on the next connection."));
  };
  for (const control of framing) {
    control.addEventListener("pointerdown", locked);
    control.addEventListener("click", locked);
    control.addEventListener("keydown", locked);
    control.addEventListener("wheel", event => { if (connected || busy) locked(event); }, { passive: false });
    control.addEventListener("input", () => { if (!connected && !busy) dirty = true; });
    control.addEventListener("change", () => { if (!connected && !busy) dirty = true; });
  }
  for (const [control, name] of [[dtr, "dtr"], [rts, "rts"]] as const) {
    control.addEventListener("change", async () => {
      if (!connected) { dirty = true; return; }
      signalBusy = true; refresh();
      try { await signal(name, control.checked); }
      finally { signalBusy = false; control.checked = connected ? levels[name] : initial?.[name] ?? false; refresh(); }
    });
  }
  form.addEventListener("submit", event => event.preventDefault());
  const write = (config: SerialParameters) => {
    baud.value = String(config.baudRate); data.value = String(config.dataBits);
    stop.value = String(config.stopBits); parity.value = config.parity;
    dtr.checked = config.dtr; rts.checked = config.rts;
    prompt.value = config.prompt ?? "# ";
  };
  refresh();
  return {
    element: form,
    update(config: SerialConfig, isConnected: boolean, applied = false) {
      const wasConnected = connected;
      connected = isConnected;
      const { port: _port, ...parameters } = config;
      if (!initial || applied || isConnected || (wasConnected && !isConnected)) {
        initial = parameters; dirty = false; write(parameters);
      } else if (!dirty) { initial = parameters; write(parameters); }
      if (initial) initial = {...initial, mode:parameters.mode, prompt:parameters.prompt};
      if (connected) { dtr.checked = levels.dtr; rts.checked = levels.rts; }
      refresh();
    },
    prepare(): SerialParameters | null {
      if (!initial || !dirty) return null;
      if (!Number.isInteger(Number(baud.value)) || Number(baud.value) < 1 || Number(baud.value) > 4294967295
        || ![5, 6, 7, 8].includes(Number(data.value)) || ![1, 2].includes(Number(stop.value))
        || !["none", "odd", "even"].includes(parity.value)) {
        throw new Error(t("Check the communication parameters before connecting."));
      }
      if (initial.mode === "shell" && (!prompt.value.trim() || /[\x00-\x1f\x7f]/.test(prompt.value) || new TextEncoder().encode(prompt.value).length > 256)) {
        throw new Error(t("Enter a literal Shell prompt of 1–256 UTF-8 bytes."));
      }
      return { baudRate: Number(baud.value), dataBits: Number(data.value), stopBits: Number(stop.value),
        parity: parity.value as SerialConfig["parity"], dtr: dtr.checked, rts: rts.checked,
        mode: initial.mode ?? "stream", prompt: initial.mode === "shell" ? prompt.value : initial.prompt ?? null };
    },
    signals(next: { dtr: boolean; rts: boolean }) {
      levels = next;
      if (connected) { dtr.checked = next.dtr; rts.checked = next.rts; }
    },
    locale(next: Translator) { t = next; refresh(); },
    setBusy(value: boolean) { busy = value; refresh(); },
  };
}
