import { hostAPI } from "./host-api";
import type { DriverFrontendModule } from "../../shared/index";

const modules = new Map<string, Promise<DriverFrontendModule | null>>();

/** Trusted MVP modules are single ES bundles with no relative imports or assets. */
export function loadDriverFrontend(driverId: string): Promise<DriverFrontendModule | null> {
  const existing = modules.get(driverId);
  if (existing) return existing;
  const loading = (async () => {
    const source = await hostAPI.frontendSource(driverId);
    if (source === null) return null;
    const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
    try {
      const module: unknown = await import(/* @vite-ignore */ url);
      if (typeof module !== "object" || module === null || !("mount" in module) || typeof module.mount !== "function") {
        throw new Error("Driver frontend must export mount(container, api)");
      }
      if ("configure" in module && module.configure !== undefined && typeof module.configure !== "function") {
        throw new Error("Driver configure export must be a function");
      }
      return module as DriverFrontendModule;
    } finally {
      URL.revokeObjectURL(url);
    }
  })();
  modules.set(driverId, loading);
  void loading.catch(() => modules.delete(driverId));
  return loading;
}
