import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useCallback, useEffect, useState } from "react";
import { hostAPI, type McpStdioStatus } from "../../host-api";
import { Modal } from "../ui/Modal";

/**
 * MCP 面板。对外只有 Stdio：桌面可执行文件的 `mcp stdio` 子命令把标准输入输出
 * 转发到固定端口的私有回环桥，桥只在用户启用期间存在。面板不展示任何 HTTP 地址。
 */
export function McpConfiguration({ onClose }: { onClose: () => void }) {
  useI18n();
  const [status, setStatus] = useState<McpStdioStatus | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await hostAPI.mcpStdioStatus());
      setError("");
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const toggle = async () => {
    setBusy(true);
    // 停用会关闭桥，旧的「已复制」提示不能继续显示。
    setCopied(false);
    try {
      setStatus(await hostAPI.setMcpStdioEnabled(!status?.enabled));
      setError("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  if (status?.command === null) {
    return (
      <Modal title={t("MCP connection")} onClose={onClose}>
        {error && <p role="alert">{error}</p>}
        <p className="text-xs text-coterm-text-muted">
          {t("This build cannot start the local Stdio adapter because its executable path is unavailable.")}
        </p>
      </Modal>
    );
  }

  return (
    <Modal title={t("MCP connection")} onClose={onClose}>
      <p className="mb-3 text-xs text-coterm-text-subtle">
        {t("Agents connect through the adapter below. Keep Coterm running; the adapter routes to this desktop and never starts plugins on its own.")}
      </p>
      {error && <p role="alert">{error}</p>}
      <div className="mb-2 flex items-center justify-between gap-3">
        <h3 className="text-xs font-medium">{t("Stdio adapter")}</h3>
        <button
          type="button"
          role="switch"
          aria-checked={Boolean(status?.enabled)}
          disabled={busy}
          onClick={() => void toggle()}
          className="border border-coterm-border-input px-2 py-1 text-[11px] disabled:opacity-50"
        >
          {status?.enabled ? t("Disable") : t("Enable")}
        </button>
      </div>
      <p className="mb-2 text-[11px] text-coterm-text-muted">
        {t("Enabling opens the local loopback bridge on a fixed port. Disabling, or closing Coterm, stops it.")}
      </p>
      {status?.clientConfig && (
        <>
          <p className="mb-1 text-[11px] text-coterm-text-muted">
            {t("Paste this into the agent's MCP settings. Host-side Agent approval still applies to write operations.")}
          </p>
          <textarea
            aria-label={t("Stdio client configuration")}
            readOnly
            rows={10}
            value={status.clientConfig}
            spellCheck={false}
            className="w-full border border-coterm-border-input bg-coterm-surface p-3 font-mono text-xs"
            onFocus={(event) => event.target.select()}
          />
          <button
            type="button"
            onClick={() => {
              void navigator.clipboard
                .writeText(status.clientConfig ?? "")
                .then(() => setCopied(true))
                .catch(() =>
                  setError(t("Could not copy. Select the configuration and copy it manually.")),
                );
            }}
            className="mt-2 border border-coterm-border-input px-2 py-1 text-[11px]"
          >
            {copied ? t("Copied") : t("Copy configuration")}
          </button>
        </>
      )}
    </Modal>
  );
}
