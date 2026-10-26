import { useCallback, useEffect, useMemo, useState } from "react";
import { Box, ChevronDown, CircleStop, MoreHorizontal, PanelLeftClose, PanelLeftOpen, Plus, PlugZap, ShieldAlert, ShieldCheck, SlidersHorizontal, Trash2 } from "lucide-react";
import { HostInteractions } from "./components/host/HostInteractions";
import { HostApprovals } from "./components/host/HostApprovals";
import { McpConfiguration } from "./components/mcp/McpConfiguration";
import { Modal } from "./components/ui/Modal";
import { PluginWorkspace } from "./components/plugins/PluginWorkspace";
import { PluginConfiguration } from "./components/plugins/PluginConfiguration";
import {
  hostAPI,
  type InstanceInfo,
  type PluginLoadIssue,
  type PluginSummary,
  type SavedWorkspace,
} from "./host-api";
import { promptForConfirmation } from "./secret-input";
import type { OperationDescriptor, OperationRecord, OperationReceipt } from "@coterm/plugin-sdk";

export default function App() {
  const [instancesCollapsed, setInstancesCollapsed] = useState(() => {
    if (typeof window !== "undefined" && typeof window.innerWidth === "number") {
      return window.innerWidth < 900;
    }
    return false;
  });

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mediaQuery = window.matchMedia("(max-width: 899px)");
    const handleMediaChange = (event: MediaQueryListEvent | MediaQueryList) => {
      setInstancesCollapsed(event.matches);
    };
    if (typeof mediaQuery.addEventListener === "function") {
      mediaQuery.addEventListener("change", handleMediaChange);
      return () => mediaQuery.removeEventListener("change", handleMediaChange);
    } else if (typeof (mediaQuery as { addListener?: (cb: (e: MediaQueryListEvent) => void) => void }).addListener === "function") {
      (mediaQuery as unknown as { addListener: (cb: (e: MediaQueryListEvent) => void) => void }).addListener(handleMediaChange);
      return () => (mediaQuery as unknown as { removeListener: (cb: (e: MediaQueryListEvent) => void) => void }).removeListener(handleMediaChange);
    }
  }, []);

  const [mcpOpen, setMcpOpen] = useState(false);
  const [plugins, setPlugins] = useState<PluginSummary[]>([]);
  const [pluginIssues, setPluginIssues] = useState<PluginLoadIssue[]>([]);
  const [instances, setInstances] = useState<InstanceInfo[]>([]);
  const [activeInstanceId, setActiveInstanceId] = useState<string | null>(null);
  const [selectedPluginId, setSelectedPluginId] = useState<string | null>(null);
  const [instanceToolsOpen, setInstanceToolsOpen] = useState(false);
  const [configText, setConfigText] = useState("{}");
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [bypass, setBypass] = useState(false);
  const [operations, setOperations] = useState<OperationDescriptor[]>([]);
  const [pluginOperations, setPluginOperations] = useState<OperationDescriptor[]>([]);
  const [operationHistory, setOperationHistory] = useState<OperationRecord[]>([]);
  const [selectedOperation, setSelectedOperation] = useState("");
  const [selectedPluginOperation, setSelectedPluginOperation] = useState("");
  const [operationArgs, setOperationArgs] = useState("{}");
  const [operationResult, setOperationResult] = useState<OperationReceipt | null>(null);
  const [workspace, setWorkspace] = useState<SavedWorkspace | null>(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const formatError = (reason: unknown) => typeof reason === "object" && reason !== null && "code" in reason
    ? String((reason as { code: unknown }).code) : String(reason);

  const refreshWorkspace = useCallback(async () => {
    try {
      const saved = await hostAPI.workspace();
      setWorkspace(saved);
      setWorkspaceError(null);
      return saved;
    } catch (reason) {
      setWorkspaceError(formatError(reason));
      return null;
    }
  }, []);

  useEffect(() => {
    void refreshWorkspace().then((saved) => {
      if (saved) setInstancesCollapsed(saved.layout.instancesCollapsed);
    });
  }, [refreshWorkspace]);


  const activeInstance = useMemo(
    () => instances.find((instance) => instance.pluginInstanceId === activeInstanceId) ?? null,
    [instances, activeInstanceId],
  );

  const orderedInstances = useMemo(() => [...instances].sort((left, right) => {
    const rank = (id: string) => {
      const index = workspace?.entries.findIndex((entry) => entry.instanceId === id) ?? -1;
      return index < 0 ? Number.MAX_SAFE_INTEGER : index;
    };
    return rank(left.pluginInstanceId) - rank(right.pluginInstanceId);
  }), [instances, workspace]);

  useEffect(() => {
    if (!workspace || workspaceError) return;
    const timer = window.setTimeout(() => {
      void hostAPI.saveLayout(activeInstanceId, instancesCollapsed, orderedInstances.map((item) => item.pluginInstanceId))
        .catch((reason) => setWorkspaceError(formatError(reason)));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [activeInstanceId, instancesCollapsed, orderedInstances, workspace, workspaceError]);

  const selectedPlugin = plugins.find((plugin) => plugin.id === selectedPluginId) ?? null;
  const selectedOperationDescriptor = operations.find((item) => item.name === selectedOperation) ?? null;

  const updateOperationArgument = (name: string, value: unknown) => {
    let current: Record<string, unknown> = {};
    try {
      const parsed: unknown = JSON.parse(operationArgs);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        current = parsed as Record<string, unknown>;
      }
    } catch {
      current = {};
    }
    setOperationArgs(JSON.stringify({ ...current, [name]: value }, null, 2));
  };

  const operationSchema = selectedOperationDescriptor?.inputSchema;
  const schemaProperties = operationSchema && typeof operationSchema === "object"
    ? (operationSchema as { properties?: Record<string, { type?: string; title?: string; description?: string; enum?: unknown[]; minimum?: number; maximum?: number }> }).properties
    : undefined;
  const schemaRequired = operationSchema && typeof operationSchema === "object"
    ? (operationSchema as { required?: string[] }).required ?? []
    : [];

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [catalog, nextInstances, nextBypass] = await Promise.all([
        hostAPI.listPlugins(),
        hostAPI.listInstances(),
        hostAPI.getBypass(),
      ]);
      // 坏包只进诊断列表，有效插件照常列出。
      setPlugins(catalog.plugins);
      setPluginIssues(catalog.issues);
      setInstances(nextInstances);
      setBypass(nextBypass);
      setActiveInstanceId((current) =>
        nextInstances.some((instance) => instance.pluginInstanceId === current)
          ? current
          : nextInstances[0]?.pluginInstanceId ?? null,
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // A failed process cannot leave a mounted frontend presenting stale success.
  useEffect(() => {
    let disposed = false;
    let pending = false;
    const timer = window.setInterval(() => {
      if (pending) return;
      pending = true;
      void hostAPI.runtimeStatus().then(({ instances: next, runningPlugins }) => {
        if (disposed) return;
        setInstances((current) => JSON.stringify(current) === JSON.stringify(next) ? current : next);
        setPlugins((current) => current.some((plugin) => plugin.running !== runningPlugins.includes(plugin.id))
          ? current.map((plugin) => ({ ...plugin, running: runningPlugins.includes(plugin.id) })) : current);
      }).catch((reason) => { if (!disposed) setError(formatError(reason)); })
        .finally(() => { pending = false; });
    }, 1000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, []);

  const restoreSavedInstance = async (configRef: string) => {
    setBusyId(configRef);
    try {
      const instance = await hostAPI.restoreInstance(configRef);
      await refresh();
      await refreshWorkspace();
      setActiveInstanceId(instance.pluginInstanceId);
    } catch (reason) { setError(formatError(reason)); }
    finally { setBusyId(null); }
  };

  const restoreWorkspace = async () => {
    if (!workspace) return;
    setBusyId("restore-workspace");
    setError(null);
    const failures: string[] = [];
    let active: string | null = null;
    for (const entry of workspace.entries) {
      if (instances.some((item) => item.pluginInstanceId === entry.instanceId)) continue;
      try {
        const instance = await hostAPI.restoreInstance(entry.configRef);
        if (!active || entry.configRef === workspace.layout.activeConfigRef) active = instance.pluginInstanceId;
      } catch (reason) { failures.push(`${entry.label}: ${formatError(reason)}`); }
    }
    await refresh();
    await refreshWorkspace();
    if (active) setActiveInstanceId(active);
    if (failures.length) setError(failures.join("; "));
    setBusyId(null);
  };

  useEffect(() => {
    if (!activeInstance) {
      setOperations([]);
      setOperationHistory([]);
      setSelectedOperation("");
      return;
    }
    let current = true;
    Promise.all([
      hostAPI.listOperations(activeInstance.pluginId),
      hostAPI.recentOperations(activeInstance.pluginInstanceId, 20),
    ])
      .then(([catalog, history]) => {
        if (!current) return;
        const instanceOperations = catalog.filter((item) => item.scope === "instance");
        setOperations(instanceOperations);
        setSelectedOperation((selected) =>
          instanceOperations.some((item) => item.name === selected)
            ? selected
            : instanceOperations[0]?.name ?? "",
        );
        setOperationHistory(history);
      })
      .catch((reason) => {
        if (current) setError(String(reason));
      });
    return () => {
      current = false;
    };
  }, [activeInstance]);

  useEffect(() => {
    if (!selectedPlugin) { setPluginOperations([]); return; }
    let current = true;
    void hostAPI.listOperations(selectedPlugin.id).then((catalog) => {
      if (current) {
        const scoped = catalog.filter((item) => item.scope === "plugin");
        setPluginOperations(scoped);
        setSelectedPluginOperation((selected) => scoped.some((item) => item.name === selected) ? selected : scoped[0]?.name ?? "");
      }
    }).catch((reason) => { if (current) setError(String(reason)); });
    return () => { current = false; };
  }, [selectedPlugin]);

  const runPluginAction = async (pluginId: string, action: () => Promise<void>) => {
    setBusyId(pluginId);
    setError(null);
    try {
      await action();
      await refresh();
      await refreshWorkspace();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusyId(null);
    }
  };

  const createConfiguredInstance = async (pluginId: string, config: unknown) => {
    setBusyId(pluginId);
    setError(null);
    try {
      if (!plugins.find((plugin) => plugin.id === pluginId)?.running) {
        await hostAPI.startPlugin(pluginId);
      }
      const instance = await hostAPI.createInstance(pluginId, config);
      await refresh();
      await refreshWorkspace();
      setActiveInstanceId(instance.pluginInstanceId);
      setSelectedPluginId(null);
    } catch (reason) {
      setError(String(reason));
      throw reason;
    } finally {
      setBusyId(null);
    }
  };

  const createInstance = async (pluginId: string) => {
    try {
      const config: unknown = JSON.parse(configText);
      await createConfiguredInstance(pluginId, config);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const submitOperation = async () => {
    if (!activeInstance || !selectedOperation) return;
    setError(null);
    setOperationResult(null);
    try {
      const args: unknown = JSON.parse(operationArgs);
      const receipt = await hostAPI.invokeOperation(
        activeInstance.pluginInstanceId,
        selectedOperation,
        args,
      );
      setOperationResult(receipt);
      setOperationHistory(await hostAPI.recentOperations(activeInstance.pluginInstanceId, 20));
    } catch (reason) {
      setError(String(reason));
    }
  };

  const respondToProposal = async (record: OperationRecord, approve: boolean) => {
    if (!activeInstance || !record.proposalId) return;
    try {
      if (approve) {
        const args: unknown = JSON.parse(operationArgs);
        await hostAPI.approveProposal(
          activeInstance.pluginInstanceId,
          record.proposalId,
          args,
        );
      } else {
        await hostAPI.rejectProposal(activeInstance.pluginInstanceId, record.proposalId);
      }
      setOperationHistory(await hostAPI.recentOperations(activeInstance.pluginInstanceId, 20));
    } catch (reason) {
      setError(String(reason));
    }
  };

  const toggleBypass = async () => {
    if (!bypass && !window.confirm(
      "Enable Bypass? Agent commit operations will skip Coterm approval. You assume the risk.",
    )) return;
    try {
      await hostAPI.setBypass(!bypass);
      setBypass(!bypass);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const errorNotice = error && (
    <div role="alert" className="break-words border-l-2 border-[#f48771] bg-[#332626] px-3 py-2 text-xs leading-relaxed text-[#f4b8aa]">
      {error}
    </div>
  );

  const pluginIssueNotice = pluginIssues.length > 0 && (
    <ul role="status" aria-label="Plugin package issues" className="max-h-32 overflow-y-auto break-words border-l-2 border-[#d7ba7d] bg-[#3a3224] px-3 py-2 text-xs leading-relaxed text-[#e8d5a9]">
      {pluginIssues.map((issue) => (
        <li key={`${issue.directory}:${issue.code}`} title={issue.code}>
          <span className="font-medium">{issue.pluginId ? `${issue.pluginId} (${issue.directory})` : issue.directory}</span>
          {" — "}
          {issue.message}
        </li>
      ))}
    </ul>
  );

  return (
    <div className="flex h-dvh flex-col overflow-hidden bg-[#1e1e1e] text-[#cccccc]">
      <div
        role="toolbar"
        aria-label="Workspace tools"
        className="flex h-8 shrink-0 items-center justify-between border-b border-coterm-border bg-coterm-surface-toolbar px-3"
      >
        <div className="flex items-center gap-2">
          <button
            type="button"
            id="toggle-instances-button"
            aria-controls="instances-sidebar"
            aria-expanded={!instancesCollapsed}
            aria-label={instancesCollapsed ? "Expand instances panel" : "Collapse instances panel"}
            title={instancesCollapsed ? "Expand instances panel" : "Collapse instances panel"}
            onClick={() => setInstancesCollapsed((prev) => !prev)}
            className="icon-button text-coterm-text-muted hover:text-coterm-text-bright"
          >
            {instancesCollapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
          </button>
          <div className="h-3.5 w-px bg-coterm-border-subtle" />
          <button
            type="button"
            aria-haspopup="dialog"
            onClick={() => setMcpOpen(true)}
            className="rounded-sm px-2 text-[11px] text-coterm-text hover:bg-white/10"
          >
            MCP
          </button>
        </div>
      </div>
      <HostInteractions />
      {workspace?.persistence === "memory_only" && <p role="status" className="px-3 py-2 text-xs text-[#d7ba7d]">Workspace is session-only on this platform; secure disk storage is not available.</p>}
      {workspaceError && <div role="alert" className="px-3 py-2 text-xs text-[#f48771]">
        Workspace: {workspaceError}. Saved state is preserved.
        {instances.length === 0 && <button className="ml-3 underline" onClick={() => {
          void promptForConfirmation("workspace-reset", { title: "Reset saved workspace?", prompt: "This removes saved configuration references and layout. Plugin data is unaffected." })
            .then(async (result) => {
              if (result.outcome !== "submitted") return;
              await hostAPI.resetWorkspace();
              await refreshWorkspace();
            }).catch((reason) => setWorkspaceError(formatError(reason)));
        }}>Reset saved workspace</button>}
      </div>}
      {pluginIssueNotice}
      {mcpOpen && <McpConfiguration onClose={() => setMcpOpen(false)} />}

      <div className="flex min-h-0 flex-1">
        <aside aria-label="Installed plugins" className="flex w-[64px] shrink-0 flex-col border-r border-coterm-border bg-coterm-surface-sidebar sm:w-[72px]">
          <h1 className="px-1 pb-3 pt-4 text-center text-[9px] font-medium leading-[14px] tracking-[0.04em] text-[#b7b7b7]">
            INSTALLED<br />PLUGINS
          </h1>
          <nav aria-label="Plugin list" aria-busy={loading} className="min-h-0 flex-1 overflow-y-auto">
            {loading && plugins.length === 0 ? (
              <div aria-label="Loading plugins" className="flex flex-col items-center gap-4 py-2">
                {[0, 1, 2].map((item) => <div key={item} className="h-8 w-8 rounded bg-[#484848]" />)}
              </div>
            ) : plugins.map((plugin) => (
              <button
                key={plugin.id}
                type="button"
                title={`${plugin.name} · ${plugin.version} · ${plugin.running ? "Running" : "Stopped"}`}
                aria-label={`Manage ${plugin.name}`}
                aria-haspopup="dialog"
                aria-pressed={selectedPluginId === plugin.id}
                onClick={() => setSelectedPluginId(plugin.id)}
                className={`group relative flex min-h-16 w-full flex-col items-center justify-center gap-1 border-l-2 px-1 py-2 hover:bg-[#3e3e3e] ${selectedPluginId === plugin.id ? "border-[#75beff] bg-[#3e3e3e]" : "border-transparent"}`}
              >
                <span className="relative grid h-7 w-8 place-items-center">
                  <PlugZap size={25} strokeWidth={1.5} className={plugin.running ? "text-[#eeeeee]" : "text-[#aaaaaa]"} />
                  <span className={`absolute bottom-0 right-0 h-2 w-2 rounded-full border-2 border-[#333333] ${plugin.running ? "bg-[#75beff]" : "bg-[#969696]"}`} />
                </span>
                <span className="w-full truncate text-center text-[10px] text-[#bcbcbc]">{plugin.name}</span>
              </button>
            ))}
            {!loading && plugins.length === 0 && <p className="px-1 py-3 text-center text-[10px] leading-relaxed text-[#aaaaaa]">No plugins</p>}
          </nav>
          <div className="flex justify-center border-t border-[#414141] py-3">
            <button
              type="button"
              title={bypass ? "Bypass on — click to require approval" : "Agent approval on — click to enable bypass"}
              aria-label={bypass ? "Disable approval bypass" : "Enable approval bypass"}
              aria-pressed={bypass}
              onClick={() => void toggleBypass()}
              className="icon-button h-9 w-9"
            >
              {bypass ? <ShieldAlert size={22} className="text-[#cca770]" /> : <ShieldCheck size={22} />}
            </button>
          </div>
        </aside>

        <aside
          id="instances-sidebar"
          aria-label="Instances"
          className={`flex w-[240px] min-w-0 shrink-0 flex-col border-r border-coterm-border bg-coterm-surface-raised max-[1024px]:w-[200px] transition-[width] duration-200 ${instancesCollapsed ? "hidden" : ""}`}
        >
          <div className="flex h-10 shrink-0 items-center justify-between px-4">
            <h2 className="text-[11px] font-medium tracking-[0.06em]">INSTANCES</h2>
            <button
              type="button"
              aria-label="Collapse instances panel"
              title="Collapse instances panel"
              onClick={() => setInstancesCollapsed(true)}
              className="icon-button text-coterm-text-muted hover:text-coterm-text-bright"
            >
              <PanelLeftClose size={15} />
            </button>
          </div>
          <div className="flex h-7 shrink-0 items-center gap-1 border-y border-coterm-border bg-coterm-surface-panel px-2 text-[11px] font-semibold">
            <ChevronDown size={14} className="text-[#b7b7b7]" />
            <span>WORKSPACE</span>
            <span className="ml-auto pr-1 font-mono text-[10px] font-normal text-[#969696]">{instances.length}</span>
          </div>
          <nav aria-label="Instance list" aria-busy={loading} className="min-h-0 flex-1 overflow-y-auto py-1">
            {workspace?.entries.some((entry) => !instances.some((item) => item.pluginInstanceId === entry.instanceId)) &&
              <button disabled={busyId !== null} className="px-3 py-2 text-xs text-[#75beff]" onClick={() => void restoreWorkspace()}>Restore workspace</button>}
            {workspace?.entries.filter((entry) => !instances.some((item) => item.pluginInstanceId === entry.instanceId)).map((entry) => (
              <div key={entry.configRef} className="px-3 py-2 text-xs text-[#b7b7b7]">
                <p className="truncate">{entry.label}</p>
                <button disabled={busyId !== null} className="mr-3 text-[#75beff] disabled:opacity-50" onClick={() => void restoreSavedInstance(entry.configRef)}>Restore instance</button>
                <button disabled={busyId !== null} onClick={() => {
                  void hostAPI.forgetInstance(entry.configRef).then(refreshWorkspace).catch((reason) => setError(formatError(reason)));
                }}>Forget</button>
              </div>
            ))}
            {loading && instances.length === 0 ? (
              <div aria-label="Loading instances" className="space-y-3 px-5 py-3">
                {["w-3/4", "w-1/2"].map((width) => <div key={width} className={`h-3 bg-[#333333] ${width}`} />)}
              </div>
            ) : orderedInstances.map((instance) => (
              <div key={instance.pluginInstanceId} className={`group flex h-8 items-center ${activeInstanceId === instance.pluginInstanceId ? "bg-coterm-surface-active" : "hover:bg-coterm-surface-hover"}`}>
                <button
                  type="button"
                  title={`${instance.pluginId} · ${instance.status}`}
                  aria-current={activeInstanceId === instance.pluginInstanceId ? "true" : undefined}
                  onClick={() => setActiveInstanceId(instance.pluginInstanceId)}
                  className="flex h-full min-w-0 flex-1 items-center gap-2 pl-5 pr-1 text-left text-xs"
                >
                  <Box size={15} strokeWidth={1.5} className="shrink-0 text-[#75beff]" />
                  <span className="truncate">{instance.label}</span>
                </button>
                <button
                  type="button"
                  title={`Manage ${instance.label}`}
                  aria-label={`Manage ${instance.label}`}
                  aria-haspopup="dialog"
                  onClick={() => {
                    setActiveInstanceId(instance.pluginInstanceId);
                    setInstanceToolsOpen(true);
                  }}
                  className="icon-button mr-1 opacity-0 focus:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100"
                >
                  <MoreHorizontal size={16} />
                </button>
              </div>
            ))}
            {!loading && instances.length === 0 && (
              <div className="px-5 py-5 text-xs leading-5 text-[#969696]">
                <p>No instances yet.</p>
                <p className="mt-1">Select a plugin on the left to create one.</p>
              </div>
            )}
          </nav>
          {errorNotice}
        </aside>

        <main aria-label="Plugin workspace" className="relative isolate min-w-0 flex-1 overflow-hidden bg-coterm-surface">
          {activeInstance?.status === "ready" && <PluginWorkspace key={activeInstance.pluginInstanceId} instance={activeInstance} />}
          {activeInstance?.status === "failed" && <div role="alert" className="p-5 text-sm text-[#f48771]">
            <p>Plugin stopped unexpectedly. In-flight results are unknown. Operations will not be replayed.</p>
            {workspace?.entries.filter((entry) => entry.instanceId === activeInstance.pluginInstanceId).map((entry) =>
              <button key={entry.configRef} disabled={busyId !== null} className="mt-3 underline" onClick={() => void restoreSavedInstance(entry.configRef)}>Restart instance</button>)}
          </div>}
          <HostApprovals />
        </main>
      </div>

      <footer className="flex h-[22px] shrink-0 items-center justify-between border-t border-coterm-border bg-coterm-surface-raised px-3 text-[11px] text-coterm-text-muted">
        <div className="flex items-center gap-4">
          <span>{plugins.length} plugins</span>
          <span>{instances.length} instances</span>
          <span title="Bounded operation history is cleared on application exit">History: this session</span>
          {busyId && <span role="status">Updating…</span>}
        </div>
        <span className={bypass ? "text-[#cca770]" : ""}>{bypass ? "Bypass on" : "Agent approval on"}</span>
      </footer>

      {selectedPlugin && (
        <Modal title={selectedPlugin.name} onClose={() => setSelectedPluginId(null)}>
          <div className="mb-5 flex items-start gap-3">
            <PlugZap size={28} strokeWidth={1.5} className="mt-1 text-[#75beff]" />
            <div className="min-w-0">
              <h3 className="text-sm font-medium">{selectedPlugin.name}</h3>
              <p className="mt-1 break-all text-xs text-[#969696]">{selectedPlugin.id} · {selectedPlugin.version}</p>
              <p className="mt-2 text-xs">{selectedPlugin.running ? "Running" : "Stopped"}</p>
            </div>
          </div>
          <div className="mb-5 flex gap-2">
            <button
              type="button"
              disabled={busyId !== null || loading}
              onClick={() => void runPluginAction(selectedPlugin.id, () => selectedPlugin.running ? hostAPI.stopPlugin(selectedPlugin.id) : hostAPI.startPlugin(selectedPlugin.id))}
              className="flex h-8 items-center gap-2 border border-[#454545] px-3 text-xs hover:bg-[#383838] disabled:opacity-50"
            >
              {selectedPlugin.running ? <CircleStop size={14} /> : <PlugZap size={14} />}
              {selectedPlugin.running ? "Stop plugin" : "Start plugin"}
            </button>
          </div>
          {pluginOperations.length > 0 && (
            <section className="mb-5 border-t border-[#383838] pt-4">
              <h3 className="mb-3 text-xs font-semibold uppercase text-[#cccccc]">Plugin operations</h3>
              <select aria-label="Plugin operation" value={selectedPluginOperation} onChange={(event) => setSelectedPluginOperation(event.target.value)} className="mb-2 h-8 w-full border border-[#454545] bg-[#252526] px-2 text-xs">
                {pluginOperations.map((item) => <option key={item.name} value={item.name}>{item.name} · {item.effect}</option>)}
              </select>
              <textarea aria-label="Plugin operation arguments JSON" value={operationArgs} onChange={(event) => setOperationArgs(event.target.value)} spellCheck={false} className="h-24 w-full border border-[#454545] bg-[#1e1e1e] p-2 font-mono text-xs" />
              <button type="button" onClick={() => {
                try {
                  const args: unknown = JSON.parse(operationArgs);
                  void hostAPI.invokePluginOperation(selectedPlugin.id, selectedPluginOperation, args).then(setOperationResult).catch((reason) => setError(String(reason)));
                } catch (reason) { setError(String(reason)); }
              }} className="mt-2 h-8 bg-[#0e639c] px-3 text-xs text-white hover:bg-[#1177bb]">Invoke plugin operation</button>
              {operationResult && <pre className="mt-2 max-h-32 overflow-auto border border-[#383838] bg-[#1e1e1e] p-2 font-mono text-xs">{JSON.stringify(operationResult, null, 2)}</pre>}
            </section>
          )}
          {selectedPlugin.hasFrontend && (
            <PluginConfiguration key={selectedPlugin.id} pluginId={selectedPlugin.id} onSubmit={(config) => createConfiguredInstance(selectedPlugin.id, config)} />
          )}
          <details className="border-t border-[#383838] pt-4" open={!selectedPlugin.hasFrontend}>
            <summary className="mb-3 cursor-pointer text-xs text-[#969696]">Advanced JSON configuration</summary>
            <h3 className="mb-4 flex items-center gap-2 text-xs font-medium"><SlidersHorizontal size={14} /> New instance</h3>
            <label htmlFor="instance-config" className="mb-2 block text-xs text-[#aaaaaa]">Configuration (JSON)</label>
            <textarea
              id="instance-config"
              value={configText}
              onChange={(event) => setConfigText(event.target.value)}
              spellCheck={false}
              className="h-24 w-full resize-y border border-[#454545] bg-[#1e1e1e] p-2 font-mono text-xs outline-none focus:border-[#007fd4]"
            />
            <button
              type="button"
              disabled={busyId !== null || loading}
              onClick={() => void createInstance(selectedPlugin.id)}
              className="mt-3 inline-flex h-8 items-center gap-1.5 bg-[#0e639c] px-3 text-xs text-white hover:bg-[#1177bb] disabled:opacity-50"
            >
              <Plus size={14} /> Create instance
            </button>
          </details>
          <div className="mt-4">{errorNotice}</div>
        </Modal>
      )}
      {instanceToolsOpen && activeInstance && (
        <Modal title="Instance tools" onClose={() => setInstanceToolsOpen(false)}>
          {errorNotice}
          <div className="max-w-3xl">
            <div className="flex items-start justify-between gap-4 border-b border-[#383838] pb-4">
              <div className="min-w-0">
                <h2 className="truncate text-lg font-medium">{activeInstance.label}</h2>
                <p className="mt-1 break-all font-mono text-xs text-[#969696]">
                  {activeInstance.pluginInstanceId}
                </p>
              </div>
              <button
                type="button"
                title="Destroy instance"
                disabled={busyId !== null}
                onClick={() => void runPluginAction(activeInstance.pluginInstanceId, async () => {
                  await hostAPI.destroyInstance(activeInstance.pluginInstanceId);
                })}
                className="grid h-8 w-8 shrink-0 place-items-center text-[#cccccc] hover:bg-[#3c3c3c]"
              >
                <Trash2 className="h-4 w-4" />
              </button>
            </div>
            <dl className="grid grid-cols-[140px_1fr] gap-y-3 py-4 text-sm">
              <dt className="text-[#969696]">Plugin</dt>
              <dd>{activeInstance.pluginId}</dd>
              <dt className="text-[#969696]">Session</dt>
              <dd className="break-all font-mono text-xs">{activeInstance.sessionId}</dd>
              <dt className="text-[#969696]">Status</dt>
              <dd>{activeInstance.status}</dd>
            </dl>
            <div className="border-t border-[#383838] py-4 text-sm text-[#aaaaaa]">
              <div className="mb-3 flex items-center justify-between">
                <h3 className="text-xs font-semibold uppercase text-[#cccccc]">Operations</h3>
                <span className="text-xs text-[#969696]">{operations.length} registered</span>
              </div>
              {operations.length > 0 ? (
                <>
                  <label className="mb-1 block text-xs text-[#aaaaaa]" htmlFor="operation-name">
                    Operation
                  </label>
                  <select
                    id="operation-name"
                    value={selectedOperation}
                    onChange={(event) => setSelectedOperation(event.target.value)}
                    className="mb-3 h-9 w-full border border-[#454545] bg-[#252526] px-2 text-sm text-[#cccccc]"
                  >
                    {operations.map((operation) => (
                      <option key={operation.name} value={operation.name}>
                        {operation.name} · {operation.effect}
                      </option>
                    ))}
                  </select>
                  {schemaProperties && Object.keys(schemaProperties).length > 0 && (
                    <fieldset className="mb-3 space-y-3 border border-[#383838] p-3">
                      <legend className="px-1 text-xs text-[#969696]">Operation arguments</legend>
                      {Object.entries(schemaProperties).map(([name, property]) => {
                        let current: Record<string, unknown> = {};
                        try {
                          const parsed: unknown = JSON.parse(operationArgs);
                          if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) current = parsed as Record<string, unknown>;
                        } catch { current = {}; }
                        const value = current[name];
                        const required = schemaRequired.includes(name);
                        return (
                          <label key={name} className="block text-xs text-[#aaaaaa]">
                            <span className="mb-1 flex items-center gap-1">
                              <span>{property.title ?? name}</span>
                              {required && <span className="text-[#f48771]">*</span>}
                            </span>
                            {property.description && <span className="mb-1 block text-[11px] text-[#888888]">{property.description}</span>}
                            {property.enum ? (
                              <select value={value === undefined ? "" : JSON.stringify(value)} onChange={(event) => updateOperationArgument(name, event.target.value ? JSON.parse(event.target.value) : undefined)} className="h-8 w-full border border-[#454545] bg-[#1e1e1e] px-2 text-xs text-[#cccccc]">
                                <option value="">Select…</option>
                                {property.enum.map((option) => <option key={JSON.stringify(option)} value={JSON.stringify(option)}>{String(option)}</option>)}
                              </select>
                            ) : property.type === "boolean" ? (
                              <input type="checkbox" checked={value === true} onChange={(event) => updateOperationArgument(name, event.target.checked)} className="accent-[#0e639c]" />
                            ) : property.type === "object" || property.type === "array" ? (
                              <textarea aria-label={name} value={value === undefined ? "" : JSON.stringify(value, null, 2)} onChange={(event) => { try { updateOperationArgument(name, JSON.parse(event.target.value)); } catch { /* Keep the last valid structured value. */ } }} spellCheck={false} className="h-20 w-full border border-[#454545] bg-[#1e1e1e] p-2 font-mono text-xs text-[#cccccc] outline-none focus:border-[#75beff]" />
                            ) : (
                              <input type={property.type === "integer" || property.type === "number" ? "number" : "text"} required={required} min={property.minimum} max={property.maximum} value={value === undefined ? "" : String(value)} onChange={(event) => updateOperationArgument(name, event.target.value === "" ? undefined : property.type === "integer" ? Number.parseInt(event.target.value, 10) : property.type === "number" ? Number(event.target.value) : event.target.value)} className="h-8 w-full border border-[#454545] bg-[#1e1e1e] px-2 text-xs text-[#cccccc] outline-none focus:border-[#75beff]" />
                            )}
                          </label>
                        );
                      })}
                    </fieldset>
                  )}
                  <details>
                    <summary className="mb-2 cursor-pointer text-xs text-[#969696]">Advanced JSON</summary>
                    <label className="mb-1 block text-xs text-[#aaaaaa]" htmlFor="operation-args">Arguments (JSON)</label>
                    <textarea id="operation-args" value={operationArgs} onChange={(event) => setOperationArgs(event.target.value)} spellCheck={false} className="h-28 w-full resize-y border border-[#454545] bg-[#1e1e1e] p-2 font-mono text-xs text-[#cccccc] outline-none focus:border-[#75beff]" />
                  </details>
                  <button
                    type="button"
                    onClick={() => void submitOperation()}
                    className="mt-2 h-8 bg-[#0e639c] px-3 text-xs text-white hover:bg-[#1177bb]"
                  >
                    Invoke as human
                  </button>
                </>
              ) : (
                <p className="text-xs text-[#969696]">This plugin has no instance operations.</p>
              )}
              {operationResult && (
                <pre className="mt-3 max-h-48 overflow-auto border border-[#383838] bg-[#1e1e1e] p-3 font-mono text-xs text-[#cccccc]">
                  {JSON.stringify(operationResult, null, 2)}
                </pre>
              )}
              <div className="mt-5 border-t border-[#383838] pt-4">
                <h3 className="mb-2 text-xs font-semibold uppercase text-[#cccccc]">Recent operations</h3>
                <div className="divide-y divide-[#383838]">
                  {operationHistory.map((record) => (
                    <article key={record.operationId} className="py-2.5">
                      <div className="flex items-center justify-between gap-3">
                        <span className="truncate text-xs text-[#cccccc]">{record.operation}</span>
                        <span className="shrink-0 text-[11px] text-[#969696]">{record.status}</span>
                      </div>
                      <div className="mt-1 break-all font-mono text-[10px] text-[#969696]">
                        {record.operationId}
                      </div>
                      {record.status === "awaiting_approval" && record.proposalId && (
                        <div className="mt-2 flex gap-2">
                          <button
                            type="button"
                            onClick={() => void respondToProposal(record, true)}
                            className="h-7 bg-[#0e639c] px-2 text-xs text-white hover:bg-[#1177bb]"
                          >
                            Approve current JSON
                          </button>
                          <button
                            type="button"
                            onClick={() => void respondToProposal(record, false)}
                            className="h-7 border border-[#454545] px-2 text-xs text-[#cccccc] hover:bg-[#383838]"
                          >
                            Reject
                          </button>
                        </div>
                      )}
                    </article>
                  ))}
                  {operationHistory.length === 0 && (
                    <p className="py-3 text-xs text-[#969696]">No operations recorded.</p>
                  )}
                </div>
              </div>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
