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
    const reveal = () => {
      const el = document.getElementById(instanceTabId(activeId));
      (el?.parentElement ?? el)?.scrollIntoView({ block: "nearest", inline: "nearest" });
    };
    reveal();
    const observer = new ResizeObserver(reveal);
    observer.observe(container);
    return () => observer.disconnect();
  }, [activeId, instances.length]);
  if (!instances.length) return null;
  const focus = (instance: InstanceInfo | undefined) => {
    if (!instance) return;
    onSelect(instance.driverInstanceId);
    document.getElementById(instanceTabId(instance.driverInstanceId))?.focus();
  };
  const close = (index: number) => {
    const instance = instances[index];
    const restoreFocus = list.current?.contains(document.activeElement);
    const target = activeId === instance.driverInstanceId
      ? instances[index + 1] ?? instances[index - 1]
      : instances.find(item => item.driverInstanceId === activeId) ?? instances[index + 1] ?? instances[index - 1];
    onClose(instance.driverInstanceId);
    if (restoreFocus) {
      if (target) document.getElementById(instanceTabId(target.driverInstanceId))?.focus();
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
    className="flex min-h-11 shrink-0 gap-3 overflow-x-auto border-b border-border bg-card px-3 pt-1.5">
    {instances.map((instance, index) => {
      const active = activeId === instance.driverInstanceId;
      return (
        <div key={instance.driverInstanceId} role="presentation"
          className={cn(
            "flex shrink-0 items-center transition-colors",
            active
              ? "coterm-tab-active"
              : "rounded-t-lg text-muted-foreground hover:bg-muted/50 hover:text-foreground"
          )}>
          <Button type="button" variant="ghost" size="sm" role="tab" id={instanceTabId(instance.driverInstanceId)}
            aria-selected={active} aria-controls={instancePanelId(instance.driverInstanceId)}
            tabIndex={active || (!activeId && index === 0) ? 0 : -1}
            title={`${instance.name} · ${instance.driverId}`} className={cn("max-w-52 justify-start font-medium", active ? "text-foreground" : "text-muted-foreground")}
            onClick={() => onSelect(instance.driverInstanceId)} onKeyDown={event => navigate(event, index)}>
            <PlugZap data-icon="inline-start" /><span className="truncate">{instance.name}</span>
          </Button>
          <Button type="button" variant="ghost" size="icon" className="mr-1 size-6 shrink-0 rounded-full hover:bg-muted"
            aria-label={t("Close view {name}", { name: instance.name })} title={t("Close view; connection stays open")}
            onClick={() => close(index)}><X data-icon="inline-start" /></Button>
        </div>
      );
    })}
  </div>;
}
