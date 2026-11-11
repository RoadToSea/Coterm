import type { DriverAPI, DriverConfigurationAPI, DriverFrontend, OperationReceipt } from "../../../../../apps/shared/index";
import { createTranslator } from "../../../../../apps/shared/components/stream-workbench/i18n";
import { createCommunicationSettings, type SerialConfig } from "./console-settings";
import { surface, option, caption, retitle } from "../../../../../apps/shared/components/stream-workbench/styles";
import { mountStreamWorkbench } from "../../../../../apps/shared/components/stream-workbench";
interface PortEntry { portName: string; portType: { kind: string; product?: string | null } }
const BAUD_RATES = [300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];
function output<T>(receipt: OperationReceipt<T>): T {
  if (receipt.status !== "completed" || receipt.error || receipt.output === null) {
    throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
  }
  return receipt.output;
}
/** The driver owns configuration fields; the base never learns UART field semantics. */
export function configure(container: HTMLElement, api: DriverConfigurationAPI): DriverFrontend {
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
      const info = output(await api.invoke<{ ports?: PortEntry[] }>("serial.driver_info", {}));
      if (disposed) return;
      if (!Array.isArray(info.ports)) {
        throw new Error(t("Serial backend does not support port discovery. Rebuild the driver."));
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
        mode: "shell",
        prompt: "# ",
      });
    } catch (reason) {
      if (!disposed) error.textContent = String(reason);
    } finally {
      if (!disposed) { busy = false; syncActions(); }
    }
  }, { signal: controller.signal });

  return { unmount() { disposed = true; stopLocale?.(); controller.abort(); container.replaceChildren(); } };
}

/** Mount the shared byte/terminal workbench with UART-specific connection settings. */
export function mount(container: HTMLElement, api: DriverAPI): DriverFrontend {
  return mountStreamWorkbench<SerialConfig>(container, api, {
    namespace: "serial", settingsLabel: "Serial settings", byteLabel: "UART console",
    createSettings(t, warn, invoke, canInteract) {
      const settings = createCommunicationSettings(t, warn, async (name, value) => {
        if (!canInteract()) return;
        try { settings.signals(await invoke<{dtr:boolean;rts:boolean}>("serial.signals", {[name]:value})); }
        catch (reason) { warn(reason instanceof Error ? reason.message : String(reason)); }
      });
      return { ...settings, sync(status) {
        const signals = status.signals as {dtr:boolean;rts:boolean} | undefined;
        settings.signals(signals ?? {dtr:status.config.dtr,rts:status.config.rts});
      } };
    },
  });
}
