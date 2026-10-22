import type { BlockListProps, BlockListView, CommandBlock } from "../plugin-api";

const style = `
  .coterm-blocks {
    height: 100%;
    overflow: auto;
    color: var(--coterm-text-primary, #cccccc);
    background: var(--coterm-surface, #1e1e1e);
    font: 12px var(--coterm-font-sans, "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif);
  }
  .coterm-block {
    display: grid;
    grid-template-columns: 34px minmax(0, 1fr) auto auto;
    min-height: 28px;
    align-items: center;
    border: 0;
    border-bottom: 1px solid var(--coterm-border-subtle, #2d2d2d);
    padding: 0;
    width: 100%;
    color: inherit;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .coterm-block:hover { background: var(--coterm-surface-hover, #2a2d2e); }
  .coterm-block[aria-selected="true"] { background: var(--coterm-accent-active, #094771); }
  .coterm-block-gutter {
    position: relative;
    display: flex;
    height: 100%;
    align-items: center;
    justify-content: center;
    color: var(--coterm-text-muted, #858585);
    font: 11px var(--coterm-font-mono, Consolas, monospace);
  }
  .coterm-block[aria-selected="true"] .coterm-block-gutter { color: var(--coterm-text-bright, #ffffff); }
  .coterm-block[aria-selected="true"] .coterm-block-gutter::before {
    position: absolute;
    inset: 0 auto 0 0;
    width: 3px;
    background: var(--coterm-accent-glow, #3794ff);
    content: "";
  }
  .coterm-block-command {
    overflow: hidden;
    padding: 5px 8px;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 12px var(--coterm-font-mono, Consolas, monospace);
  }
  .coterm-block-actor { margin-right: 8px; color: var(--coterm-text-muted, #9da0a3); font-size: 10px; }
  .coterm-block-actor[data-actor="robot"] { color: var(--coterm-accent-glow, #75beff); }
  .coterm-block-status { min-width: 20px; color: var(--coterm-danger, #f48771); text-align: center; }
`;

function createRow(block: CommandBlock, selected: boolean, toggle: () => void): HTMLButtonElement {
  const row = document.createElement("button");
  row.type = "button";
  row.className = "coterm-block";
  row.setAttribute("aria-pressed", String(selected));
  row.title = block.command;
  row.addEventListener("click", toggle);

  const gutter = document.createElement("span");
  gutter.className = "coterm-block-gutter";
  gutter.textContent = String(block.index);

  const command = document.createElement("span");
  command.className = "coterm-block-command";
  command.textContent = block.command;

  const actor = document.createElement("span");
  actor.className = "coterm-block-actor";
  actor.dataset.actor = block.actor;
  actor.textContent = block.actor === "robot" ? "Robot" : "Human";

  const status = document.createElement("span");
  status.className = "coterm-block-status";
  status.textContent = block.exitCode != null && block.exitCode !== 0 ? "!" : "";
  if (status.textContent) status.title = `Exit code ${block.exitCode}`;

  row.append(gutter, command, actor, status);
  return row;
}

/** Optional host-rendered block selector; block boundaries and source come from the plugin. */
export function mountBlockList(container: HTMLElement, initial: BlockListProps): BlockListView {
  const root = document.createElement("div");
  root.className = "coterm-blocks";
  root.setAttribute("role", "list");
  root.setAttribute("aria-label", "Command blocks");
  const sheet = document.createElement("style");
  sheet.textContent = style;
  container.replaceChildren(sheet, root);

  let props = initial;
  let selected = new Set(initial.blocks.filter((block) => block.selected).map((block) => block.id));
  let disposed = false;
  const render = () => {
    if (disposed) return;
    const fragment = document.createDocumentFragment();
    for (const block of props.blocks) {
      fragment.append(createRow(block, selected.has(block.id), () => {
        if (selected.has(block.id)) selected.delete(block.id);
        else selected.add(block.id);
        render();
        props.onSelectionChange?.([...selected]);
      }));
    }
    root.replaceChildren(fragment);
  };
  render();

  return {
    update(next) {
      props = next;
      selected = new Set(next.blocks.filter((block) => block.selected).map((block) => block.id));
      render();
    },
    unmount() {
      disposed = true;
      container.replaceChildren();
    },
  };
}
