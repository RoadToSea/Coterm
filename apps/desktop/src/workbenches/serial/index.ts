import type { DriverAPI, DriverFrontend } from '../../../../shared/index';
import { createCommunicationSettings, type SerialConfig } from './console-settings';
import { mountStreamWorkbench } from '../common/stream-workbench';
/** Mount the shared byte/terminal workbench with UART-specific connection settings. */
export function mount(container: HTMLElement, api: DriverAPI): DriverFrontend {
  return mountStreamWorkbench<SerialConfig>(container, api, {
    namespace: "serial", settingsLabel: "Serial settings", byteLabel: "UART console",
    createSettings(form, t, warn, invoke, canInteract) {
      const settings = createCommunicationSettings(form, t, warn, async (name, value) => {
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
