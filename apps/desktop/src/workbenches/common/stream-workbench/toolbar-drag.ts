import { useLayoutEffect, useRef, useState, type PointerEvent, type RefObject } from "react";

interface Gesture {
  id: string; pointer: number; element: HTMLElement; x: number; y: number;
  offsetX: number; offsetY: number; active: boolean; timer: ReturnType<typeof setTimeout>;
}
interface Options {
  editing: boolean; order: string[]; bar: RefObject<HTMLElement>; trash: RefObject<HTMLElement>;
  commit(order: string[]): void;
  remove(id: string): void;
}
const duration = () => matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 180;

/** Preview reordering uses real flex geometry; only release commits the layout. Cancellation restores it. */
export function useToolbarDrag(options: Options) {
  const latest = useRef(options); latest.current = options;
  const gesture = useRef<Gesture | null>(null), trashHit = useRef(false);
  const positions = useRef(new Map<HTMLElement, DOMRect>());
  const animations = useRef(new Map<HTMLElement, Animation>());
  const preview = useRef<string[] | null>(null);
  const [order, setOrder] = useState<string[] | null>(null);
  const [dragged, setDragged] = useState<string | null>(null);
  const [overTrash, setOverTrash] = useState(false);
  const items = () => [...latest.current.bar.current?.querySelectorAll<HTMLElement>("[data-control]:not([hidden])") ?? []];
  const snapshot = () => { positions.current = new Map(items().map(item => [item, item.getBoundingClientRect()])); };
  const animate = (element: HTMLElement, from: string, to: string) => {
    animations.current.get(element)?.cancel();
    const animation = element.animate([{ transform: from }, { transform: to }],
      { duration: duration(), easing: "cubic-bezier(.2,.8,.2,1)" });
    animations.current.set(element, animation);
    animation.onfinish = () => { if (animations.current.get(element) === animation) animations.current.delete(element); };
  };
  const follow = (current: Gesture) => {
    const rect = current.element.getBoundingClientRect();
    const control = current.element.firstElementChild as HTMLElement;
    control.style.transform = `translate3d(${current.x - current.offsetX - rect.left}px, ${current.y - current.offsetY - rect.top}px, 0)`;
  };
  const finish = (commit = false) => {
    const current = gesture.current;
    if (!current) return;
    const control = current.element.firstElementChild as HTMLElement;
    const from = control.getBoundingClientRect();
    if (current.active) snapshot();
    gesture.current = null;
    clearTimeout(current.timer);
    if (commit && current.active) {
      if (trashHit.current) latest.current.remove(current.id);
      else if (preview.current) latest.current.commit(preview.current);
    }
    control.style.removeProperty("transform");
    preview.current = null; setOrder(null); setDragged(null); setOverTrash(false);
    trashHit.current = false;
    if (latest.current.bar.current?.hasPointerCapture(current.pointer)) latest.current.bar.current.releasePointerCapture(current.pointer);
    // Final geometry is available after React restores the committed or cancelled order.
    requestAnimationFrame(() => {
      if (!current.active || !control.isConnected || !control.getClientRects().length || current.element.hidden || gesture.current) return;
      const to = control.getBoundingClientRect();
      animate(control, `translate(${from.left - to.left}px, ${from.top - to.top}px)`, "translate(0, 0)");
    });
  };
  const lift = (current: Gesture) => {
    current.active = true; clearTimeout(current.timer);
    preview.current = [...latest.current.order];
    setDragged(current.id); follow(current);
  };
  const locate = (current: Gesture) => {
    const contains = (rect: DOMRect) => current.x >= rect.left && current.x <= rect.right && current.y >= rect.top && current.y <= rect.bottom;
    trashHit.current = !!latest.current.trash.current && contains(latest.current.trash.current.getBoundingClientRect());
    setOverTrash(trashHit.current);
    if (trashHit.current) return;
    const bar = latest.current.bar.current!.getBoundingClientRect();
    for (const item of items()) {
      // Layout offsets ignore the animation currently moving a neighbour out of the way.
      const rect = new DOMRect(bar.left + item.offsetLeft, bar.top + item.offsetTop, item.offsetWidth, item.offsetHeight);
      if (!contains(rect)) continue;
      if (item === current.element) return;
      const next = (preview.current ?? latest.current.order).filter(id => id !== current.id);
      next.splice(next.indexOf(item.dataset.control!) + Number(current.x >= rect.left + rect.width / 2), 0, current.id);
      if (next.join() !== preview.current?.join()) { snapshot(); preview.current = next; setOrder(next); }
      return;
    }
  };
  useLayoutEffect(() => {
    for (const [item, before] of positions.current) {
      animations.current.get(item)?.cancel();
      if (item === gesture.current?.element || item.hidden) continue;
      const after = item.getBoundingClientRect();
      if (before.left !== after.left || before.top !== after.top) animate(item,
        `translate(${before.left - after.left}px, ${before.top - after.top}px)`, "translate(0, 0)");
    }
    positions.current.clear();
    if (gesture.current?.active) follow(gesture.current);
  }, [order, options.order]);
  useLayoutEffect(() => {
    if (!options.editing) finish();
    const cancel = (event: KeyboardEvent) => { if (event.key === "Escape") finish(); };
    const blur = () => finish();
    document.addEventListener("keydown", cancel, true); window.addEventListener("blur", blur);
    return () => { document.removeEventListener("keydown", cancel, true); window.removeEventListener("blur", blur); finish(); };
  }, [options.editing]);
  useLayoutEffect(() => () => { for (const animation of animations.current.values()) animation.cancel(); }, []);
  return { dragged, order: order ?? options.order, overTrash,
    onPointerDown(event: PointerEvent<HTMLElement>) {
      if (!latest.current.editing || event.button !== 0 || !event.isPrimary || gesture.current) return;
      event.preventDefault();
      const element = event.currentTarget, rect = element.getBoundingClientRect();
      const control = element.firstElementChild as HTMLElement;
      animations.current.get(control)?.cancel(); element.focus();
      latest.current.bar.current!.setPointerCapture(event.pointerId);
      const current: Gesture = { id: element.dataset.control!, pointer: event.pointerId, element,
        x: event.clientX, y: event.clientY, offsetX: event.clientX - rect.left, offsetY: event.clientY - rect.top,
        active: false, timer: setTimeout(() => { if (gesture.current === current) lift(current); }, 280) };
      gesture.current = current;
    },
    handlers: {
      onPointerMove(event: PointerEvent<HTMLElement>) {
        const current = gesture.current;
        if (!current || current.pointer !== event.pointerId) return;
        if (!current.active && Math.hypot(event.clientX - current.x, event.clientY - current.y) < 5) return;
        current.x = event.clientX; current.y = event.clientY;
        if (!current.active) lift(current);
        follow(current); locate(current);
      },
      onPointerUp(event: PointerEvent<HTMLElement>) { if (gesture.current?.pointer === event.pointerId) finish(true); },
      onPointerCancel() { finish(); },
      onLostPointerCapture() { finish(); },
    },
  };
}
