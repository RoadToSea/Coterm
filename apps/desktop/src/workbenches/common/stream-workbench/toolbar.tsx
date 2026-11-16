import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Button } from "../../../components/ui/button";
import { Input } from "../../../components/ui/input";
import { FormatSwitchView } from "../../WorkbenchWidgets";
import { driverI18n } from "../../../i18n";
import { createTranslator, type Translator } from "./i18n";
import { useToolbarDrag } from "./toolbar-drag";

const labels = {
  format: "Receive format", timestamps: "Show timestamps", autoscroll: "Auto-scroll",
  pause: "Pause / resume", clear: "Clear", save: "Save receive buffer", stats: "Byte counters",
};
type Id = keyof typeof labels;
interface Layout { order: Id[]; hidden: Id[] }
interface ToolbarController {
  panel: HTMLElement; trash: HTMLElement; done: HTMLButtonElement;
  setEditing(value: boolean): void;
  locale(t: Translator): void;
  unmount(): void;
}
const controllers = new WeakMap<HTMLElement, ToolbarController>();
const defaults = Object.keys(labels) as Id[];

function readLayout(key: string): Layout {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(key) ?? "null");
    if (saved && typeof saved === "object" && "order" in saved && "hidden" in saved
      && Array.isArray(saved.order) && Array.isArray(saved.hidden)) {
      const known = (id: unknown): id is Id => typeof id === "string" && Object.prototype.hasOwnProperty.call(labels, id);
      return { order: [...new Set([...saved.order.filter(known), ...defaults])], hidden: saved.hidden.filter(known) };
    }
  } catch { /* Layout remains usable when storage is unavailable. */ }
  return { order: [...defaults], hidden: [] };
}

/** Bind the existing host view; a missing React toolbar is a mount error. */
export function createToolbar(bar: HTMLElement, t: Translator): ToolbarController {
  const controller = controllers.get(bar);
  if (!controller) throw new Error("Missing React stream toolbar");
  controller.locale(t);
  return controller;
}

