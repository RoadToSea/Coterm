import { useState } from "react";
import { hostAPI } from "../../host-api";
import { useI18n } from "../../use-i18n";
import { Button } from "../ui/button";
import { ToggleGroup, ToggleGroupItem } from "../ui/toggle-group";
import { McpConfiguration } from "../mcp/McpConfiguration";
import type { McpSettingsController } from "./use-mcp-settings";

export function McpConnectionSetting({ mcp }: { mcp: McpSettingsController }) {
  const { t } = useI18n();
  const [configurationOpen, setConfigurationOpen] = useState(false);
  const { status, busy, perform } = mcp;
  return <div className="flex w-64 max-w-full flex-col gap-2">
    <ToggleGroup id="mcp-connection" type="single" value={status?.startupMode ?? ""}
      disabled={busy || !status} aria-labelledby="mcp-connection-label"
      className="grid w-full grid-cols-2 gap-0 rounded-full border border-input bg-muted p-1"
      onValueChange={value => {
        if (value === "auto" || value === "manual") void perform(() => hostAPI.setMcpStartupMode(value));
      }}>
      <ToggleGroupItem value="auto" className="min-w-0 rounded-full">{t("Auto start")}</ToggleGroupItem>
      <ToggleGroupItem value="manual" className="min-w-0 rounded-full">{t("Manual start")}</ToggleGroupItem>
    </ToggleGroup>
    {status?.startupMode === "manual" && <Button id="mcp-toggle" type="button" variant={status.enabled ? "outline" : "default"}
      disabled={busy} onClick={() => void perform(() => hostAPI.setMcpStdioEnabled(!status.enabled))}>
      {t(status.enabled ? "Stop MCP" : "Start MCP")}
    </Button>}
    <Button type="button" data-mcp-config variant="ghost" size="sm" disabled={busy || !status}
      onClick={() => setConfigurationOpen(true)}>{t("Client configuration")}</Button>
    {configurationOpen && status && <McpConfiguration status={status} onClose={() => setConfigurationOpen(false)} />}
  </div>;
}
