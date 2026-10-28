import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useCallback, useEffect, useMemo, useState } from "react";
import { hostAPI, type PluginSummary } from "../../host-api";
import { Modal } from "../ui/Modal";

/**
 * Agent 审批放行。基座只比对操作名：放行跳过审批，但不改变 effect、不绕过插件
 * 校验、不允许 Agent 自批。默认只放行基座内置的只读依赖查询。
 */
export function ApprovalSettings({ plugins, onClose }: { plugins: PluginSummary[]; onClose: () => void }) {
  useI18n();
  const [allowed, setAllowed] = useState<string[]>([]);
  const [operations, setOperations] = useState<string[]>([]);
  const [custom, setCustom] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [current, catalogs] = await Promise.all([
        hostAPI.getAllowedOperations(),
        Promise.all(
          plugins.filter((plugin) => plugin.running).map(async (plugin) => ({
            plugin,
            operations: await hostAPI.listOperations(plugin.id),
          })),
        ),
      ]);
      setAllowed(current);
      const names = catalogs.flatMap(({ plugin, operations }) =>
        operations
          .filter((operation) => operation.effect === "commit" || operation.effect === "read")
          .map((operation) => `${plugin.id} :: ${operation.name}`),
      );
      setOperations([...new Set(names)].sort());
    } catch (reason) {
      setError(String(reason));
    }
  }, [plugins]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const save = async (next: string[]) => {
    setBusy(true);
    try {
      const stored = await hostAPI.setAllowedOperations(next);
      setAllowed(stored);
      setError("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (name: string) => {
    const next = allowed.includes(name)
      ? allowed.filter((entry) => entry !== name)
      : [...allowed, name];
    void save(next);
  };

  const suggestions = useMemo(
    () => operations.filter((operation) => !allowed.includes(operation.split(" :: ")[1])),
    [operations, allowed],
  );

  return (
    <Modal title={t("Agent approval")} onClose={onClose}>
      <p className="mb-3 text-xs text-[#aaaaaa]">
        {t("An Agent operation normally waits for human approval. Adding an operation here lets that operation skip the prompt. Approval is only the host-side gate: plugin checks, secret input and protocol authentication still apply, and an Agent can never approve its own proposal.")}
      </p>
      <p className="mb-4 text-[11px] text-[#888888]">
        {t("Read-only operations are not listed: they are already exempt because they cannot change state.")}
      </p>
      {error && <p role="alert">{error}</p>}
      <section>
        <h3 className="mb-2 text-xs font-medium">{t("Allowed operations ({count})", { count: allowed.length })}</h3>
        {allowed.length === 0 ? (
          <p className="text-[11px] text-[#888888]">{t("No operation is allowed yet.")}</p>
        ) : (
          <ul className="mb-4 divide-y divide-[#383838] border border-[#383838]">
            {allowed.map((name) => (
              <li key={name} className="flex items-center gap-3 px-3 py-2">
                <code className="min-w-0 flex-1 truncate text-[11px]">{name}</code>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => toggle(name)}
                  className="border border-[#454545] px-2 py-1 text-[11px] disabled:opacity-50"
                >
                  {t("Remove")}
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
      <section>
        <h3 className="mb-2 text-xs font-medium">{t("Add an operation")}</h3>
        <div className="mb-3 flex gap-2">
          <input
            aria-label={t("Operation name")}
            value={custom}
            spellCheck={false}
            placeholder="plugin.operation"
            onChange={(event) => setCustom(event.target.value)}
            className="min-w-0 flex-1 border border-[#454545] bg-[#1e1e1e] p-2 font-mono text-[11px]"
          />
          <button
            type="button"
            disabled={busy || custom.trim() === ""}
            onClick={() => {
              void save([...allowed, custom.trim()]);
              setCustom("");
            }}
            className="border border-[#454545] px-3 text-[11px] disabled:opacity-50"
          >
            {t("Allow")}
          </button>
        </div>
        {suggestions.length > 0 && (
          <>
            <p className="mb-2 text-[11px] text-[#888888]">{t("From running plugins:")}</p>
            <ul className="max-h-48 overflow-auto border border-[#383838]">
              {suggestions.map((entry) => {
                const [pluginId, name] = entry.split(" :: ");
                return (
                  <li key={entry} className="flex items-center gap-2 px-3 py-1.5">
                    <span className="min-w-0 flex-1 truncate text-[11px]">
                      <span className="text-[#888888]">{pluginId}</span> {name}
                    </span>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => toggle(name)}
                      className="border border-[#454545] px-2 py-0.5 text-[11px] disabled:opacity-50"
                    >
                      {t("Allow")}
                    </button>
                  </li>
                );
              })}
            </ul>
          </>
        )}
      </section>
    </Modal>
  );
}
