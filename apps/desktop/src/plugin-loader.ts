import { hostAPI } from "./host-api";
import type { PluginFrontendModule } from "@coterm/plugin-sdk";

const modules = new Map<string, Promise<PluginFrontendModule | null>>();

/** Trusted MVP modules are single ES bundles with no relative imports or assets. */
export function loadPluginFrontend(pluginId: string): Promise<PluginFrontendModule | null> {
  const existing = modules.get(pluginId);
  if (existing) return existing;
  const loading = (async () => {
    const source = await hostAPI.frontendSource(pluginId);
    if (source === null) return null;
    const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
    try {
      const module: unknown = await import(/* @vite-ignore */ url);
      if (typeof module !== "object" || module === null || !("mount" in module) || typeof module.mount !== "function") {
        throw new Error("Plugin frontend must export mount(container, api)");
      }
      if ("configure" in module && module.configure !== undefined && typeof module.configure !== "function") {
        throw new Error("Plugin configure export must be a function");
      }
      return module as PluginFrontendModule;
    } finally {
      URL.revokeObjectURL(url);
    }
  })();
  modules.set(pluginId, loading);
  void loading.catch(() => modules.delete(pluginId));
  return loading;
}
