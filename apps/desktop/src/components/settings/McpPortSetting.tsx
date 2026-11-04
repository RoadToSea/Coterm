import { useEffect, useRef, useState } from "react";
import { hostAPI } from "../../host-api";
import { useI18n } from "../../use-i18n";
import { Button } from "../ui/button";
import type { McpSettingsController } from "./use-mcp-settings";

export function McpPortSetting({ mcp }: { mcp: McpSettingsController }) {
  const { t } = useI18n();
  const { status, busy, error, perform, setError } = mcp;
  const [draft, setDraft] = useState("");
  const [saved, setSaved] = useState(false);
  const previousPort = useRef<number | null>(null);
  useEffect(() => {
    if (!status) return;
    const previous = previousPort.current;
    setDraft(current => previous === null || current === String(previous) ? String(status.port) : current);
    previousPort.current = status.port;
  }, [status]);

  const save = async () => {
    if (busy || !status) return;
    const port = Number(draft);
    if (!/^\d+$/.test(draft) || !Number.isInteger(port) || port < 1 || port > 65535) {
      setError("mcp_port_invalid"); return;
    }
    setSaved(false);
    const value = await perform(() => hostAPI.setMcpStdioPort(port));
    if (value) { setDraft(String(value.port)); setSaved(true); }
  };
  return <form className="flex w-64 max-w-full flex-col gap-2" noValidate data-invalid={Boolean(error)}
    data-disabled={busy || !status} onSubmit={event => { event.preventDefault(); void save(); }}>
    <div className="flex gap-2">
      <input id="mcp-port" type="number" inputMode="numeric" min="1" max="65535" step="1"
        className="h-9 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
        value={draft} disabled={busy || !status} aria-label={t("MCP port")} aria-invalid={Boolean(error)}
        aria-describedby={error ? "mcp-settings-error" : undefined}
        onChange={event => { setDraft(event.target.value); setError(""); setSaved(false); }} />
      <Button type="submit" variant="outline" disabled={busy || !status || (!status.configurationError && draft === String(status.port))}>
        {busy ? t("Saving…") : t("Save")}
      </Button>
    </div>
    {saved && <p role="status" className="text-xs text-muted-foreground">{t("Saved")}</p>}
  </form>;
}
