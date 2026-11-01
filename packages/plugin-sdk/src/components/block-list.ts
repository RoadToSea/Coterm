import type { BlockListProps, BlockListView, CommandBlock } from "../plugin-api";

const style = `
  .coterm-blocks, .coterm-blocks * { box-sizing: border-box; }
  .coterm-blocks {
    height: 100%;
    min-height: 0;
    min-width: 0;
    overflow: auto;
    padding: 12px;
    color: var(--coterm-text-primary, #cccccc);
    background: var(--coterm-surface, #1e1e1e);
    font: var(--coterm-ui-font-size, 12px) var(--coterm-font-sans, "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif);
  }
  .coterm-block {
    display: grid;
    grid-template-columns: minmax(40px, max-content) minmax(0, 1fr);
    min-width: 0;
    margin-bottom: 8px;
    overflow: hidden;
    border: 1px solid var(--coterm-border-subtle, #2d2d2d);
    border-radius: 6px;
    background: var(--coterm-surface-raised, #252526);
  }
  .coterm-block:last-child { margin-bottom: 0; }
  .coterm-block[data-selected="true"] {
    border-color: var(--coterm-accent-glow, #3794ff);
    background: var(--coterm-accent-active, #094771);
  }
  .coterm-block-gutter {
    display: flex;
    align-items: flex-start;
    justify-content: center;
    min-width: 40px;
    min-height: 40px;
    margin: 0;
    padding: 12px 8px;
    border: 0;
    border-right: 1px solid var(--coterm-border-subtle, #2d2d2d);
    border-radius: 0;
    color: var(--coterm-text-muted, #858585);
    background: transparent;
    font: 11px/20px var(--coterm-font-mono, Consolas, monospace);
    cursor: pointer;
    user-select: none;
  }
  .coterm-block-gutter:hover { background: var(--coterm-surface-hover, #2a2d2e); }
  .coterm-block-gutter:focus-visible {
    outline: 2px solid var(--coterm-focus, #3794ff);
    outline-offset: -3px;
  }
  .coterm-block-gutter:active { transform: translateY(1px); }
  .coterm-block[data-selected="true"] .coterm-block-gutter {
    color: var(--coterm-text-bright, #ffffff);
    box-shadow: inset 3px 0 var(--coterm-accent-glow, #3794ff);
  }
  .coterm-block-content { min-width: 0; }
  .coterm-block-header {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 10px 12px;
  }
  .coterm-block-command, .coterm-block-output {
    min-width: 0;
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    tab-size: 4;
    font: var(--coterm-terminal-font-size, 12px)/1.7 var(--coterm-font-mono, Consolas, monospace);
    user-select: text;
  }
  .coterm-block-command { flex: 1; }
  .coterm-block-output {
    padding: 10px 12px 12px;
    border-top: 1px solid var(--coterm-border-subtle, #2d2d2d);
  }
  .coterm-block-actor {
    flex-shrink: 0;
    padding-top: 2px;
    color: var(--coterm-text-muted, #9da0a3);
    font-size: 10px;
    line-height: 18px;
  }
  .coterm-block-state { color: var(--coterm-text-muted, #9da0a3); font-size: 10px; line-height: 20px; }
  .coterm-block-actor[data-actor="robot"] { color: var(--coterm-accent-glow, #75beff); }
  .coterm-block-status { color: var(--coterm-danger, #f48771); line-height: 20px; }
  .coterm-block-empty {
    margin: 0;
    padding: 20px 12px;
    color: var(--coterm-text-muted, #858585);
    line-height: 1.7;
  }
`;

interface BlockElements {
  row: HTMLElement;
  gutter: HTMLButtonElement;
  command: HTMLElement;
  output: HTMLElement;
  actor: HTMLElement;
  state: HTMLElement;
  status: HTMLElement;
}

function setText(element: HTMLElement, text: string): void {
  // Keep text selection intact while a plugin refreshes an unchanged snapshot.
  if (element.textContent !== text) element.textContent = text;
}

/** A command and its plain-text reply form one message; only its number toggles selection. */
function createBlock(doc: Document, id: string): BlockElements {
  const row = doc.createElement("article");
  row.className = "coterm-block";
  row.setAttribute("role", "listitem");
  row.dataset.blockId = id;

  const gutter = doc.createElement("button");
  gutter.type = "button";
  gutter.className = "coterm-block-gutter";

  const content = doc.createElement("div");
  content.className = "coterm-block-content";
  const header = doc.createElement("div");
  header.className = "coterm-block-header";
  const command = doc.createElement("pre");
  command.className = "coterm-block-command";
  const output = doc.createElement("pre");
  output.className = "coterm-block-output";
  const actor = doc.createElement("span");
  actor.className = "coterm-block-actor";
  const status = doc.createElement("span");
  status.className = "coterm-block-status";
  const state = doc.createElement("span");
  state.className = "coterm-block-state";
  header.append(command, actor, state, status);
  content.append(header, output);
  row.append(gutter, content);
  return { row, gutter, command, output, actor, state, status };
}

