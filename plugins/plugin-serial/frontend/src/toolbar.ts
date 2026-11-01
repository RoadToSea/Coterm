import type { Translator } from "./i18n";

interface Item { id: string; label: string; element: HTMLElement }
const KEY = "coterm.serial.toolbar.v1";

/** Only UI layout IDs are persisted; no messages, file paths or protocol data enter storage. */
export function createToolbar(bar: HTMLElement, items: Item[], translate: Translator) {
  let t = translate;
  let order = items.map(item => item.id);
  const hidden = new Set<string>();
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (saved && typeof saved === "object" && "order" in saved && "hidden" in saved
      && Array.isArray(saved.order) && Array.isArray(saved.hidden)) {
      const known = new Set(order);
      const selected = saved.order.filter((id): id is string => typeof id === "string" && known.has(id));
      order = [...new Set([...selected, ...order])];
      for (const id of saved.hidden) if (typeof id === "string" && known.has(id)) hidden.add(id);
    }
  } catch { /* Unavailable storage or invalid preferences leave usable defaults. */ }
  const slots = new Map<string, HTMLElement>();
  for (const item of items) {
    const slot = document.createElement("div"); slot.className = "toolbar-item"; slot.dataset.control = item.id;
    slot.append(item.element); slots.set(item.id, slot);
  }
  const panel = document.createElement("section");
  panel.className = "toolbar-customization"; panel.hidden = true;
  const title = document.createElement("strong");
  const hint = document.createElement("p"); hint.className = "hint";
  const list = document.createElement("div"); list.className = "toolbar-settings-list";
  const reset = document.createElement("button"); reset.type = "button";
  panel.append(title, hint, list, reset);
  const controller = new AbortController();
  let editing = false;
  let dragged: string | null = null;
  const clearDrop = () => { for (const slot of slots.values()) delete slot.dataset.dropSide; };
  const finishDrag = () => {
    dragged = null; clearDrop();
    for (const slot of slots.values()) delete slot.dataset.dragging;
  };
  const save = () => {
    try { localStorage.setItem(KEY, JSON.stringify({ order, hidden: [...hidden] })); } catch { /* Session layout still works. */ }
  };
  const arrange = () => {
    for (const id of order) {
      const slot = slots.get(id)!; slot.hidden = hidden.has(id); bar.append(slot);
    }
  };
  const move = (id: string, target: string, after: boolean) => {
    if (id === target) return;
    const next = order.filter(value => value !== id);
    const index = next.indexOf(target);
    if (!slots.has(id) || index < 0) return;
    next.splice(index + Number(after), 0, id);
    if (next.every((value, index) => value === order[index])) return;
    order = next; arrange(); save(); render();
    slots.get(id)?.focus({ preventScroll: true });
  };
  const render = () => {
    panel.setAttribute("aria-label", t("Customize toolbar")); title.textContent = t("Customize toolbar");
    hint.textContent = t("Drag controls in the toolbar to reorder them. Alt + Left / Right also moves the focused control.");
    reset.textContent = t("Restore default toolbar"); list.replaceChildren();
    order.forEach(id => {
      const item = items.find(value => value.id === id)!;
      const slot = slots.get(id)!;
      slot.draggable = editing; item.element.inert = editing;
      if (editing) {
        slot.tabIndex = 0; slot.setAttribute("role", "group");
        slot.setAttribute("aria-label", t("Reorder {item}", { item: t(item.label) }));
        slot.setAttribute("aria-keyshortcuts", "Alt+ArrowLeft Alt+ArrowRight");
        slot.title = t("Drag to reorder. Alt + Left / Right also works.");
      } else {
        for (const name of ["tabindex", "role", "aria-label", "aria-keyshortcuts", "title"]) slot.removeAttribute(name);
      }
      const row = document.createElement("div"); row.className = "toolbar-setting"; row.dataset.item = id;
      const label = document.createElement("label"), check = document.createElement("input"), text = document.createElement("span");
      check.type = "checkbox"; check.checked = !hidden.has(id); text.textContent = t(item.label);
      check.addEventListener("change", () => {
        if (check.checked) hidden.delete(id); else hidden.add(id);
        arrange(); save();
      });
      label.append(check, text); row.append(label);
      list.append(row);
    });
  };
  for (const [id, slot] of slots) {
    slot.addEventListener("dragstart", event => {
      if (!editing || !event.dataTransfer) { event.preventDefault(); return; }
      finishDrag(); dragged = id; slot.dataset.dragging = "true";
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("application/x-coterm-serial-toolbar", id);
    }, { signal: controller.signal });
    slot.addEventListener("dragover", event => {
      if (!editing || !dragged) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      clearDrop();
      if (dragged !== id) slot.dataset.dropSide = event.clientX < slot.getBoundingClientRect().left + slot.offsetWidth / 2 ? "before" : "after";
    }, { signal: controller.signal });
    slot.addEventListener("dragleave", () => { delete slot.dataset.dropSide; }, { signal: controller.signal });
    slot.addEventListener("drop", event => {
      if (!editing || !dragged) return;
      event.preventDefault();
      const source = dragged;
      const after = event.clientX >= slot.getBoundingClientRect().left + slot.offsetWidth / 2;
      finishDrag(); move(source, id, after);
    }, { signal: controller.signal });
    slot.addEventListener("dragend", finishDrag, { signal: controller.signal });
    slot.addEventListener("keydown", event => {
      if (!editing || !event.altKey || event.ctrlKey || event.metaKey || !["ArrowLeft", "ArrowRight"].includes(event.key)) return;
      event.preventDefault(); event.stopPropagation();
      const visible = order.filter(value => !hidden.has(value));
      const delta = event.key === "ArrowLeft" ? -1 : 1;
      const target = visible[visible.indexOf(id) + delta];
      if (target) move(id, target, delta > 0);
    }, { signal: controller.signal });
  }
  bar.addEventListener("click", event => {
    if (editing) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, { capture: true, signal: controller.signal });
  const positionPanel = () => panel.style.setProperty("--toolbar-height", `${bar.offsetHeight}px`);
  const observer = new ResizeObserver(positionPanel); observer.observe(bar);
  reset.addEventListener("click", () => { order = items.map(item => item.id); hidden.clear(); arrange(); save(); render(); }, { signal: controller.signal });
  arrange(); render();
  return {
    panel,
    setEditing(value: boolean) {
      editing = value; finishDrag(); bar.dataset.customizing = String(value); positionPanel(); render();
    },
    locale(next: Translator) { t = next; render(); },
    unmount() {
      editing = false; finishDrag(); controller.abort(); observer.disconnect();
      for (const item of items) item.element.inert = false;
      for (const slot of slots.values()) slot.draggable = false;
      delete bar.dataset.customizing; panel.remove();
    },
  };
}
