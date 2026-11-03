import { hostAPI } from "../../host-api";
import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useEffect, useRef, useState } from "react";
import type { InvokeOptions, PluginFrontend } from "@coterm/plugin-sdk";
import { loadPluginFrontend } from "../../plugin-loader";
import { pluginI18n } from "../../i18n";

export function PluginConfiguration({ pluginId, onSubmit, disabled = false }: { pluginId: string; onSubmit(config: unknown, name?: string): Promise<void>; disabled?: boolean }) {
  useI18n();
  const containerRef = useRef<HTMLDivElement>(null);
  const submitRef = useRef(onSubmit);
  submitRef.current = onSubmit;
  const nameRef = useRef("");
  const [name, setName] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);
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
        submit: (config) => {
          const trimmedName = nameRef.current.trim();
          if (new TextEncoder().encode(trimmedName).length > 256 || [...trimmedName].some(char => /[\u0000-\u001f\u007f-\u009f]/u.test(char))) {
            setNameError(t("Instance name must be at most 256 UTF-8 bytes and contain no control characters."));
            return Promise.reject(new Error("instance_name_invalid"));
          }
          setNameError(null);
          return submitRef.current(config, trimmedName || undefined);
        },
        invoke: async <T,>(operation: string, args?: unknown, options?: InvokeOptions) => {
          if (disposed) throw new Error("plugin_configuration_closed");
          // Configuration can need backend data before the first instance exists.
          // Runtime start is idempotent and completes the operation-catalog handshake.
          await hostAPI.startPlugin(pluginId);
          if (disposed) throw new Error("plugin_configuration_closed");
          return hostAPI.invokePluginOperation<T>(pluginId, operation, args ?? {}, options?.waitTimeoutMs);
        },
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
  nameRef.current = name;
  return <fieldset disabled={disabled} aria-busy={disabled} className="m-0 min-w-0 border-0 p-0">
    {error && <p role="alert" className="mb-3 text-xs text-destructive">{error}</p>}
    <label className="mb-4 block text-sm font-medium" htmlFor="new-instance-name">
      <span className="mb-1.5 block">{t("Instance name (optional)")}</span>
      <input id="new-instance-name" type="text" autoComplete="off" maxLength={256} value={name}
        onChange={event => { setName(event.currentTarget.value); setNameError(null); }}
        placeholder={t("Leave empty to use the plugin default")}
        aria-invalid={!!nameError}
        className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-normal text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring" />
    </label>
    {nameError && <p role="alert" className="mb-3 text-xs text-destructive">{nameError}</p>}
    <div ref={containerRef} />
  </fieldset>;
}
