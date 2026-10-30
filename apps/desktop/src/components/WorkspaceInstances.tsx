import { useState, type DragEvent } from "react";
import { ChevronDown, ChevronRight, MoreHorizontal, PlugZap, RotateCcw, Trash2 } from "lucide-react";
import { t } from "../i18n";
import type { InstanceInfo, SavedWorkspace, WorkspaceGroup } from "../host-api";
import { cn } from "../lib/utils";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";

const ENTRY_TYPE = "application/x-coterm-workspace-entry";
export const workspaceName = (group: WorkspaceGroup) => group.id === "default" ? t("Default workspace") : group.name;

interface Props {
  workspace: SavedWorkspace;
  instances: InstanceInfo[];
  activeInstanceId: string | null;
  activeWorkspaceId: string;
  busy: boolean;
  onSelect(instance: InstanceInfo, workspaceId: string): void;
  onSelectWorkspace(workspaceId: string): void;
  onManage(instance: InstanceInfo): void;
  onDestroy(instance: InstanceInfo): void;
  onDeleteWorkspace(group: WorkspaceGroup): void;
  onMove(configRef: string, workspaceId: string): void;
  onRestore(configRef: string): void;
  onRestoreWorkspace(workspaceId: string): void;
  onForget(configRef: string): void;
}

