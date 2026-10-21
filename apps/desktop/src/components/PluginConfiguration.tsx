import { useEffect, useRef, useState } from "react";
import type { PluginFrontend } from "../plugin-api";
import { loadPluginFrontend } from "../plugin-loader";

export function PluginConfiguration({ pluginId, onSubmit }: { pluginId: string; onSubmit(config: unknown): Promise<void> }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const submitRef = useRef(onSubmit);
  submitRef.current = onSubmit;
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let frontend: PluginFrontend | undefined;
    const container = containerRef.current;
    if (!container) return;
    void loadPluginFrontend(pluginId).then((module) => {
      if (disposed) return;
      if (!module?.configure) throw new Error("This plugin does not provide a configuration screen. Use JSON configuration below.");
      frontend = module.configure(container, { submit: (config) => submitRef.current(config) });
    }).catch((reason) => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; frontend?.unmount(); container.replaceChildren(); };
  }, [pluginId]);
  return <>{error && <p role="alert" className="mb-3 text-xs text-[#f48771]">{error}</p>}<div ref={containerRef} /></>;
}
