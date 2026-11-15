import type { BlockListProps, BlockListView } from "../../../../shared/driver-api";
import { Button } from "../../components/ui/button";
import { mountReactContent } from "./react-content";

/** Keyed replies preserve text selection and focus when another block changes. */
export function mountBlockList(container: HTMLElement, initial: BlockListProps): BlockListView {
  const view = mountReactContent(container);
  let props = initial;
  let disposed = false;
  const selected = new Set<string>();
  const toggle = (id: string) => {
    if (disposed || !props.blocks.some(block => block.id === id)) return;
    if (selected.has(id)) selected.delete(id); else selected.add(id);
    render();
    props.onSelectionChange?.(props.blocks.filter(block => selected.has(block.id)).map(block => block.id));
  };
  const render = () => view.render(<div className="coterm-blocks" role="list" aria-label={props.ariaLabel ?? "Command blocks"}>
    {!props.blocks.length && <p className="coterm-block-empty" role="status">{props.emptyText ?? "No command blocks yet."}</p>}
    {props.blocks.map((block, index) => <article key={block.id} className="coterm-block" role="listitem" data-block-id={block.id} data-selected={String(selected.has(block.id))}>
      <Button variant="ghost" className="coterm-block-gutter" size="sm" aria-label={`Block ${block.index}`} aria-pressed={selected.has(block.id)} onClick={() => toggle(block.id)}
        onKeyDown={event => {
          const next = event.key === "ArrowDown" ? Math.min(index + 1, props.blocks.length - 1) : event.key === "ArrowUp" ? Math.max(index - 1, 0)
            : event.key === "Home" ? 0 : event.key === "End" ? props.blocks.length - 1 : null;
          if (next === null) return;
          event.preventDefault(); container.querySelectorAll<HTMLButtonElement>(".coterm-block-gutter")[next]?.focus();
        }}>{block.index}</Button>
      <div className="coterm-block-content"><div className="coterm-block-header">
        <pre className="coterm-block-command">{block.command}</pre>
        <span className="coterm-block-actor" data-actor={block.actor}>{block.actor === "robot" ? "Robot" : "Human"}</span>
        <span className="coterm-block-state" hidden={!block.statusText}>{block.statusText ?? ""}</span>
        <span className="coterm-block-status" title={block.exitCode ? `Exit code ${block.exitCode}` : ""} aria-label={block.exitCode ? `Exit code ${block.exitCode}` : undefined}>{block.exitCode ? "!" : ""}</span>
      </div><pre className="coterm-block-output" hidden={!block.output}>{block.output ?? ""}</pre></div>
    </article>)}
  </div>);
  const update = (next: BlockListProps) => {
    if (disposed) return;
    props = next;
    const ids = new Set(props.blocks.map(block => block.id));
    for (const id of selected) if (!ids.has(id)) selected.delete(id);
    for (const block of props.blocks) {
      if (block.selected === true) selected.add(block.id);
      else if (block.selected === false) selected.delete(block.id);
    }
    render();
  };
  update(initial);
  return { update, unmount() { disposed = true; selected.clear(); view.unmount(); } };
}
