import { useEffect, useRef, useState } from "react";
import { createPluginAPI, type InstanceInfo } from "../../host-api";
import type { PluginFrontend } from "@coterm/plugin-sdk";
import { loadPluginFrontend } from "../../plugin-loader";

export function PluginWorkspace({ instance }: { instance: InstanceInfo }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let frontend: PluginFrontend | undefined;
    const api = createPluginAPI(instance);
    const container = containerRef.current;
    if (!container) return;
    void loadPluginFrontend(instance.pluginId).then((module) => {
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
  }, [instance.pluginId, instance.pluginInstanceId, instance.sessionId]);
  return (
    <div className="flex h-full min-h-0 flex-col">
      {error && <p role="alert" className="px-4 py-2 text-xs text-destructive">{error}</p>}
      <div ref={containerRef} className="min-h-0 flex-1" />
    </div>
  );
}
