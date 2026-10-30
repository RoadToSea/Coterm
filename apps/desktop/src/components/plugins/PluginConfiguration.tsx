import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useEffect, useRef, useState } from "react";
import type { PluginFrontend } from "@coterm/plugin-sdk";
import { loadPluginFrontend } from "../../plugin-loader";
import { pluginI18n } from "../../i18n";

export function PluginConfiguration({ pluginId, onSubmit, disabled = false }: { pluginId: string; onSubmit(config: unknown): Promise<void>; disabled?: boolean }) {
  useI18n();
  const containerRef = useRef<HTMLDivElement>(null);
  const submitRef = useRef(onSubmit);
  submitRef.current = onSubmit;
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { if (containerRef.current) containerRef.current.inert = disabled; }, [disabled]);
  useEffect(() => {
    let disposed = false;
    let frontend: PluginFrontend | undefined;
    const container = containerRef.current;
    const subscriptions = new Set<() => void>();
    if (!container) return;
    setError(null);
    void loadPluginFrontend(pluginId).then((module) => {
      if (disposed) return;
      if (!module?.configure) throw new Error(t("This plugin does not provide a connection configuration screen."));
      frontend = module.configure(container, {
        submit: (config) => submitRef.current(config),
        i18n: {
          get locale() { return pluginI18n.locale; },
          subscribe: (listener) => {
            if (disposed) return () => {};
            const stop = pluginI18n.subscribe(listener);
            const cancel = () => { subscriptions.delete(cancel); stop(); };
            subscriptions.add(cancel);
            return cancel;
          },
        },
      });
    }).catch((reason) => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; for (const stop of subscriptions) stop(); frontend?.unmount(); container.replaceChildren(); };
  }, [pluginId]);
  return <fieldset disabled={disabled} aria-busy={disabled} className="m-0 min-w-0 border-0 p-0">
    {error && <p role="alert" className="mb-3 text-xs text-destructive">{error}</p>}
    <div ref={containerRef} />
  </fieldset>;
}