/** Keyed React controls preserve focus while users reorder, hide and restore items. */
export function StreamToolbarView({ namespace }: { namespace: string }) {
  const key = `coterm.${namespace}.toolbar.v1`;
  const [layout, setLayout] = useState(() => readLayout(key));
  const currentLayout = useRef(layout);
  const [editing, setEditing] = useState(false);
  const [t, setTranslate] = useState(() => createTranslator(driverI18n));
  const pendingFocus = useRef<Id | null>();
  const bar = useRef<HTMLDivElement>(null), panel = useRef<HTMLElement>(null), trash = useRef<HTMLDivElement>(null), done = useRef<HTMLButtonElement>(null);
  const save = (next: Layout) => {
    currentLayout.current = next;
    setLayout(next);
    try { localStorage.setItem(key, JSON.stringify(next)); } catch { /* Session layout still works. */ }
  };
  const focus = (id?: Id) => { pendingFocus.current = id ?? null; };
  useLayoutEffect(() => {
    if (pendingFocus.current === undefined) return;
    const id = pendingFocus.current;
    pendingFocus.current = undefined;
    (id ? bar.current?.querySelector<HTMLElement>(`[data-control="${id}"]`) : done.current)?.focus();
  }, [layout]);
  const move = (id: Id, target: Id, after: boolean) => {
    if (id === target) return;
    const layout = currentLayout.current;
    const order = layout.order.filter(value => value !== id);
    order.splice(order.indexOf(target) + Number(after), 0, id);
    save({ ...layout, order }); focus(id);
  };
  const remove = (id: Id) => {
    const layout = currentLayout.current;
    if (!editing || layout.hidden.includes(id)) return;
    const visible = layout.order.filter(value => !layout.hidden.includes(value)), index = visible.indexOf(id);
    save({ ...layout, hidden: [...layout.hidden, id] }); focus(visible[index + 1] ?? visible[index - 1]);
  };
  const { dragged, drop, overTrash, handlers } = useToolbarDrag({ editing, bar, trash,
    move: (id, target, after) => move(id as Id, target as Id, after), remove: id => remove(id as Id) });
  useLayoutEffect(() => {
    const element = bar.current!, settings = panel.current!;
    const position = () => settings.style.setProperty("--toolbar-height", `${element.offsetHeight}px`);
    const observer = new ResizeObserver(position); observer.observe(element); position();
    controllers.set(element, {
      panel: settings, trash: trash.current!, done: done.current!,
      setEditing(value) { setEditing(value); },
      locale(next) { setTranslate(() => next); }, unmount() { setEditing(false); },
    });
    return () => { controllers.delete(element); observer.disconnect(); };
  }, []);
  const controls: Record<Id, ReactNode> = {
    format: <FormatSwitchView name="receiveFormats" />,
    timestamps: <label><Input type="checkbox" data-ui="timestamps" /><span data-ui="timestampsText" /></label>,
    autoscroll: <label><Input type="checkbox" data-ui="autoscroll" /><span data-ui="autoscrollText" /></label>,
    pause: <Button variant="ghost" size="icon" className="icon-button" data-ui="pause"><svg viewBox="0 0 24 24" aria-hidden="true"><path data-ui="pauseIcon" d="M8 5v14M16 5v14" /></svg></Button>,
    clear: <Button variant="ghost" size="icon" className="icon-button" data-ui="clear"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7" /></svg></Button>,
    save: <Button variant="ghost" size="icon" className="export-receive icon-button" data-ui="exportButton"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5" /></svg></Button>,
    stats: <span className="stats" data-ui="stats" />,
  };
  return <>
    <div ref={bar} className="uart-bar" data-ui="uartBar" data-customizing={String(editing)}
      onClickCapture={event => { if (editing && !(event.target as Element).closest(".toolbar-toggle")) { event.preventDefault(); event.stopPropagation(); } }}>
      {layout.order.map(id => <div key={id} className="toolbar-item" data-control={id} hidden={layout.hidden.includes(id)}
        tabIndex={editing ? 0 : undefined} role={editing ? "group" : undefined}
        aria-label={editing ? t("Reorder {item}", { item: t(labels[id]) }) : undefined}
        aria-keyshortcuts={editing ? "Alt+ArrowLeft Alt+ArrowRight Delete Backspace" : undefined}
        title={editing ? t("Hold and drag to reorder; drop on the trash to remove. Delete removes; Alt + Left / Right moves.") : undefined}
        data-dragging={dragged === id || undefined} data-drop-side={drop?.id === id ? drop.after ? "after" : "before" : undefined}
        ref={element => { if (element?.firstElementChild instanceof HTMLElement) element.firstElementChild.inert = editing; }}
        {...handlers}
        onKeyDown={event => {
          if (!editing || event.ctrlKey || event.metaKey) return;
          if (!event.altKey && ["Delete", "Backspace"].includes(event.key)) { event.preventDefault(); event.stopPropagation(); remove(id); }
          if (!event.altKey || !["ArrowLeft", "ArrowRight"].includes(event.key)) return;
          event.preventDefault(); event.stopPropagation();
          const visible = layout.order.filter(value => !layout.hidden.includes(value)), delta = event.key === "ArrowLeft" ? -1 : 1;
          const target = visible[visible.indexOf(id) + delta]; if (target) move(id, target, delta > 0);
        }}>{controls[id]}</div>)}
      <Button variant="ghost" size="icon" className="toolbar-toggle icon-button" data-ui="toolbarButton" aria-expanded={editing} aria-controls={`${namespace}-toolbar-settings`} title={t("Customize toolbar")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h7m4 0h5M4 17h2m4 0h10M11 4v6M6 14v6" /></svg>
      </Button>
    </div>
    <section ref={panel} id={`${namespace}-toolbar-settings`} className="toolbar-customization" hidden={!editing}
      aria-label={t("Customize toolbar")} title={t("Hold and drag to reorder. Drop on the trash to remove.")}>
      <div className="toolbar-edit-actions"><Button variant="ghost" size="sm" className="toolbar-reset" onClick={() => save({ order: [...defaults], hidden: [] })}>{t("Restore default toolbar")}</Button><Button variant="ghost" size="sm" ref={done} className="toolbar-done">{t("Done")}</Button></div>
      <div className="toolbar-removed" hidden={!layout.hidden.length}><span>{t("Removed:")}</span>
        {layout.order.filter(id => layout.hidden.includes(id)).map(id => <Button variant="ghost" key={id} size="sm" data-restore={id}
          aria-label={t("Add {item} back to the toolbar", { item: t(labels[id]) })} onClick={() => { const layout = currentLayout.current; save({ ...layout, hidden: layout.hidden.filter(value => value !== id) }); focus(id); }}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10" /></svg>{t(labels[id])}
        </Button>)}
      </div>
    </section>
    <div ref={trash} className="toolbar-trash" hidden={!dragged} role="group" data-over={overTrash || undefined} aria-label={t("Drop here to remove from toolbar")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7" /></svg><span>{t("Drop here to remove from toolbar")}</span>
    </div>
  </>;
}
