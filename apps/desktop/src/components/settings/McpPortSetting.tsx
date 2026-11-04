import { useEffect, useRef, useState } from "react";
import { hostAPI, type McpStdioStatus } from "../../host-api";
import { useI18n } from "../../use-i18n";
import { Alert, AlertDescription } from "../ui/alert";
import { Button } from "../ui/button";
import { McpConfiguration } from "../mcp/McpConfiguration";

/** Device-local Bridge configuration; display preferences and plugin connections are independent. */
export function McpPortSetting() {
  const { t } = useI18n();
  const [status, setStatus] = useState<McpStdioStatus | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [connectionOpen, setConnectionOpen] = useState(false);
  const mounted = useRef(false);
  const saving = useRef(false);
  const loaded = useRef<McpStdioStatus | null>(null);

  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    let cancelled = false;
    setError(""); setSaved(false);
    void hostAPI.mcpStdioStatus().then(value => {
      if (cancelled) return;
      const previous = loaded.current;
      setStatus(value);
      setDraft(current => !previous || current === String(previous.port) ? String(value.port) : current);
      loaded.current = value;
    }).catch(reason => { if (!cancelled) setError(String(reason)); });
    return () => { cancelled = true; };
  }, [attempt]);

  const save = async () => {
    if (saving.current || !status) return;
    const port = Number(draft);
    if (!/^\d+$/.test(draft) || !Number.isInteger(port) || port < 1 || port > 65535) {
      setError("mcp_port_invalid"); return;
    }
    saving.current = true; setBusy(true); setError(""); setSaved(false);
    try {
      const value = await hostAPI.setMcpStdioPort(port);
      if (!mounted.current) return;
      loaded.current = value;
      setStatus(value); setDraft(String(value.port)); setSaved(true);
    } catch (reason) {
      if (mounted.current) setError(String(reason));
    } finally {
      saving.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  const problem = error || status?.configurationError || "";
  return <><form className="flex w-full max-w-64 flex-col gap-2" noValidate data-invalid={Boolean(problem)} data-disabled={busy || !status} onSubmit={event => { event.preventDefault(); void save(); }}>
    <div className="flex gap-2">
      <input id="mcp-port" type="number" inputMode="numeric" min="1" max="65535" step="1"
        className="h-9 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
        value={draft} disabled={busy || !status} aria-label={t("MCP port")} aria-invalid={Boolean(problem)}
        aria-describedby={`mcp-port-description mcp-port-help${problem ? " mcp-port-error" : ""}`}
        onChange={event => { setDraft(event.target.value); setError(""); setSaved(false); }} />
      <Button type="submit" variant="outline" disabled={busy || !status || (!status.configurationError && draft === String(status.port))}>
        {busy ? t("Saving…") : t("Save")}
      </Button>
    </div>
    <p id="mcp-port-help" className="text-xs leading-relaxed text-muted-foreground">{t("Saving changes the local bridge port immediately. Update the MCP client configuration and reconnect.")}</p>
    {status && <p role="status" className="text-xs text-muted-foreground">{saved
      ? t(status.enabled ? "Port saved. Copy the updated configuration from MCP connection and reconnect your client." : "Port saved. It will be used when MCP is enabled.")
      : status.enabled ? t("Local bridge: {address}", { address: status.address ?? "" }) : t("MCP is disabled.")}</p>}
    {problem && <Alert id="mcp-port-error" variant="destructive"><AlertDescription>{t(problem)}</AlertDescription></Alert>}
    {!status && error && <Button type="button" variant="outline" onClick={() => setAttempt(value => value + 1)}>{t("Retry")}</Button>}
    <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={() => setConnectionOpen(true)}>{t("MCP connection")}</Button>
  </form>
    {connectionOpen && <McpConfiguration onClose={() => { setConnectionOpen(false); setAttempt(value => value + 1); }} />}
  </>;
}
