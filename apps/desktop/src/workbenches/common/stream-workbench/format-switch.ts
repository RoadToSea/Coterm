export type Format = "text" | "hex";

/** Two-position slider supporting buttons, arrow keys and touch / pointer swipes. */
export function createFormatSwitch(element: HTMLElement, label: string, change?: (next: Format) => boolean) {
  element.setAttribute("aria-label", label);
  const buttons = Array.from(element.querySelectorAll<HTMLButtonElement>("button"));
  const controller = new AbortController();
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
    button.addEventListener("click", () => { if (!suppressClick) choose(index === 0 ? "text" : "hex"); }, { signal: controller.signal });
  });
  element.addEventListener("keydown", event => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault(); choose(event.key === "ArrowLeft" ? "text" : "hex");
    buttons[value === "text" ? 0 : 1].focus();
  }, { signal: controller.signal });
  element.addEventListener("pointerdown", event => {
    if (disabled || !event.isPrimary || event.button !== 0) return;
    down = { id: event.pointerId, x: event.clientX };
  }, { signal: controller.signal });
  element.addEventListener("pointermove", event => {
    if (down?.id === event.pointerId && Math.abs(event.clientX - down.x) >= 20) element.setPointerCapture(event.pointerId);
  }, { signal: controller.signal });
  element.addEventListener("pointercancel", () => { down = null; }, { signal: controller.signal });
  element.addEventListener("pointerup", event => {
    if (!down || down.id !== event.pointerId) return;
    const delta = event.clientX - down.x;
    down = null;
    if (Math.abs(delta) < 20) return;
    suppressClick = true; choose(delta > 0 ? "hex" : "text");
    window.clearTimeout(suppressTimer);
    suppressTimer = window.setTimeout(() => { suppressClick = false; }, 0);
  }, { signal: controller.signal });
  render();
  return {
    element, buttons,
    get value() { return value; }, set value(next: Format) { value = next; render(); },
    get disabled() { return disabled; }, set disabled(next: boolean) { disabled = next; render(); },
    label(next: string) { element.setAttribute("aria-label", next); },
    unmount() { window.clearTimeout(suppressTimer); controller.abort(); },
  };
}
