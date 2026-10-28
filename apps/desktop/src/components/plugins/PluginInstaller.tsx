import { useEffect, useRef, useState } from "react";
import { hostAPI, type PluginInstallPreview } from "../../host-api";
import { useI18n } from "../../use-i18n";
import { Modal } from "../ui/Modal";

export function PluginInstaller({ onClose, onInstalled }: { onClose(): void; onInstalled(): Promise<void> }) {
  const { locale, t } = useI18n();
  const [preview, setPreview] = useState<PluginInstallPreview | null>(null);
  const [busy, setBusy] = useState<"select" | "install" | null>(null);
  const [error, setError] = useState("");
  const [installed, setInstalled] = useState(false);
  const pendingToken = useRef<string | null>(null);
  const working = useRef(false);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (pendingToken.current) void hostAPI.discardPluginInstall(pendingToken.current).catch(() => {});
    };
  }, []);

  const failure = (reason: unknown) => {
    const code = typeof reason === "object" && reason !== null && "code" in reason ? String(reason.code) : String(reason);
    setError(code);
  };

  const choose = async () => {
    if (working.current) return;
    working.current = true;
    setBusy("select");
    setError("");
    setInstalled(false);
    setPreview(null);
    pendingToken.current = null;
    try {
      const next = await hostAPI.preparePluginInstall(locale);
      if (!mounted.current) {
        if (next) await hostAPI.discardPluginInstall(next.token);
        return;
      }
      pendingToken.current = next?.token ?? null;
      setPreview(next);
    } catch (reason) { if (mounted.current) failure(reason); }
    finally { working.current = false; if (mounted.current) setBusy(null); }
  };

  const install = async () => {
    if (!preview || working.current) return;
    working.current = true;
    setBusy("install");
    setError("");
    try {
      await hostAPI.installPluginPackage(preview.token);
      pendingToken.current = null;
      if (mounted.current) { setInstalled(true); setPreview(null); }
      await onInstalled();
    } catch (reason) { if (mounted.current) failure(reason); }
    finally {
      pendingToken.current = null;
      working.current = false;
      if (mounted.current) { setPreview(null); setBusy(null); }
    }
  };

  return (
    <Modal title={t("Install offline plugin")} onClose={() => { if (!working.current) onClose(); }}>
      <div aria-busy={busy !== null} className="space-y-4 text-xs">
        <p className="text-coterm-text-muted">{t("Choose a .coterm or .zip package built for this platform.")}</p>
        <p className="border-l-2 border-[#d7ba7d] bg-[#3a3224] px-3 py-2 leading-relaxed text-[#e8d5a9]">
          {t("Plugins run native code with your user permissions. Only install packages from authors you trust.")}
        </p>
        {error && <p role="alert" className="break-words text-[#f48771]">{t(error)}</p>}
        {installed && <p role="status" className="text-[#89d185]">{t("Plugin installed. Select it on the left to create an instance.")}</p>}
        {preview && <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-2 border-y border-coterm-border py-3">
          <dt>{t("Package")}</dt><dd className="break-all">{preview.name} ({preview.id})</dd>
          <dt>{t("Version")}</dt><dd>{preview.version}</dd>
          <dt>{t("Platform")}</dt><dd>{preview.platform}</dd>
        </dl>}
        <div className="flex flex-wrap justify-end gap-2">
          <button type="button" disabled={busy !== null} onClick={() => void choose()}
            className="h-8 border border-coterm-border-input px-3 hover:bg-coterm-surface-hover disabled:opacity-50">
            {t(busy === "select" ? "Selecting and validating…" : preview ? "Choose another package" : "Choose plugin package")}
          </button>
          {preview && <button type="button" disabled={busy !== null} onClick={() => void install()}
            className="h-8 bg-[#0e639c] px-3 text-white hover:bg-[#1177bb] disabled:opacity-50">
            {t(busy === "install" ? "Installing…" : "Install trusted package")}
          </button>}
        </div>
      </div>
    </Modal>
  );
}
