import { useLayoutEffect, useRef, useState, type PointerEvent, type RefObject } from "react";

interface Target { id: string; after: boolean }
interface Gesture {
  id: string; pointer: number; element: HTMLElement; x: number; y: number;
  active: boolean; timer: ReturnType<typeof setTimeout>;
}
interface Options {
  editing: boolean; bar: RefObject<HTMLElement>; trash: RefObject<HTMLElement>;
  move(id: string, target: string, after: boolean): void;
  remove(id: string): void;
}

/** Pointer capture keeps a lifted control attached to the mouse until release or cancellation. */
export function useToolbarDrag(options: Options) {
  const latest = useRef(options); latest.current = options;
  const gesture = useRef<Gesture | null>(null);
  const target = useRef<Target | null>(null), trashHit = useRef(false);
  const [dragged, setDragged] = useState<string | null>(null);
  const [drop, setDrop] = useState<Target | null>(null);
  const [overTrash, setOverTrash] = useState(false);
  const finish = () => {
    const current = gesture.current;
    gesture.current = null;
    if (current) {
      clearTimeout(current.timer);
      current.element.style.removeProperty("transform");
      if (current.element.hasPointerCapture(current.pointer)) current.element.releasePointerCapture(current.pointer);
    }
    target.current = null; trashHit.current = false;
    setDragged(null); setDrop(null); setOverTrash(false);
  };
  const lift = (current: Gesture) => {
    current.active = true; clearTimeout(current.timer);
    setDragged(current.id);
  };
  const locate = (x: number, y: number, id: string) => {
    const contains = (rect: DOMRect) => x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
    trashHit.current = !!latest.current.trash.current && contains(latest.current.trash.current.getBoundingClientRect());
    setOverTrash(trashHit.current);
    let next: Target | null = null;
    if (!trashHit.current) {
      for (const item of latest.current.bar.current?.querySelectorAll<HTMLElement>("[data-control]:not([hidden])") ?? []) {
        if (item.dataset.control === id) continue;
        const rect = item.getBoundingClientRect();
        if (contains(rect)) { next = { id: item.dataset.control!, after: x >= rect.left + rect.width / 2 }; break; }
      }
    }
    if (target.current?.id !== next?.id || target.current?.after !== next?.after) setDrop(next);
    target.current = next;
  };
  useLayoutEffect(() => {
    if (!options.editing) finish();
    const cancel = (event: KeyboardEvent) => { if (event.key === "Escape") finish(); };
    document.addEventListener("keydown", cancel, true);
    return () => { document.removeEventListener("keydown", cancel, true); finish(); };
  }, [options.editing]);
  return { dragged, drop, overTrash,
    handlers: {
      onPointerDown(event: PointerEvent<HTMLElement>) {
        if (!latest.current.editing || event.button !== 0 || !event.isPrimary || gesture.current) return;
        event.preventDefault();
        const element = event.currentTarget;
        element.focus(); element.setPointerCapture(event.pointerId);
        const current: Gesture = { id: element.dataset.control!, pointer: event.pointerId, element,
          x: event.clientX, y: event.clientY, active: false,
          timer: setTimeout(() => { if (gesture.current === current) lift(current); }, 280) };
        gesture.current = current;
      },
      onPointerMove(event: PointerEvent<HTMLElement>) {
        const current = gesture.current;
        if (!current || current.pointer !== event.pointerId) return;
        const dx = event.clientX - current.x, dy = event.clientY - current.y;
        if (!current.active && Math.hypot(dx, dy) < 5) return;
        if (!current.active) lift(current);
        current.element.style.transform = `translate3d(${dx}px, ${dy}px, 0)`;
        locate(event.clientX, event.clientY, current.id);
      },
      onPointerUp(event: PointerEvent<HTMLElement>) {
        const current = gesture.current;
        if (!current || current.pointer !== event.pointerId) return;
        if (current.active) {
          locate(event.clientX, event.clientY, current.id);
          if (trashHit.current) latest.current.remove(current.id);
          else if (target.current) latest.current.move(current.id, target.current.id, target.current.after);
        }
        finish();
      },
      onPointerCancel: finish,
      onLostPointerCapture: finish,
    },
  };
}
