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
  const list = document.createElement("div"); list.className = "toolbar-settings-list";
  const reset = document.createElement("button"); reset.type = "button";
  panel.append(title, list, reset);
  const controller = new AbortController();
  const save = () => {
    try { localStorage.setItem(KEY, JSON.stringify({ order, hidden: [...hidden] })); } catch { /* Session layout still works. */ }
  };
  const arrange = () => {
    for (const id of order) {
      const slot = slots.get(id)!; slot.hidden = hidden.has(id); bar.append(slot);
    }
  };
  const move = (id: string, target: number) => {
    const index = order.indexOf(id);
    if (index < 0 || target < 0 || target >= order.length || index === target) return;
    order.splice(index, 1); order.splice(target, 0, id); arrange(); save(); render();
    list.querySelector<HTMLInputElement>(`[data-item="${id}"] input`)?.focus();
  };
  const render = () => {
    panel.setAttribute("aria-label", t("Customize toolbar")); title.textContent = t("Customize toolbar");
    reset.textContent = t("Restore default toolbar"); list.replaceChildren();
    order.forEach((id, index) => {
      const item = items.find(value => value.id === id)!;
      const row = document.createElement("div"); row.className = "toolbar-setting"; row.dataset.item = id; row.draggable = true;
      const label = document.createElement("label"), check = document.createElement("input"), text = document.createElement("span");
      check.type = "checkbox"; check.checked = !hidden.has(id); text.textContent = t(item.label);
      check.addEventListener("change", () => {
        if (check.checked) hidden.delete(id); else hidden.add(id);
        arrange(); save();
      });
      label.append(check, text); row.append(label);
      for (const delta of [-1, 1]) {
        const button = document.createElement("button"); button.type = "button";
        button.className = "icon-button"; button.disabled = index + delta < 0 || index + delta >= order.length;
        button.setAttribute("aria-label", t(delta < 0 ? "Move {item} left" : "Move {item} right", { item: t(item.label) }));
        button.innerHTML = delta < 0
          ? '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14 6-6 6 6 6"/></svg>'
          : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m10 6 6 6-6 6"/></svg>';
        button.addEventListener("click", () => move(id, index + delta)); row.append(button);
      }
      row.addEventListener("dragstart", event => { event.dataTransfer?.setData("text/plain", id); });
      row.addEventListener("dragover", event => event.preventDefault());
      row.addEventListener("drop", event => {
        event.preventDefault(); const from = event.dataTransfer?.getData("text/plain");
        if (from && order.includes(from)) move(from, index);
      });
      list.append(row);
    });
  };
  reset.addEventListener("click", () => { order = items.map(item => item.id); hidden.clear(); arrange(); save(); render(); }, { signal: controller.signal });
  arrange(); render();
  return {
    panel,
    locale(next: Translator) { t = next; render(); },
    unmount() { controller.abort(); panel.remove(); },
  };
}
