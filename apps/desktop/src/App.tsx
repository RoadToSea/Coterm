import { t } from "./i18n";
import { useI18n } from "./use-i18n";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useInstanceViews } from "./use-instance-views";
import { InstanceTabs, instanceTabId, instancePanelId } from "./components/InstanceTabs";
import { PanelLeftClose, PanelLeftOpen, Plus, PlugZap, ShieldAlert, ShieldCheck, SlidersHorizontal, TerminalSquare, Trash2 } from "lucide-react";
import { WorkspaceInstances, workspaceName } from "./components/WorkspaceInstances";
import { ThemeSwitcher } from "./components/ThemeSwitcher";
import { Button } from "./components/ui/button";
import { Badge } from "./components/ui/badge";
import { Separator } from "./components/ui/separator";
import { Skeleton } from "./components/ui/skeleton";
import { Alert, AlertDescription } from "./components/ui/alert";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "./components/ui/empty";
import { cn } from "./lib/utils";
import { HostInteractions } from "./components/host/HostInteractions";
import { HostApprovals } from "./components/host/HostApprovals";
import { ApprovalSettings } from "./components/host/ApprovalSettings";
import { Modal } from "./components/ui/Modal";
import { PluginWorkspace } from "./components/plugins/PluginWorkspace";
import { PluginConfiguration } from "./components/plugins/PluginConfiguration";
import { PluginInstaller } from "./components/plugins/PluginInstaller";
import {
  hostAPI,
  type InstanceInfo,
  type PluginLoadIssue,
  type PluginSummary,
  type SavedWorkspace,
  type WorkspaceGroup,
} from "./host-api";
import { promptForConfirmation, promptForText } from "./secret-input";

const EMPTY_WORKSPACE: SavedWorkspace = {
  entries: [], groups: [{ id: "default", name: "Default workspace" }],
  layout: { activeConfigRef: null, instancesCollapsed: false, activeWorkspaceId: "default" },
  allowedOperations: [], historyPolicy: "session_only", persistence: "memory_only",
};

