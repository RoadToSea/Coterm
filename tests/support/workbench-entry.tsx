import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { TerminalView, StreamView } from "../../apps/desktop/src/workbenches/WorkbenchViews";
import { DriverConfiguration } from "../../apps/desktop/src/components/drivers/DriverConfiguration";
import { mount as ssh } from "../../apps/desktop/src/workbenches/ssh";
import { mount as local } from "../../apps/desktop/src/workbenches/local";
import { mount as serial } from "../../apps/desktop/src/workbenches/serial";
import { mountNetwork } from "../../apps/desktop/src/workbenches/common/network-connection";
import type { DriverAPI, DriverConfigurationAPI } from "../../apps/shared";
import css from "../../apps/desktop/src/index.css?inline";

declare const __WORKBENCH_KIND__: "ssh" | "local" | "serial" | "tcp" | "telnet";
const kind = __WORKBENCH_KIND__;
function host(container: HTMLElement) {
  if (!document.getElementById("workbench-test-styles")) {
    const style = document.createElement("style"); style.id = "workbench-test-styles"; style.textContent = css; document.head.append(style);
  }
  const element = document.createElement("div"); element.style.height = "100%";
  element.className = "driver-workbench"; element.dataset.workbench = kind === "ssh" || kind === "local" ? kind : "stream";
  container.replaceChildren(element);
  return { element, root: createRoot(element) };
}

/** Fixture adapter exercises the same React view and controller used by the desktop. */
export function mount(container: HTMLElement, api: DriverAPI) {
  const { element, root } = host(container);
  flushSync(() => root.render(kind === "ssh" || kind === "local" ? <TerminalView kind={kind} /> : <StreamView namespace={kind} />));
  const view = kind === "ssh" ? ssh(element, api) : kind === "local" ? local(element, api) : kind === "serial" ? serial(element, api) : mountNetwork(element, api, kind === "telnet");
  return { unmount() { view.unmount(); root.unmount(); element.remove(); } };
}

/** Configuration fixtures supply their own operation channel; no Tauri backend is required. */
export function configure(container: HTMLElement, api: DriverConfigurationAPI) {
  const { element, root } = host(container);
  flushSync(() => root.render(<DriverConfiguration driverId={`coterm-driver-${kind}`} existingNames={[]} onSubmit={api.submit} configurationAPI={api} />));
  return { unmount() { root.unmount(); element.remove(); } };
}
