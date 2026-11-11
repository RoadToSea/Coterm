import type { DriverAPI, DriverConfigurationAPI, DriverFrontend } from "../../../../../apps/shared/index";
import { configureNetwork, mountNetwork } from "../../../../../apps/shared/components/network-connection";

/** Create the connection draft; returns a disposable view and never opens a socket. */
export function configure(container: HTMLElement, api: DriverConfigurationAPI): DriverFrontend {
  return configureNetwork(container,api,false);
}

/** Mount this instance's terminal; host invocation errors appear in the view. */
export function mount(container: HTMLElement, api: DriverAPI): DriverFrontend {
  return mountNetwork(container,api,false);
}