function updateBlock(elements: BlockElements, block: CommandBlock, selected: boolean): void {
  elements.row.dataset.selected = String(selected);
  elements.gutter.setAttribute("aria-pressed", String(selected));
  elements.gutter.setAttribute("aria-label", `Block ${block.index}`);
  setText(elements.gutter, String(block.index));
  setText(elements.command, block.command);
  setText(elements.output, block.output ?? "");
  elements.output.hidden = block.output === undefined || block.output.length === 0;
  elements.actor.dataset.actor = block.actor;
  setText(elements.actor, block.actor === "robot" ? "Robot" : "Human");
  setText(elements.state, block.statusText ?? "");
  elements.state.hidden = !block.statusText;
  const failed = block.exitCode != null && block.exitCode !== 0;
  setText(elements.status, failed ? "!" : "");
  elements.status.title = failed ? `Exit code ${block.exitCode}` : "";
  if (failed) elements.status.setAttribute("aria-label", elements.status.title);
  else elements.status.removeAttribute("aria-label");
}

/** Optional shared UI. Plugins own block boundaries, output bounds and selected context. */
export function mountBlockList(container: HTMLElement, initial: BlockListProps): BlockListView {
  const doc = container.ownerDocument;
  const root = doc.createElement("div");
  root.className = "coterm-blocks";
  root.setAttribute("role", "list");
  const empty = doc.createElement("p");
  empty.className = "coterm-block-empty";
  empty.setAttribute("role", "status");
  const sheet = doc.createElement("style");
  sheet.textContent = style;
  container.replaceChildren(sheet, root);

  let props = initial;
  const selected = new Set<string>();
  const elements = new Map<string, BlockElements>();
  let disposed = false;

  const toggle = (id: string) => {
    if (disposed || !elements.has(id)) return;
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
    const block = props.blocks.find((item) => item.id === id);
    const view = elements.get(id);
    if (block && view) updateBlock(view, block, selected.has(id));
    props.onSelectionChange?.(props.blocks.filter((item) => selected.has(item.id)).map((item) => item.id));
  };

  const render = () => {
    root.setAttribute("aria-label", props.ariaLabel ?? "Command blocks");
    const ids = new Set(props.blocks.map((block) => block.id));
    for (const [id, view] of elements) {
      if (ids.has(id)) continue;
      view.row.remove();
      elements.delete(id);
      selected.delete(id);
    }
    if (props.blocks.length === 0) {
      setText(empty, props.emptyText ?? "No command blocks yet.");
      root.append(empty);
      return;
    }
    empty.remove();
    let position = root.firstElementChild;
    for (const block of props.blocks) {
      if (block.selected !== undefined) {
        if (block.selected) selected.add(block.id);
        else selected.delete(block.id);
      }
      let view = elements.get(block.id);
      if (!view) {
        view = createBlock(doc, block.id);
        elements.set(block.id, view);
      }
      updateBlock(view, block, selected.has(block.id));
      // Reuse keyed DOM, and avoid moving unchanged rows: focus, copy selection and scroll survive.
      if (view.row !== position) root.insertBefore(view.row, position);
      position = view.row.nextElementSibling;
    }
  };

  const activate = (event: MouseEvent) => {
    const block = props.blocks.find((item) => elements.get(item.id)?.gutter === event.target);
    if (block) toggle(block.id);
  };
  const navigate = (event: KeyboardEvent) => {
    const index = props.blocks.findIndex((block) => elements.get(block.id)?.gutter === event.target);
    if (index < 0) return;
    const next = event.key === "ArrowDown" ? Math.min(index + 1, props.blocks.length - 1)
      : event.key === "ArrowUp" ? Math.max(index - 1, 0)
      : event.key === "Home" ? 0
      : event.key === "End" ? props.blocks.length - 1
      : null;
    if (next === null) return;
    event.preventDefault();
    elements.get(props.blocks[next].id)?.gutter.focus();
  };
  root.addEventListener("click", activate);
  root.addEventListener("keydown", navigate);
  render();

  return {
    update(next) {
      if (disposed) return;
      props = next;
      render();
    },
    unmount() {
      if (disposed) return;
      disposed = true;
      root.removeEventListener("click", activate);
      root.removeEventListener("keydown", navigate);
      elements.clear();
      selected.clear();
      container.replaceChildren();
    },
  };
}
