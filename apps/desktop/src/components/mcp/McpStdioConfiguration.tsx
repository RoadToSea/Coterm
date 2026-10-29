import { useState } from "react";
import { t } from "../../i18n";
import { hostAPI, type McpStdioStatus } from "../../host-api";

/**
 * The Stdio adapter section. It never reuses the Streamable HTTP URL: the adapter
 * speaks to a separate token-protected loopback bridge that only exists while the
 * user has enabled it during this run.
 */
export function McpStdioConfiguration({
  status,
  onStatus,
  onError,
}: {
  status: McpStdioStatus | null;
  onStatus: (status: McpStdioStatus) => void;
  onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  const toggle = async () => {
    setBusy(true);
    // A disable revokes the token, so a stale "Copied" label would be misleading.
    setCopied(false);
    try {
      onStatus(await hostAPI.setMcpStdioEnabled(!status?.enabled));
    } catch (reason) {
      onError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  if (status?.command === null) {
    return (
      <section className="mt-5 border-t border-[#383838] pt-4">
        <h3 className="mb-2 text-xs font-medium">{t("Stdio adapter")}</h3>
        <p className="text-[11px] text-[#888888]">
          {t("This build does not include the local Stdio adapter. Use the Streamable HTTP URL above instead.")}
        </p>
      </section>
    );
  }

  return (
    <section className="mt-5 border-t border-[#383838] pt-4">
      <div className="mb-2 flex items-center justify-between gap-3">
        <h3 className="text-xs font-medium">{t("Stdio adapter")}</h3>
        <button
          type="button"
          role="switch"
          aria-checked={Boolean(status?.enabled)}
          disabled={busy}
          onClick={() => void toggle()}
          className="border border-[#454545] px-2 py-1 text-[11px] disabled:opacity-50"
        >
          {status?.enabled ? t("Disable") : t("Enable")}
        </button>
      </div>
      <p className="mb-2 text-[11px] text-[#888888]">
        {t("For agents that only support Stdio. Enabling opens a private loopback bridge protected by a token issued at startup. Disabling, or closing Coterm, revokes it.")}
      </p>
      {status?.clientConfig && (
        <>
          <p className="mb-1 text-[11px] text-[#888888]">
            {t("Paste this into the agent's MCP settings. The token below is a secret: do not share the configuration.")}
          </p>
          <textarea
            aria-label={t("Stdio client configuration")}
            readOnly
            rows={10}
            value={status.clientConfig}
            spellCheck={false}
            className="w-full border border-[#454545] bg-[#1e1e1e] p-3 font-mono text-xs"
            onFocus={(event) => event.target.select()}
          />
          <button
            type="button"
            onClick={() => {
              void navigator.clipboard
                .writeText(status.clientConfig ?? "")
                .then(() => setCopied(true))
                .catch(() =>
                  onError(t("Could not copy. Select the configuration and copy it manually.")),
                );
            }}
            className="mt-2 border border-[#454545] px-2 py-1 text-[11px]"
          >
            {copied ? t("Copied") : t("Copy configuration")}
          </button>
        </>
      )}
    </section>
  );
}
