import { useEffect, useState } from "react";
import { hostAPI } from "../../host-api";
import { Modal } from "../ui/Modal";

export function McpConfiguration({ onClose }: { onClose: () => void }) {
  const [config, setConfig] = useState("");
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    let disposed = false;
    void hostAPI.mcpConfiguration().then((connection) => {
      if (!disposed) setConfig(connection.url);
    }).catch((reason) => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; };
  }, []);
  return <Modal title="MCP connection" onClose={onClose}>
    <p className="mb-3 text-xs text-[#aaaaaa]">Add this URL to your Agent as a Streamable HTTP MCP server. No token is required. Keep Coterm open; the address stays the same after restarting.</p>
    {error && <p role="alert">{error}</p>}
    {config && <div className="flex gap-2">
      <input aria-label="MCP connection URL" readOnly value={config} spellCheck={false} className="min-w-0 flex-1 border border-[#454545] bg-[#1e1e1e] p-3 font-mono text-xs" onFocus={(event) => event.target.select()} />
      <button type="button" disabled={!config} onClick={() => {
        void navigator.clipboard.writeText(config).then(() => setCopied(true)).catch(() => setError("Could not copy. Select the address and copy it manually."));
      }} className="border border-[#454545] px-3 text-xs disabled:opacity-50">{copied ? "Copied" : "Copy URL"}</button>
    </div>}
  </Modal>;
}