/** Workspace membership is host metadata; dragging never reconnects a plugin. */
export function WorkspaceInstances(props: Props) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const { workspace, instances, activeInstanceId, activeWorkspaceId, busy } = props;
  const liveIds = new Set(instances.map(instance => instance.pluginInstanceId));
  const startDrag = (event: DragEvent, configRef: string) => {
    event.dataTransfer.setData(ENTRY_TYPE, configRef);
    event.dataTransfer.effectAllowed = "move";
  };
  return workspace.groups.map(group => {
    const entries = workspace.entries.filter(entry => entry.workspaceId === group.id);
    const live = instances.filter(instance => {
      const saved = workspace.entries.find(entry => entry.instanceId === instance.pluginInstanceId);
      return saved ? saved.workspaceId === group.id : group.id === workspace.groups[0]?.id;
    });
    const pending = entries.filter(entry => !entry.instanceId || !liveIds.has(entry.instanceId));
    const isCollapsed = collapsed.has(group.id);
    return <section key={group.id} data-workspace-id={group.id} aria-label={workspaceName(group)}
      className={cn("group/workspace mx-2 mb-3 rounded-lg border border-transparent", dropTarget === group.id && "border-primary bg-accent")}
      onDragOver={event => {
        if (busy || !event.dataTransfer.types.includes(ENTRY_TYPE)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        setDropTarget(group.id);
      }}
      onDragLeave={event => {
        if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) setDropTarget(null);
      }}
      onDrop={event => {
        event.preventDefault();
        setDropTarget(null);
        const ref = event.dataTransfer.getData(ENTRY_TYPE);
        if (!busy && workspace.entries.some(entry => entry.configRef === ref)) {
          setCollapsed(previous => { const next = new Set(previous); next.delete(group.id); return next; });
          props.onMove(ref, group.id);
        }
      }} onDragEnd={() => setDropTarget(null)}>
      <div className="flex h-9 items-center gap-1">
        <button type="button" aria-expanded={!isCollapsed} aria-controls={`workspace-${group.id}`}
          aria-current={activeWorkspaceId === group.id ? "true" : undefined}
          className={cn("flex h-full min-w-0 flex-1 items-center gap-1.5 px-1 text-left text-xs font-medium", activeWorkspaceId === group.id ? "text-primary" : "text-muted-foreground")}
          onClick={() => {
            props.onSelectWorkspace(group.id);
            setCollapsed(previous => { const next = new Set(previous); if (next.has(group.id)) next.delete(group.id); else next.add(group.id); return next; });
          }}>
          {isCollapsed ? <ChevronRight size={14} className="shrink-0" /> : <ChevronDown size={14} className="shrink-0" />}
          <span className="truncate">{workspaceName(group)}</span>
          <Badge variant="secondary" className="ml-auto">{live.length + pending.length}</Badge>
        </button>
        {pending.length > 0 && <Button type="button" variant="ghost" size="icon" className="size-6"
          disabled={busy} aria-label={t("Restore workspace")} title={t("Restore workspace")}
          onClick={() => props.onRestoreWorkspace(group.id)}><RotateCcw data-icon="inline-start" /></Button>}
        {workspace.groups.length > 1 && <Button type="button" variant="ghost" size="icon"
          className="size-6 opacity-0 group-hover/workspace:opacity-100 group-focus-within/workspace:opacity-100"
          disabled={busy} aria-label={t("Delete workspace {name}", { name: workspaceName(group) })}
          title={t("Delete workspace")} onClick={() => props.onDeleteWorkspace(group)}><Trash2 data-icon="inline-start" /></Button>}
      </div>
      <div id={`workspace-${group.id}`} hidden={isCollapsed}>
        {live.map(instance => {
          const entry = entries.find(item => item.instanceId === instance.pluginInstanceId);
          return <div key={instance.pluginInstanceId} data-instance-id={instance.pluginInstanceId}
            draggable={!busy && !!entry} onDragStart={event => { if (entry) startDrag(event, entry.configRef); }}
            className={cn("group/instance mb-1 flex h-10 items-center rounded-lg border transition-colors", activeInstanceId === instance.pluginInstanceId ? "border-primary/20 bg-accent text-accent-foreground" : "border-transparent hover:bg-muted")}>
            <button type="button" title={`${instance.pluginId} · ${instance.status}`}
              aria-current={activeInstanceId === instance.pluginInstanceId ? "true" : undefined}
              onClick={() => props.onSelect(instance, group.id)} className="flex h-full min-w-0 flex-1 items-center gap-2 pl-2 pr-1 text-left text-xs">
              <PlugZap size={15} strokeWidth={1.5} className="shrink-0 text-primary" data-plugin-icon />
              <span className="truncate">{instance.label}</span>
            </button>
            <div className="flex shrink-0 items-center pr-1 opacity-0 group-hover/instance:opacity-100 group-focus-within/instance:opacity-100">
              <Button type="button" variant="ghost" size="icon" className="size-6" disabled={busy}
                title={t("Manage {name}", { name: instance.label })} aria-label={t("Manage {name}", { name: instance.label })}
                aria-haspopup="dialog" onClick={() => props.onManage(instance)}><MoreHorizontal data-icon="inline-start" /></Button>
              <Button type="button" variant="ghost" size="icon" className="size-6" disabled={busy}
                title={t("Destroy instance")} aria-label={t("Delete instance {name}", { name: instance.label })}
                onClick={() => props.onDestroy(instance)}><Trash2 data-icon="inline-start" /></Button>
            </div>
          </div>;
        })}
        {pending.map(entry => <div key={entry.configRef} data-saved-ref={entry.configRef} draggable={!busy}
          onDragStart={event => startDrag(event, entry.configRef)}
          className="group/instance mb-1 flex min-h-10 items-center rounded-lg border border-border bg-muted/30">
          <button type="button" className="flex min-w-0 flex-1 items-center gap-2 px-2 py-2 text-left text-xs"
            title={t("Restore instance")} disabled={busy} onClick={() => props.onRestore(entry.configRef)}>
            <PlugZap size={15} strokeWidth={1.5} className="shrink-0 text-muted-foreground" data-plugin-icon />
            <span className="min-w-0"><span className="block truncate">{entry.label}</span><span className="block text-[10px] text-muted-foreground">{t("Restore instance")}</span></span>
          </button>
          <Button type="button" variant="ghost" size="icon" disabled={busy}
            className="mr-1 size-6 shrink-0 opacity-0 group-hover/instance:opacity-100 group-focus-within/instance:opacity-100"
            aria-label={t("Forget {name}", { name: entry.label })} title={t("Forget")} onClick={() => props.onForget(entry.configRef)}><Trash2 data-icon="inline-start" /></Button>
        </div>)}
        {live.length + pending.length === 0 && <p className="px-2 py-3 text-[11px] text-muted-foreground">{t("Drop instances here")}</p>}
      </div>
    </section>;
  });
}
