import { useEffect, useRef, useState } from "react";
import { createDriverAPI, type InstanceInfo } from "../../host-api";
import { TerminalView, StreamView } from "../../workbenches/WorkbenchViews";
import { mount as mountSsh } from "../../workbenches/ssh";
import { mount as mountLocal } from "../../workbenches/local";
import { mount as mountSerial } from "../../workbenches/serial";
import { mountNetwork } from "../../workbenches/common/network-connection";

type Kind = "ssh" | "local" | "serial" | "tcp" | "telnet";
const kinds = new Set<string>(["ssh", "local", "serial", "tcp", "telnet"]);

/** React owns view lifetime; disposing a view never destroys its saved or running instance. */
export function DriverWorkspace({ instance, configRef }: { instance: InstanceInfo; configRef?: string }) {
  const container = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const kind = instance.driverId.replace(/^coterm-driver-/, "") as Kind;
  useEffect(() => {
    const element = container.current;
    if (!element || !kinds.has(kind)) return;
    const api = createDriverAPI(instance, configRef);
    setError(null);
    try {
      const controller = kind === "ssh" ? mountSsh(element, api) : kind === "local" ? mountLocal(element, api)
        : kind === "serial" ? mountSerial(element, api) : mountNetwork(element, api, kind === "telnet");
      return () => { controller.unmount(); api.dispose(); };
    } catch (reason) { api.dispose(); setError(String(reason)); }
  }, [instance.driverId, instance.driverInstanceId, instance.sessionId, configRef, kind]);
  return <div className="flex h-full min-h-0 flex-col">
    {error && <p role="alert" className="px-4 py-2 text-xs text-destructive">{error}</p>}
    <div ref={container} className="driver-workbench min-h-0 flex-1" data-workbench={kind === "ssh" || kind === "local" ? kind : "stream"}>
      {kind === "ssh" || kind === "local" ? <TerminalView kind={kind} /> : kinds.has(kind) ? <StreamView namespace={kind} /> : <p role="alert">Unsupported workbench</p>}
    </div>
  </div>;
}
