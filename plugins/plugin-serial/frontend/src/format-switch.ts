export type Format = "text" | "hex";

/** Two-position slider supporting buttons, arrow keys and touch / pointer swipes. */
export function createFormatSwitch(label: string, change?: (next: Format) => boolean) {
  const element = document.createElement("div");
  element.className = "format-switch";
  element.setAttribute("role", "group"); element.setAttribute("aria-label", label);
  const thumb = document.createElement("span"); thumb.className = "format-thumb"; thumb.setAttribute("aria-hidden", "true");
  element.append(thumb);
  const buttons = [document.createElement("button"), document.createElement("button")] as const;
  let value: Format = "text", disabled = false, suppressClick = false;
  let down: { id: number; x: number } | null = null;
  let suppressTimer = 0;
  const render = () => {
    element.dataset.value = value;
    buttons.forEach((button, index) => {
      button.disabled = disabled;
      button.setAttribute("aria-pressed", String(value === (index === 0 ? "text" : "hex")));
    });
  };
  const choose = (next: Format) => {
    if (disabled || next === value || change?.(next) === false) return;
    value = next; render(); element.dispatchEvent(new Event("change"));
  };
  buttons.forEach((button, index) => {
    button.type = "button"; button.textContent = index === 0 ? "ASCII" : "HEX";
    button.dataset.value = index === 0 ? "text" : "hex";
    button.addEventListener("click", () => { if (!suppressClick) choose(index === 0 ? "text" : "hex"); });
    element.append(button);
  });
  element.addEventListener("keydown", event => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault(); choose(event.key === "ArrowLeft" ? "text" : "hex");
    buttons[value === "text" ? 0 : 1].focus();
  });
  element.addEventListener("pointerdown", event => {
    if (disabled || !event.isPrimary || event.button !== 0) return;
    down = { id: event.pointerId, x: event.clientX };
  });
  element.addEventListener("pointermove", event => {
    if (down?.id === event.pointerId && Math.abs(event.clientX - down.x) >= 20) element.setPointerCapture(event.pointerId);
  });
  element.addEventListener("pointercancel", () => { down = null; });
  element.addEventListener("pointerup", event => {
    if (!down || down.id !== event.pointerId) return;
    const delta = event.clientX - down.x;
    down = null;
    if (Math.abs(delta) < 20) return;
    suppressClick = true; choose(delta > 0 ? "hex" : "text");
    window.clearTimeout(suppressTimer);
    suppressTimer = window.setTimeout(() => { suppressClick = false; }, 0);
  });
  render();
  return {
    element, buttons,
    get value() { return value; }, set value(next: Format) { value = next; render(); },
    get disabled() { return disabled; }, set disabled(next: boolean) { disabled = next; render(); },
    label(next: string) { element.setAttribute("aria-label", next); },
    unmount() { window.clearTimeout(suppressTimer); element.remove(); },
  };
}
