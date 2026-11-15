import { useState } from "react";
import type { McpStatus } from "../../host-api";
import { useI18n } from "../../use-i18n";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/button";
import { Textarea } from "../ui/input";
import { Alert, AlertDescription } from "../ui/alert";

/** Client configuration only; startup and manual controls live in the Settings row. */
export function McpConfiguration({ status, onClose }: { status: McpStatus; onClose: () => void }) {
  const { t } = useI18n();
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  return <Modal title={t("MCP connection")} onClose={onClose}>
    {error && <Alert variant="destructive"><AlertDescription>{error}</AlertDescription></Alert>}
    {status.clientConfig ? <>
      <Textarea aria-label={t("HTTP client configuration")} readOnly rows={10} value={status.clientConfig}
        title={t("Paste this into the agent's MCP settings. Host-side Agent approval still applies to write operations.")}
        aria-description={t("Paste this into the agent's MCP settings. Host-side Agent approval still applies to write operations.")}
        spellCheck={false} className="p-3 text-xs"
        onFocus={event => event.target.select()} />
      <Button type="button" variant="outline" size="sm" className="mt-2" onClick={() => {
        void navigator.clipboard.writeText(status.clientConfig ?? "").then(() => { setError(""); setCopied(true); })
          .catch(() => setError(t("Could not copy. Select the configuration and copy it manually.")));
      }}>{t(copied ? "Copied" : "Copy configuration")}</Button>
    </> : <p className="text-xs text-muted-foreground">{t("Start MCP to view the client configuration.")}</p>}
  </Modal>;
}
