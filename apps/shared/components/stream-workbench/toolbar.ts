import type { Translator } from "./i18n";

interface Item { id: string; label: string; element: HTMLElement }

/** Only UI layout IDs are persisted; no messages, file paths or protocol data enter storage. */
export function createToolbar(bar: HTMLElement, items: Item[], translate: Translator, key = "coterm.serial.toolbar.v1") {
  let t = translate;
  let order = items.map(item => item.id);
  const hidden = new Set<string>();
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(key) ?? "null");
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
  const hint = document.createElement("p"); hint.className = "hint";
  const actions = document.createElement("div"); actions.className = "toolbar-edit-actions";
  const reset = document.createElement("button"); reset.type = "button";
  reset.className = "toolbar-reset";
  const done = document.createElement("button"); done.type = "button"; done.className = "toolbar-done";
  const removed = document.createElement("div"); removed.className = "toolbar-removed";
  const trash = document.createElement("div"); trash.className = "toolbar-trash"; trash.hidden = true;
  trash.setAttribute("role", "group");
  trash.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7"/></svg>';
  const trashText = document.createElement("span"); trash.append(trashText);
  actions.append(reset, done); panel.append(hint, actions, removed);
  const controller = new AbortController();
  let editing = false;
  let dragged: string | null = null;
  const clearDrop = () => { for (const slot of slots.values()) delete slot.dataset.dropSide; };
  const finishDrag = () => {
    dragged = null; clearDrop();
    trash.hidden = true; delete trash.dataset.over;
    for (const slot of slots.values()) delete slot.dataset.dragging;
  };
  const save = () => {
    try { localStorage.setItem(key, JSON.stringify({ order, hidden: [...hidden] })); } catch { /* Session layout still works. */ }
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
    panel.setAttribute("aria-label", t("Customize toolbar"));
    hint.textContent = t("Drag to reorder. Drop on the trash to remove.");
    reset.textContent = t("Restore default toolbar"); done.textContent = t("Done");
    trashText.textContent = t("Drop here to remove from toolbar"); trash.setAttribute("aria-label", trashText.textContent);
    removed.replaceChildren(); removed.hidden = hidden.size === 0;
    if (hidden.size) {
      const label = document.createElement("span"); label.textContent = t("Removed:"); removed.append(label);
    }
    order.forEach((id, index) => {
      const item = items.find(value => value.id === id)!;
      const slot = slots.get(id)!;
      slot.draggable = editing; item.element.inert = editing;
      slot.style.setProperty("--wiggle-delay", `${-index * 37}ms`);
      slot.style.setProperty("--wiggle-period", `${260 + index % 3 * 30}ms`);
      if (editing) {
        slot.tabIndex = 0; slot.setAttribute("role", "group");
        slot.setAttribute("aria-label", t("Reorder {item}", { item: t(item.label) }));
        slot.setAttribute("aria-keyshortcuts", "Alt+ArrowLeft Alt+ArrowRight Delete Backspace");
        slot.title = t("Drag to reorder; drop on the trash to remove. Delete removes; Alt + Left / Right moves.");
      } else {
        for (const name of ["tabindex", "role", "aria-label", "aria-keyshortcuts", "title"]) slot.removeAttribute(name);
      }
      if (!hidden.has(id)) return;
      const restore = document.createElement("button"); restore.type = "button"; restore.dataset.restore = id;
      restore.setAttribute("aria-label", t("Add {item} back to the toolbar", { item: t(item.label) }));
      restore.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"/></svg>';
      restore.append(document.createTextNode(t(item.label)));
      restore.addEventListener("click", () => {
        hidden.delete(id); arrange(); save(); render(); slot.focus({ preventScroll: true });
      });
      removed.append(restore);
    });
  };
  const remove = (id: string) => {
    if (!editing || !slots.has(id) || hidden.has(id)) return;
    const visible = order.filter(value => !hidden.has(value));
    const index = visible.indexOf(id);
    hidden.add(id); arrange(); save(); render();
    (slots.get(visible[index + 1] ?? visible[index - 1]) ?? done).focus({ preventScroll: true });
  };
  for (const [id, slot] of slots) {
    slot.addEventListener("dragstart", event => {
      if (!editing || !event.dataTransfer) { event.preventDefault(); return; }
      finishDrag(); dragged = id; slot.dataset.dragging = "true";
      trash.hidden = false;
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("application/x-coterm-serial-toolbar", id);
    }, { signal: controller.signal });
    slot.addEventListener("dragover", event => {
      if (!editing || !dragged) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      clearDrop();
      delete trash.dataset.over;
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
      if (editing && !event.altKey && !event.ctrlKey && !event.metaKey && ["Delete", "Backspace"].includes(event.key)) {
        event.preventDefault(); event.stopPropagation(); remove(id); return;
      }
      if (!editing || !event.altKey || event.ctrlKey || event.metaKey || !["ArrowLeft", "ArrowRight"].includes(event.key)) return;
      event.preventDefault(); event.stopPropagation();
      const visible = order.filter(value => !hidden.has(value));
      const delta = event.key === "ArrowLeft" ? -1 : 1;
      const target = visible[visible.indexOf(id) + delta];
      if (target) move(id, target, delta > 0);
    }, { signal: controller.signal });
  }
  trash.addEventListener("dragover", event => {
    if (!editing || !dragged) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    clearDrop(); trash.dataset.over = "true";
  }, { signal: controller.signal });
  trash.addEventListener("dragleave", () => { delete trash.dataset.over; }, { signal: controller.signal });
  trash.addEventListener("drop", event => {
    if (!editing || !dragged) return;
    event.preventDefault(); const id = dragged; finishDrag(); remove(id);
  }, { signal: controller.signal });
  bar.addEventListener("click", event => {
    if (editing) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, { capture: true, signal: controller.signal });
  const positionPanel = () => panel.style.setProperty("--toolbar-height", `${bar.offsetHeight}px`);
  const observer = new ResizeObserver(positionPanel); observer.observe(bar);
  reset.addEventListener("click", () => { order = items.map(item => item.id); hidden.clear(); arrange(); save(); render(); }, { signal: controller.signal });
  arrange(); render();
  return {
    panel, trash, done,
    setEditing(value: boolean) {
      editing = value; finishDrag(); bar.dataset.customizing = String(value); positionPanel(); render();
    },
    locale(next: Translator) { t = next; render(); },
    unmount() {
      editing = false; finishDrag(); controller.abort(); observer.disconnect();
      for (const item of items) item.element.inert = false;
      for (const slot of slots.values()) slot.draggable = false;
      delete bar.dataset.customizing; panel.remove(); trash.remove();
    },
  };
}
