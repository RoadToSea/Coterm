import { useCallback, useEffect, useRef, useState } from "react";
import { hostAPI, type McpStatus } from "../../host-api";

/** One backend snapshot and mutation lock for every MCP control in Settings. */
export function useMcpSettings() {
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const mounted = useRef(false);
  const pending = useRef(false);

  const perform = useCallback(async (action: () => Promise<McpStatus>) => {
    if (pending.current) return null;
    pending.current = true;
    setBusy(true); setError("");
    try {
      const value = await action();
      if (!mounted.current) return null;
      setStatus(value);
      return value;
    } catch (reason) {
      if (mounted.current) setError(String(reason));
      return null;
    } finally {
      pending.current = false;
      if (mounted.current) setBusy(false);
    }
  }, []);
  const refresh = useCallback(() => perform(() => hostAPI.mcpStatus()), [perform]);
  useEffect(() => {
    mounted.current = true;
    void refresh();
    return () => { mounted.current = false; };
  }, [refresh]);

  return { status, error, busy, perform, refresh, setError };
}

export type McpSettingsController = ReturnType<typeof useMcpSettings>;
