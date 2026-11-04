import { useState } from "react";
import type { McpStdioStatus } from "../../host-api";
import { useI18n } from "../../use-i18n";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/button";
import { Alert, AlertDescription } from "../ui/alert";

/** Client configuration only; startup and manual controls live in the Settings row. */
export function McpConfiguration({ status, onClose }: { status: McpStdioStatus; onClose: () => void }) {
  const { t } = useI18n();
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  return <Modal title={t("MCP connection")} onClose={onClose}>
    {error && <Alert variant="destructive"><AlertDescription>{error}</AlertDescription></Alert>}
    {status.command === null ? <p className="text-xs text-muted-foreground">
      {t("This build cannot start the local Stdio adapter because its executable path is unavailable.")}
    </p> : status.clientConfig ? <>
      <p className="mb-3 text-xs text-muted-foreground">
        {t("Paste this into the agent's MCP settings. Host-side Agent approval still applies to write operations.")}
      </p>
      <textarea aria-label={t("Stdio client configuration")} readOnly rows={10} value={status.clientConfig}
        spellCheck={false} className="w-full rounded-md border border-input bg-background p-3 font-mono text-xs"
        onFocus={event => event.target.select()} />
      <Button type="button" variant="outline" size="sm" className="mt-2" onClick={() => {
        void navigator.clipboard.writeText(status.clientConfig ?? "").then(() => { setError(""); setCopied(true); })
          .catch(() => setError(t("Could not copy. Select the configuration and copy it manually.")));
      }}>{t(copied ? "Copied" : "Copy configuration")}</Button>
    </> : <p className="text-xs text-muted-foreground">{t("Start MCP to view the client configuration.")}</p>}
  </Modal>;
}
