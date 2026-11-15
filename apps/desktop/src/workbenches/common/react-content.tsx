import { createRoot, type Root } from "react-dom/client";
import type { ReactNode } from "react";

interface LeafRoot { root: Root; owner: symbol; retiring: boolean }
const roots = new WeakMap<HTMLElement, LeafRoot>();

/** Render into a controller's leaf only; disposal ignores late asynchronous updates. */
export function mountReactContent(container: HTMLElement) {
  const previous = roots.get(container);
  if (previous && !previous.retiring) throw new Error("React leaf is already mounted");
  const owner = Symbol();
  const entry = previous ?? { root: createRoot(container), owner, retiring: false };
  entry.owner = owner;
  entry.retiring = false;
  roots.set(container, entry);
  let disposed = false;
  return {
    render(content: ReactNode) { if (!disposed) entry.root.render(content); },
    unmount() {
      if (disposed) return;
      disposed = true;
      entry.retiring = true;
      // Effect cleanup may run inside a commit, followed by setup on the same leaf.
      queueMicrotask(() => {
        if (entry.owner !== owner || !entry.retiring) return;
        entry.root.unmount();
        roots.delete(container);
      });
    },
  };
}
