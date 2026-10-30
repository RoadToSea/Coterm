import { useEffect, useRef, useState } from "react";
import { hostAPI, type PluginInstallPreview } from "../../host-api";
import { useI18n } from "../../use-i18n";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/button";
import { Alert, AlertDescription } from "../ui/alert";

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
      <div aria-busy={busy !== null} className="flex flex-col gap-4 text-xs">
        <p className="text-coterm-text-muted">{t("Choose a .coterm or .zip package built for this platform.")}</p>
        <Alert variant="warning" role="note"><AlertDescription>
          {t("Plugins run native code with your user permissions. Only install packages from authors you trust.")}
        </AlertDescription></Alert>
        {error && <p role="alert" className="break-words text-destructive">{t(error)}</p>}
        {installed && <p role="status" className="text-success">{t("Plugin installed. Select it on the left to create an instance.")}</p>}
        {preview && <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-2 border-y border-coterm-border py-3">
          <dt>{t("Package")}</dt><dd className="break-all">{preview.name} ({preview.id})</dd>
          <dt>{t("Version")}</dt><dd>{preview.version}</dd>
          <dt>{t("Platform")}</dt><dd>{preview.platform}</dd>
        </dl>}
        <div className="flex flex-wrap justify-end gap-2">
          <Button variant="outline" size="sm" type="button" disabled={busy !== null} onClick={() => void choose()}>
            {t(busy === "select" ? "Selecting and validating…" : preview ? "Choose another package" : "Choose plugin package")}
          </Button>
          {preview && <Button size="sm" type="button" disabled={busy !== null} onClick={() => void install()}>
            {t(busy === "install" ? "Installing…" : "Install trusted package")}
          </Button>}
        </div>
      </div>
    </Modal>
  );
}