export default function App() {
  const { locale, setLocale } = useI18n();
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

  const [approvalOpen, setApprovalOpen] = useState(false);
  const [installerOpen, setInstallerOpen] = useState(false);
  const [plugins, setPlugins] = useState<PluginSummary[]>([]);
  const [pluginIssues, setPluginIssues] = useState<PluginLoadIssue[]>([]);
  const [instances, setInstances] = useState<InstanceInfo[]>([]);
  const [managedInstanceId, setManagedInstanceId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [bypass, setBypass] = useState(false);
  const [workspace, setWorkspace] = useState<SavedWorkspace | null>(null);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState("default");
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const views = useInstanceViews(instances, loading);
  const activeInstanceId = views.activeId;
  const selectedPluginId = views.pluginId;
  const managedInstance = instances.find(instance => instance.pluginInstanceId === managedInstanceId) ?? null;
  const openInstances = views.ids.flatMap(id => instances.find(instance => instance.pluginInstanceId === id) ?? []);
  const formatError = (reason: unknown) => typeof reason === "object" && reason !== null && "code" in reason
    ? String((reason as { code: unknown }).code) : String(reason);

  const refreshWorkspace = useCallback(async () => {
    try {
      const saved = await hostAPI.workspace();
      setWorkspace(saved);
      setActiveWorkspaceId(current => saved.groups.some(group => group.id === current) ? current : saved.layout.activeWorkspaceId);
      setWorkspaceError(null);
      return saved;
    } catch (reason) {
      setWorkspaceError(formatError(reason));
      return null;
    }
  }, []);

  useEffect(() => {
    void refreshWorkspace().then((saved) => {
      if (saved) {
        setInstancesCollapsed(saved.layout.instancesCollapsed);
        setActiveWorkspaceId(saved.layout.activeWorkspaceId);
      }
    });
  }, [refreshWorkspace]);


  const orderedInstances = useMemo(() => [...instances].sort((left, right) => {
    const rank = (id: string) => {
      const index = workspace?.entries.findIndex((entry) => entry.instanceId === id) ?? -1;
      return index < 0 ? Number.MAX_SAFE_INTEGER : index;
    };
    return rank(left.pluginInstanceId) - rank(right.pluginInstanceId);
  }), [instances, workspace]);

  useEffect(() => {
    if (!workspace || workspaceError || busyId !== null) return;
    const timer = window.setTimeout(() => {
      void hostAPI.saveLayout(activeInstanceId, instancesCollapsed, orderedInstances.map((item) => item.pluginInstanceId), activeWorkspaceId)
        .catch((reason) => setWorkspaceError(formatError(reason)));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [activeInstanceId, instancesCollapsed, orderedInstances, workspace, workspaceError, activeWorkspaceId, busyId]);

  const selectedPlugin = plugins.find((plugin) => plugin.id === selectedPluginId) ?? null;
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
      views.open(instance.pluginInstanceId);
      const entry = workspace?.entries.find(item => item.configRef === configRef);
      if (entry) setActiveWorkspaceId(entry.workspaceId);
    } catch (reason) { setError(formatError(reason)); }
    finally { setBusyId(null); }
  };

  const restoreWorkspace = async (workspaceId: string) => {
    if (!workspace) return;
    setBusyId("restore-workspace");
    setError(null);
    const failures: string[] = [];
    const restored: string[] = [];
    let active: string | null = null;
    for (const entry of workspace.entries) {
      if (entry.workspaceId !== workspaceId) continue;
      if (instances.some((item) => item.pluginInstanceId === entry.instanceId)) continue;
      try {
        const instance = await hostAPI.restoreInstance(entry.configRef);
        restored.push(instance.pluginInstanceId);
        if (!active || entry.configRef === workspace.layout.activeConfigRef) active = instance.pluginInstanceId;
      } catch (reason) { failures.push(`${entry.label}: ${formatError(reason)}`); }
    }
    await refresh();
    await refreshWorkspace();
    for (const id of restored) views.open(id);
    if (active) views.open(active);
    setActiveWorkspaceId(workspaceId);
    if (failures.length) setError(failures.join("; "));
    setBusyId(null);
  };

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
      const instance = await hostAPI.createInstance(pluginId, config, activeWorkspaceId);
      await refresh();
      await refreshWorkspace();
      views.open(instance.pluginInstanceId);
    } catch (reason) {
      setError(String(reason));
      throw reason;
    } finally {
      setBusyId(null);
    }
  };

  const toggleBypass = async () => {
    if (!bypass && !window.confirm(
      t("Enable Bypass? Agent commit operations will skip Coterm approval. You assume the risk."),
    )) return;
    try {
      await hostAPI.setBypass(!bypass);
      setBypass(!bypass);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const createWorkspace = async () => {
    const result = await promptForText("create-workspace", { title: t("New workspace"), prompt: t("Workspace name (up to 64 characters)"), allowCancel: true });
    if (result.outcome !== "submitted") return;
    setBusyId("workspace-create");
    setError(null);
    try {
      const group = await hostAPI.createWorkspace(result.value);
      setActiveWorkspaceId(group.id);
      await refreshWorkspace();
    } catch (reason) { setError(t(formatError(reason))); }
    finally { setBusyId(null); }
  };

  const deleteWorkspace = async (group: WorkspaceGroup) => {
    const target = workspace?.groups.find(item => item.id !== group.id);
    if (!target) return;
    const result = await promptForConfirmation("delete-workspace", {
      title: t("Delete workspace {name}", { name: workspaceName(group) }),
      prompt: t("Instances move to {name}. Connections remain open.", { name: workspaceName(target) }), allowCancel: true,
    });
    if (result.outcome !== "submitted") return;
    setBusyId(group.id);
    setError(null);
    try {
      await hostAPI.deleteWorkspace(group.id);
      if (activeWorkspaceId === group.id) setActiveWorkspaceId(target.id);
      await refreshWorkspace();
    } catch (reason) { setError(t(formatError(reason))); }
    finally { setBusyId(null); }
  };

  const moveWorkspaceEntry = async (configRef: string, workspaceId: string) => {
    setBusyId(configRef);
    setError(null);
    try {
      await hostAPI.moveWorkspaceEntry(configRef, workspaceId);
      setActiveWorkspaceId(workspaceId);
      await refreshWorkspace();
    } catch (reason) { setError(t(formatError(reason))); }
    finally { setBusyId(null); }
  };

  const destroyInstance = async (instance: InstanceInfo) => {
    const result = await promptForConfirmation(`destroy-${instance.pluginInstanceId}`, {
      title: t("Delete instance {name}", { name: instance.label }),
      prompt: t("This closes the connection and removes its saved restore entry."), allowCancel: true,
    });
    if (result.outcome !== "submitted") return;
    await runPluginAction(instance.pluginInstanceId, () => hostAPI.destroyInstance(instance.pluginInstanceId));
    setManagedInstanceId(null);
  };

  const errorNotice = error && (
    <Alert variant="destructive" className="break-words"><AlertDescription>{error}</AlertDescription></Alert>
  );

  const pluginIssueNotice = pluginIssues.length > 0 && (
    <ul role="status" aria-label={t("Plugin package issues")} className="max-h-32 overflow-y-auto break-words border-l-2 border-warning bg-warning-surface px-3 py-2 text-xs leading-relaxed text-warning">
      {pluginIssues.map((issue) => (
        <li key={`${issue.directory}:${issue.code}`} title={issue.code}>
          <span className="font-medium">{issue.pluginId ? `${issue.pluginId} (${issue.directory})` : issue.directory}</span>
          {" — "}
          {locale === "zh-CN" && t(issue.code) !== issue.code ? t(issue.code) : issue.message}
        </li>
      ))}
    </ul>
  );

  return (
    <div className="flex h-dvh flex-col overflow-hidden bg-coterm-surface text-coterm-text"
      onDragOver={event => { if (event.dataTransfer.types.includes("Files")) event.preventDefault(); }}
      onDrop={event => { if (event.dataTransfer.types.includes("Files")) event.preventDefault(); }}>
      <div
        role="toolbar"
        aria-label={t("Workspace tools")}
        className="flex h-14 shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-3"
      >
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-2 px-1">
            <TerminalSquare className="size-5 text-primary" />
            <span className="text-sm font-semibold tracking-tight">Coterm</span>
          </div>
        </div>
        <div className="flex items-center gap-3">
        <ThemeSwitcher />
        <label className="flex items-center gap-2 text-[11px] text-coterm-text-muted">
          <span className="hidden sm:inline">{t("Language")}</span>
          <select id="locale-select" aria-label={t("Language")} value={locale}
            onChange={(event) => setLocale(event.target.value as "en" | "zh-CN")}
            className="h-8 border border-input bg-background px-2 text-foreground">
            <option value="zh-CN">简体中文</option>
            <option value="en">English</option>
          </select>
        </label>
        </div>
      </div>
      <HostInteractions />
      {workspace?.persistence === "memory_only" && <p role="status" className="px-3 py-2 text-xs text-warning">{t("Workspace is session-only on this platform; secure disk storage is not available.")}</p>}
      {workspaceError && <div role="alert" className="px-3 py-2 text-xs text-destructive">
        {t("Workspace:")} {workspaceError}{t(". Saved state is preserved.")}
        {instances.length === 0 && <button className="ml-3 underline" onClick={() => {
          void promptForConfirmation("workspace-reset", { title: t("Reset saved workspace?"), prompt: t("This removes saved configuration references and layout. Plugin data is unaffected.") })
            .then(async (result) => {
              if (result.outcome !== "submitted") return;
              await hostAPI.resetWorkspace();
              await refreshWorkspace();
            }).catch((reason) => setWorkspaceError(formatError(reason)));
        }}>{t("Reset saved workspace")}</button>}
      </div>}
      {pluginIssueNotice}
      {approvalOpen && <ApprovalSettings plugins={plugins} onClose={() => setApprovalOpen(false)} />}
      {installerOpen && <PluginInstaller onClose={() => setInstallerOpen(false)} onInstalled={refresh} />}

      <div className="flex min-h-0 flex-1">
        <aside aria-label={t("Installed plugins")} className="flex w-[64px] shrink-0 flex-col border-r border-coterm-border bg-coterm-surface-sidebar sm:w-[72px]">
          {instancesCollapsed && <Button variant="ghost" size="icon" type="button" id="toggle-instances-button"
            className="mx-auto mt-3 shrink-0" aria-controls="instances-sidebar" aria-expanded={false}
            aria-label={t("Expand instances panel")} title={t("Expand instances panel")}
            onClick={() => setInstancesCollapsed(false)}><PanelLeftOpen data-icon="inline-start" /></Button>}
          <nav aria-label={t("Plugin list")} aria-busy={loading} className="min-h-0 flex-1 overflow-y-auto pt-3">
            {loading && plugins.length === 0 ? (
              <div aria-label={t("Loading plugins")} className="flex flex-col items-center gap-4 py-2">
                {[0, 1, 2].map((item) => <Skeleton key={item} className="size-8" />)}
              </div>
            ) : plugins.map((plugin) => (
              <button
                key={plugin.id}
                type="button"
                title={`${plugin.name} · ${plugin.version} · ${plugin.running ? t("Running") : t("Stopped")}`}
                aria-label={t("Manage {name}", { name: plugin.name })}
                aria-controls="plugin-configuration"
                aria-pressed={selectedPluginId === plugin.id}
                onClick={() => views.configure(plugin.id)}
                className={cn("group relative flex min-h-16 w-full flex-col items-center justify-center gap-1.5 border-l-2 px-1 py-3 transition-colors hover:bg-accent", selectedPluginId === plugin.id ? "border-primary bg-accent" : "border-transparent")}
              >
                <span className="relative grid h-7 w-8 place-items-center">
                  <PlugZap size={25} strokeWidth={1.5} className={plugin.running ? "text-primary" : "text-muted-foreground"} data-plugin-icon />
                  <span className={cn("absolute bottom-0 right-0 size-2 rounded-full ring-2 ring-card", plugin.running ? "bg-success" : "bg-muted-foreground")} />
                </span>
                <span className="w-full truncate text-center text-[10px] text-coterm-text-subtle">{plugin.name}</span>
              </button>
            ))}
            {!loading && plugins.length === 0 && <p className="px-1 py-3 text-center text-[10px] leading-relaxed text-coterm-text-subtle">{t("No plugins")}</p>}
          </nav>
          <Separator />
          <div className="flex flex-col items-center gap-2 py-3">
            <Button variant="outline" size="icon" type="button" id="install-plugin-button" title={t("Install plugin")} aria-label={t("Install plugin")}
              aria-haspopup="dialog" onClick={() => setInstallerOpen(true)}>
              <Plus data-icon="inline-start" />
            </Button>
            <Button variant="ghost" size="icon"
              type="button"
              title={t(bypass ? "Bypass on — click to require approval" : "Agent approval on — click to enable bypass")}
              aria-label={t(bypass ? "Disable approval bypass" : "Enable approval bypass")}
              aria-pressed={bypass}
              onClick={() => void toggleBypass()}
            >
              {bypass ? <ShieldAlert data-icon="inline-start" className="text-warning" /> : <ShieldCheck data-icon="inline-start" />}
            </Button>
            <Button variant="ghost" size="icon"
              type="button"
              title={t("Choose which Agent operations skip approval")}
              aria-label={t("Open approval settings")}
              onClick={() => setApprovalOpen(true)}
            >
              <SlidersHorizontal data-icon="inline-start" />
            </Button>
          </div>
        </aside>

        <aside
          id="instances-sidebar"
          aria-label={t("Instances")}
          className={cn("flex w-[240px] min-w-0 shrink-0 flex-col border-r border-coterm-border bg-coterm-surface-raised max-[1024px]:w-[200px]", instancesCollapsed && "hidden")}
        >
          <div className="flex h-14 shrink-0 items-center justify-between px-4">
            <h2 className="text-[11px] font-medium tracking-[0.06em]">{t("INSTANCES")}</h2>
            <Button variant="ghost" size="icon" type="button" className="ml-auto mr-1 size-7"
              disabled={busyId !== null || !workspace} aria-label={t("New workspace")} title={t("New workspace")}
              onClick={() => void createWorkspace()}><Plus data-icon="inline-start" /></Button>
            <button
              type="button"
              id={instancesCollapsed ? undefined : "toggle-instances-button"}
              aria-controls="instances-sidebar"
              aria-expanded={true}
              aria-label={t("Collapse instances panel")}
              title={t("Collapse instances panel")}
              onClick={() => setInstancesCollapsed(true)}
              className="icon-button text-coterm-text-muted hover:text-coterm-text-bright"
            >
              <PanelLeftClose size={15} />
            </button>
          </div>
          <nav aria-label={t("Instance list")} aria-busy={loading} className="min-h-0 flex-1 overflow-y-auto py-1">
            {loading && instances.length === 0 ? (
              <div aria-label={t("Loading instances")} className="flex flex-col gap-3 px-5 py-3">
                {["w-3/4", "w-1/2"].map((width) => <Skeleton key={width} className={cn("h-3", width)} />)}
              </div>
            ) : <WorkspaceInstances workspace={workspace ?? EMPTY_WORKSPACE} instances={orderedInstances}
              activeInstanceId={selectedPluginId ? null : activeInstanceId} activeWorkspaceId={activeWorkspaceId} busy={busyId !== null}
              onSelect={(instance, groupId) => { views.open(instance.pluginInstanceId); setActiveWorkspaceId(groupId); }}
              onSelectWorkspace={setActiveWorkspaceId} onManage={instance => setManagedInstanceId(instance.pluginInstanceId)}
              onDestroy={instance => void destroyInstance(instance)} onDeleteWorkspace={group => void deleteWorkspace(group)}
              onMove={(ref, groupId) => void moveWorkspaceEntry(ref, groupId)} onRestore={ref => void restoreSavedInstance(ref)}
              onRestoreWorkspace={groupId => void restoreWorkspace(groupId)}
              onForget={ref => { void runPluginAction(ref, () => hostAPI.forgetInstance(ref)); }} />}
            {!loading && instances.length === 0 && (workspace?.entries.length ?? 0) === 0 && (
              <div className="px-5 py-5 text-xs leading-5 text-coterm-text-muted">
                <p>{t("No instances yet.")}</p>
                <p className="mt-1">{t("Select a plugin on the left to create one.")}</p>
              </div>
            )}
          </nav>
        </aside>

        <main aria-label={t("Plugin workspace")} className="relative isolate min-w-0 flex-1 overflow-hidden flex flex-col bg-coterm-surface" tabIndex={-1}>
          <InstanceTabs instances={openInstances} activeId={selectedPluginId ? null : activeInstanceId}
            onSelect={id => {
              views.open(id);
              const entry = workspace?.entries.find(item => item.instanceId === id);
              if (entry) setActiveWorkspaceId(entry.workspaceId);
            }} onClose={views.close} />
          {errorNotice}
          {selectedPlugin && <section id="plugin-configuration" aria-label={t("New instance")}
            className="min-h-0 flex-1 overflow-y-auto p-5 sm:p-8">
            <div className="mx-auto max-w-xl rounded-xl border border-border bg-card p-5 sm:p-6">
              <div className="mb-6 flex items-center gap-3">
                <PlugZap className="size-6 shrink-0 text-primary" />
                <div className="min-w-0">
                  <h1 className="text-lg font-medium">{selectedPlugin.name} · {t("New instance")}</h1>
                  <p className="mt-1 text-xs text-muted-foreground">{t("Workspace:")} {workspaceName((workspace ?? EMPTY_WORKSPACE).groups.find(group => group.id === activeWorkspaceId) ?? EMPTY_WORKSPACE.groups[0])}</p>
                </div>
              </div>
              {selectedPlugin.hasFrontend
                ? <PluginConfiguration key={selectedPlugin.id} pluginId={selectedPlugin.id}
                    disabled={busyId !== null} onSubmit={config => createConfiguredInstance(selectedPlugin.id, config)} />
                : <p role="status" className="text-sm text-muted-foreground">{t("This plugin does not provide a connection configuration screen.")}</p>}
            </div>
          </section>}
          {!selectedPlugin && !activeInstanceId && !loading && <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon"><TerminalSquare /></EmptyMedia>
              <EmptyTitle>{t("Ready when you are")}</EmptyTitle>
              <EmptyDescription>{t("Open an instance from the sidebar, or select a plugin to create a connection.")}</EmptyDescription>
            </EmptyHeader>
            <EmptyContent><Button variant="outline" onClick={() => setInstallerOpen(true)}><Plus data-icon="inline-start" />{t("Install plugin")}</Button></EmptyContent>
          </Empty>}
          {openInstances.map(instance => <section key={instance.pluginInstanceId} role="tabpanel"
            id={instancePanelId(instance.pluginInstanceId)} aria-labelledby={instanceTabId(instance.pluginInstanceId)}
            hidden={!!selectedPluginId || instance.pluginInstanceId !== activeInstanceId} tabIndex={0}
            className="min-h-0 flex-1 overflow-hidden">
            {instance.status === "ready" && <PluginWorkspace instance={instance} />}
            {instance.status !== "ready" && instance.status !== "failed" && <p role="status" className="p-5 text-sm text-muted-foreground">{t(instance.status)}</p>}
            {instance.status === "failed" && <div role="alert" className="p-5 text-sm text-destructive">
              <p>{t("Plugin stopped unexpectedly. In-flight results are unknown. Operations will not be replayed.")}</p>
              {workspace?.entries.filter(entry => entry.instanceId === instance.pluginInstanceId).map(entry =>
                <Button key={entry.configRef} variant="outline" size="sm" disabled={busyId !== null}
                  className="mt-3" onClick={() => void restoreSavedInstance(entry.configRef)}>{t("Restart instance")}</Button>)}
            </div>}
          </section>)}
          <HostApprovals />
        </main>
      </div>

      <footer className="flex h-8 shrink-0 items-center justify-between gap-3 border-t border-coterm-border bg-coterm-surface-raised px-3 text-[10px] text-coterm-text-muted">
        <div className="flex items-center gap-4">
          <span>{plugins.length} {t("plugins")}</span>
          <span>{instances.length} {t("instances")}</span>
          <span className="hidden sm:inline" title={t("Bounded operation history is cleared on application exit")}>{t("History: this session")}</span>
          {busyId && <span role="status">{t("Updating…")}</span>}
        </div>
        <span className={bypass ? "text-warning" : ""}>{bypass ? t("Bypass on") : t("Agent approval on")}</span>
      </footer>

      {managedInstance && (
        <Modal title={t("Instance settings")} onClose={() => setManagedInstanceId(null)}>
          <div className="flex flex-col gap-5">
            {errorNotice}
            <div className="flex items-center justify-between gap-3">
              <h2 className="min-w-0 truncate text-lg font-medium">{managedInstance.label}</h2>
              <Badge variant="secondary">{t(managedInstance.status)}</Badge>
            </div>
            <p className="text-xs text-muted-foreground">{t("Plugin")}: {plugins.find(plugin => plugin.id === managedInstance.pluginId)?.name ?? managedInstance.pluginId}</p>
            {workspace && <label className="flex items-center gap-3 text-xs">
              {t("Move to workspace")}
              <select aria-label={t("Move to workspace")} disabled={busyId !== null || !workspace.entries.some(entry => entry.instanceId === managedInstance.pluginInstanceId)}
                value={workspace.entries.find(entry => entry.instanceId === managedInstance.pluginInstanceId)?.workspaceId ?? workspace.groups[0]?.id}
                className="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-2"
                onChange={event => {
                  const entry = workspace.entries.find(item => item.instanceId === managedInstance.pluginInstanceId);
                  if (entry) void moveWorkspaceEntry(entry.configRef, event.target.value);
                }}>
                {workspace.groups.map(group => <option key={group.id} value={group.id}>{workspaceName(group)}</option>)}
              </select>
            </label>}
            <Button variant="destructive" disabled={busyId !== null} onClick={() => void destroyInstance(managedInstance)}>
              <Trash2 data-icon="inline-start" />{t("Destroy instance")}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
