import type { Translator } from "./i18n";

export interface SerialConfig {
  port: string; baudRate: number; dataBits: number; stopBits: number;
  parity: "none" | "odd" | "even"; dtr: boolean; rts: boolean;
}

/** Disconnected-only framing editor; it never opens a port or writes bytes itself. */
export function createCommunicationSettings(translate: Translator,
  apply: (config: Omit<SerialConfig, "port">) => Promise<void>) {
  let t = translate;
  let busy = false;
  let externalBusy = false;
  let connected = false;
  const form = document.createElement("form");
  form.className = "communication-settings";
  const fields = document.createElement("fieldset");
  const title = document.createElement("legend");
  fields.append(title);
  const grid = document.createElement("div");
  grid.className = "settings-grid";
  const labels: [HTMLSpanElement, string][] = [];
  const field = (name: string, control: HTMLElement) => {
    const label = document.createElement("label");
    const text = document.createElement("span");
    labels.push([text, name]);
    label.append(text, control);
    grid.append(label);
  };
  const select = (values: string[]) => {
    const control = document.createElement("select");
    for (const value of values) control.add(new Option(value, value));
    return control;
  };
  const port = document.createElement("input");
  port.readOnly = true;
  port.name = "port";
  const baud = document.createElement("input");
  baud.type = "number"; baud.min = "1"; baud.max = "4294967295";
  baud.required = true; baud.name = "baudRate";
  const data = select(["5", "6", "7", "8"]); data.name = "dataBits";
  const stop = select(["1", "2"]); stop.name = "stopBits";
  const parity = select(["none", "odd", "even"]); parity.name = "parity";
  const dtr = document.createElement("input"); dtr.type = "checkbox"; dtr.name = "dtr";
  const rts = document.createElement("input"); rts.type = "checkbox"; rts.name = "rts";
  field("Port", port); field("Baud rate", baud); field("Data bits", data);
  field("Stop bits", stop); field("Parity", parity);
  const lines = document.createElement("div");
  lines.className = "checks";
  for (const [control, name] of [[dtr, "DTR high on open"], [rts, "RTS high on open"]] as const) {
    const label = document.createElement("label");
    const text = document.createElement("span"); labels.push([text, name]);
    label.append(control, text); lines.append(label);
  }
  const save = document.createElement("button"); save.type = "submit";
  save.className = "primary apply-settings";
  fields.append(grid, lines, save);
  const hint = document.createElement("p"); hint.className = "hint";
  const error = document.createElement("p"); error.className = "error"; error.setAttribute("role", "alert");
  form.append(fields, hint, error);
  const refresh = () => {
    title.textContent = t("Communication parameters");
    for (const [text, name] of labels) text.textContent = t(name);
    for (const [index, name] of ["None", "Odd", "Even"].entries()) parity.options[index].textContent = t(name);
    save.textContent = busy ? t("Applying…") : t("Apply parameters");
    fields.disabled = connected || busy || externalBusy;
    hint.textContent = t("Disconnect before applying parameters. DTR / RTS are line levels, not automatic flow control.");
  };
  form.addEventListener("submit", async event => {
    event.preventDefault();
    if (connected || busy || externalBusy || !form.reportValidity()) return;
    busy = true; error.textContent = ""; refresh();
    try {
      await apply({ baudRate: Number(baud.value), dataBits: Number(data.value), stopBits: Number(stop.value),
        parity: parity.value as SerialConfig["parity"], dtr: dtr.checked, rts: rts.checked });
    } catch (reason) { error.textContent = String(reason); }
    finally { busy = false; refresh(); }
  });
  refresh();
  return {
    element: form,
    update(config: SerialConfig, isConnected: boolean) {
      // Status events must not overwrite a user's disconnected draft.
      if (port.value === "" || connected !== isConnected) {
        port.value = config.port; baud.value = String(config.baudRate);
        data.value = String(config.dataBits); stop.value = String(config.stopBits);
        parity.value = config.parity; dtr.checked = config.dtr; rts.checked = config.rts;
      }
      connected = isConnected; refresh();
    },
    locale(next: Translator) { t = next; refresh(); },
    setBusy(value: boolean) { externalBusy = value; refresh(); },
  };
}
