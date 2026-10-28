import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useCallback, useEffect, useState } from "react";
import { hostAPI, type McpSessionInfo } from "../../host-api";
import { Modal } from "../ui/Modal";

/** Formats an age in seconds without pulling in a date library. */
function age(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

export function McpConfiguration({ onClose }: { onClose: () => void }) {
  useI18n();
  const [config, setConfig] = useState("");
  const [sessions, setSessions] = useState<McpSessionInfo[]>([]);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [connection, live] = await Promise.all([
        hostAPI.mcpConfiguration(),
        hostAPI.mcpSessions(),
      ]);
      setConfig(connection.url);
      setSessions(live);
      setError("");
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    void refresh();
    const timer = window.setInterval(() => {
      if (!disposed) void refresh();
    }, 2000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [refresh]);

  const close = async (prefix: string) => {
    setBusy(true);
    try {
      await hostAPI.closeMcpSession(prefix);
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title={t("MCP connection")} onClose={onClose}>
      <p className="mb-3 text-xs text-[#aaaaaa]">
        {t("Add this URL to your Agent as a Streamable HTTP MCP server. No token is required. Keep Coterm open; the address stays the same after restarting.")}
      </p>
      {error && <p role="alert">{error}</p>}
      {config && (
        <div className="flex gap-2">
          <input
            aria-label={t("MCP connection URL")}
            readOnly
            value={config}
            spellCheck={false}
            className="min-w-0 flex-1 border border-[#454545] bg-[#1e1e1e] p-3 font-mono text-xs"
            onFocus={(event) => event.target.select()}
          />
          <button
            type="button"
            disabled={!config}
            onClick={() => {
              void navigator.clipboard
                .writeText(config)
                .then(() => setCopied(true))
                .catch(() =>
                  setError(t("Could not copy. Select the address and copy it manually.")),
                );
            }}
            className="border border-[#454545] px-3 text-xs disabled:opacity-50"
          >
            {copied ? t("Copied") : t("Copy URL")}
          </button>
        </div>
      )}
      <section className="mt-5">
        <div className="mb-2 flex items-center justify-between">
          <h3 className="text-xs font-medium">
            {t("Active sessions ({count}/8)", { count: sessions.length })}
          </h3>
          <button
            type="button"
            onClick={() => void refresh()}
            className="border border-[#454545] px-2 py-1 text-[11px]"
          >
            {t("Refresh")}
          </button>
        </div>
        <p className="mb-2 text-[11px] text-[#888888]">
          {t("When a new client reports “too many MCP sessions”, close an unused session here. Idle sessions also expire automatically after 30 minutes.")}
        </p>
        {sessions.length === 0 ? (
          <p className="text-[11px] text-[#888888]">{t("No client is connected.")}</p>
        ) : (
          <ul className="divide-y divide-[#383838] border border-[#383838]">
            {sessions.map((session) => (
              <li key={session.idPrefix} className="flex items-center gap-3 px-3 py-2">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-xs">
                    {session.clientName ?? t("Unknown client")}
                    {session.clientVersion ? ` ${session.clientVersion}` : ""}
                  </div>
                  <div className="text-[11px] text-[#888888]">
                    {session.idPrefix} · {session.protocolVersion} {t("· idle")} {age(session.idleSeconds)}
                    {session.ready ? "" : t(" · initializing")}
                  </div>
                </div>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void close(session.idPrefix)}
                  className="border border-[#454545] px-2 py-1 text-[11px] disabled:opacity-50"
                >
                  {t("Close")}
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </Modal>
  );
}
