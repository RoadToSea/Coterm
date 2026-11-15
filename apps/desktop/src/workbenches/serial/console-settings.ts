import type { Translator } from "../common/stream-workbench/i18n";

export interface SerialConfig {
  port: string; baudRate: number; dataBits: number; stopBits: number;
  parity: "none" | "odd" | "even"; dtr: boolean; rts: boolean;
  mode?: "stream" | "shell"; prompt?: string | null;
}
export type SerialParameters = Omit<SerialConfig, "port">;

/** A local draft: framing is applied only by the next explicit connection action. */
export function createCommunicationSettings(form: HTMLFormElement, translate: Translator, warn: (message: string) => void,
  signal: (name: "dtr" | "rts", value: boolean) => Promise<void>) {
  let t = translate;
  let connected = false, busy = false, dirty = false, signalBusy = false;
  let initial: SerialParameters | null = null;
  let levels = { dtr: false, rts: false };
  const input = <T extends HTMLInputElement | HTMLSelectElement>(name: string): T => {
    const field=form.elements.namedItem(name);
    if (!(field instanceof HTMLInputElement || field instanceof HTMLSelectElement)) throw new Error(`Missing setting ${name}`);
    return field as T;
  };
  const baud=input<HTMLInputElement>('baudRate'),data=input<HTMLSelectElement>('dataBits'),stop=input<HTMLSelectElement>('stopBits'),parity=input<HTMLSelectElement>('parity');
  const prompt=input<HTMLInputElement>('prompt'),dtr=input<HTMLInputElement>('dtr'),rts=input<HTMLInputElement>('rts');
  const promptLabel=prompt.closest<HTMLElement>('[data-field]')!;
  const framing = [baud, data, stop, parity, prompt];
  const refresh = () => {

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
