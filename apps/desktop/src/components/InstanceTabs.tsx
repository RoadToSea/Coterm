import { useEffect, useRef, type KeyboardEvent } from "react";
import { PlugZap, X } from "lucide-react";
import type { InstanceInfo } from "../host-api";
import { t } from "../i18n";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";

export const instanceTabId = (id: string) => `instance-tab-${id}`;
export const instancePanelId = (id: string) => `instance-panel-${id}`;

interface Props {
  instances: InstanceInfo[];
  activeId: string | null;
  onSelect(id: string): void;
  onClose(id: string): void;
}

export function InstanceTabs({ instances, activeId, onSelect, onClose }: Props) {
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const container = list.current;
    if (!container || !activeId) return;
    const reveal = () => document.getElementById(instanceTabId(activeId))?.scrollIntoView({ block: "nearest", inline: "nearest" });
    reveal();
    const observer = new ResizeObserver(reveal);
    observer.observe(container);
    return () => observer.disconnect();
  }, [activeId, instances.length]);
  if (!instances.length) return null;
  const focus = (instance: InstanceInfo | undefined) => {
    if (!instance) return;
    onSelect(instance.pluginInstanceId);
    document.getElementById(instanceTabId(instance.pluginInstanceId))?.focus();
  };
  const close = (index: number) => {
    const instance = instances[index];
    const restoreFocus = list.current?.contains(document.activeElement);
    const target = activeId === instance.pluginInstanceId
      ? instances[index + 1] ?? instances[index - 1]
      : instances.find(item => item.pluginInstanceId === activeId) ?? instances[index + 1] ?? instances[index - 1];
    onClose(instance.pluginInstanceId);
    if (restoreFocus) {
      if (target) document.getElementById(instanceTabId(target.pluginInstanceId))?.focus();
      else list.current?.parentElement?.focus();
    }
  };
  const navigate = (event: KeyboardEvent, index: number) => {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    let target: number;
    switch (event.key) {
      case "ArrowRight": target = (index + 1) % instances.length; break;
      case "ArrowLeft": target = (index - 1 + instances.length) % instances.length; break;
      case "Home": target = 0; break;
      case "End": target = instances.length - 1; break;
      case "Delete": event.preventDefault(); close(index); return;
      default: return;
    }
    event.preventDefault();
    focus(instances[target]);
  };
  return <div ref={list} role="tablist" aria-label={t("Open instances")} aria-orientation="horizontal"
    className="flex min-h-11 shrink-0 gap-1 overflow-x-auto border-b border-border bg-card px-2 pt-1">
    {instances.map((instance, index) => <div key={instance.pluginInstanceId} role="presentation"
      className={cn("flex shrink-0 items-center rounded-t-lg border border-b-0", activeId === instance.pluginInstanceId ? "border-border bg-background" : "border-transparent")}>
      <Button type="button" variant="ghost" size="sm" role="tab" id={instanceTabId(instance.pluginInstanceId)}
        aria-selected={activeId === instance.pluginInstanceId} aria-controls={instancePanelId(instance.pluginInstanceId)}
        tabIndex={activeId === instance.pluginInstanceId || (!activeId && index === 0) ? 0 : -1}
        title={`${instance.label} · ${instance.pluginId}`} className="max-w-52 justify-start"
        onClick={() => onSelect(instance.pluginInstanceId)} onKeyDown={event => navigate(event, index)}>
        <PlugZap data-icon="inline-start" /><span className="truncate">{instance.label}</span>
      </Button>
      <Button type="button" variant="ghost" size="icon" className="mr-1 size-6 shrink-0"
        aria-label={t("Close view {name}", { name: instance.label })} title={t("Close view; connection stays open")}
        onClick={() => close(index)}><X data-icon="inline-start" /></Button>
    </div>)}
  </div>;
}
