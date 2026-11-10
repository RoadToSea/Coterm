import { useEffect, useRef, useState } from "react";
import { createDriverAPI, type InstanceInfo } from "../../host-api";
import type { DriverFrontend } from "../../../../shared/index";
import { loadDriverFrontend } from "../../driver-loader";

export function DriverWorkspace({ instance, configRef }: { instance: InstanceInfo; configRef?: string }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let frontend: DriverFrontend | undefined;
    const api = createDriverAPI(instance, configRef);
    const container = containerRef.current;
    if (!container) return;
    void loadDriverFrontend(instance.driverId).then((module) => {
      if (disposed || !module) return;
      frontend = module.mount(container, api);
    }).catch((reason) => {
      if (!disposed) setError(String(reason));
    });
    return () => {
      disposed = true;
      api.dispose();
      frontend?.unmount();
      container.replaceChildren();
    };
  }, [instance.driverId, instance.driverInstanceId, instance.sessionId, configRef]);
  return (
    <div className="flex h-full min-h-0 flex-col">
      {error && <p role="alert" className="px-4 py-2 text-xs text-destructive">{error}</p>}
      <div ref={containerRef} className="min-h-0 flex-1" />
    </div>
  );
}
