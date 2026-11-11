export const css = `
  :host {
    display: block;
    height: 100%;
    min-height: 0;
    color: var(--coterm-text-primary, #cccccc);
    font: var(--coterm-ui-font-size, 13px) var(--coterm-font-sans, "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif);
  }
  * { box-sizing: border-box; }
  button, input, select, textarea { font: inherit; color: inherit; }
  form input, form select, form button { border-radius: var(--radius, 8px); }
  button {
    border: 1px solid var(--coterm-border-input, #454545);
    padding: 5px 12px;
    background: var(--coterm-surface-sidebar, #333333);
    cursor: pointer;
    border-radius: var(--radius, 6px);
  }
  button:hover { background: var(--coterm-surface-hover, #414141); }
  button:disabled { opacity: .5; cursor: default; }
  button[aria-pressed="true"] {
    border-color: var(--coterm-focus, #007fd4);
    background: var(--coterm-surface-active, #37373d);
  }
  button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible {
    outline: 1px solid var(--coterm-focus, #007fd4);
  }
  .primary { background: var(--coterm-accent, #0e639c); color: var(--coterm-accent-foreground, #ffffff); }
  .primary:hover { background: var(--coterm-accent-hover, #1177bb); }
  form { display: grid; gap: 14px; margin-bottom: 20px; }
  label { display: grid; gap: 6px; color: var(--coterm-text-subtle, #aaaaaa); }
  input, select, textarea {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--coterm-border-input, #454545);
    background: var(--coterm-surface, #1e1e1e);
    color: var(--coterm-text-primary, #cccccc);
    padding: 7px 9px;
  }
  textarea { resize: none; font-family: var(--coterm-font-mono, Consolas, monospace); min-height: 68px; }
  [hidden] { display: none !important; }
  .pair { display: grid; grid-template-columns: minmax(0, 1fr) 130px; gap: 12px; }
  @media (max-width: 560px) { .pair { grid-template-columns: minmax(0, 1fr); } }
  .triple { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
  .checks { display: flex; flex-wrap: wrap; gap: 16px; }
  .checks label { display: flex; align-items: center; gap: 7px; color: var(--coterm-text-primary, #cccccc); }
  .checks input { width: auto; }
  .error { color: var(--coterm-danger, #f48771); overflow-wrap: anywhere; margin: 0; }
  .hint { color: var(--coterm-text-muted, #969696); font-size: 11px; overflow-wrap: anywhere; margin: 0; }
  .layout { position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; overflow: hidden; background: var(--coterm-surface, #1e1e1e); }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    min-height: 42px;
    border-bottom: 1px solid var(--coterm-border, #333333);
    padding: 6px 14px;
    background: var(--coterm-surface-raised, #252526);
  }
  .bar .spacer { margin-left: auto; }
  .bar button { font-size: 11px; padding: 4px 10px; }
  .bar .connect-button[data-connected="true"], .repeat-button[aria-pressed="true"] {
    border-color: var(--coterm-success, hsl(var(--success, 149 58% 38%)));
    background: color-mix(in srgb, var(--coterm-success, hsl(var(--success, 149 58% 38%))) 26%, var(--coterm-surface-raised, #252526));
  }
  .icon-button { display: inline-grid; place-items: center; min-width: 30px; min-height: 28px; padding: 5px; }
  .icon-button svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 1.5; }
  .format-switch { position: relative; display: inline-grid; grid-template-columns: 1fr 1fr; width: 116px;
    padding: 2px; border: 1px solid var(--coterm-border-input, #454545); border-radius: var(--radius, 6px);
    background: var(--coterm-surface, #1e1e1e); touch-action: pan-y; }
  .format-thumb { position: absolute; inset: 2px auto 2px 2px; width: calc((100% - 4px) / 2);
    border-radius: max(2px, calc(var(--radius, 6px) - 2px)); background: var(--coterm-surface-active, #37373d);
    transition: transform 160ms ease-out; pointer-events: none; }
  .format-switch[data-value="hex"] .format-thumb { transform: translateX(100%); }
  .format-switch button { position: relative; min-height: 25px; padding: 3px 8px; background: transparent; border: 0; border-radius: 4px; font-size: 11px; }
  .format-switch button[aria-pressed="true"] { background: transparent; color: var(--coterm-text-bright, #ffffff); }
  .modes { display: inline-flex; }
  .modes button { border-radius: 0; }
  .modes button:first-child { border-top-left-radius: var(--radius, 6px); border-bottom-left-radius: var(--radius, 6px); }
  .modes button:last-child { border-top-right-radius: var(--radius, 6px); border-bottom-right-radius: var(--radius, 6px); border-left: 0; }
  .panel { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .panel[hidden] { display: none; }
  .receive-area { position: relative; display: flex; flex: 1; min-height: 0; overflow: hidden; isolation: isolate; }
  .settings-panel {
    position: absolute; z-index: 2; inset: 0 0 auto;
    max-height: 100%; overflow: auto; padding: 16px;
    border-bottom: 1px solid var(--coterm-border-input, #454545);
    background: var(--coterm-surface-raised, #252526);
    box-shadow: 0 10px 18px #0003;
    transform: translateY(-102%); opacity: 0; visibility: hidden;
    transition: transform 180ms ease-out, opacity 180ms ease-out, visibility 180ms;
  }
  .settings-panel[data-open="true"] { transform: translateY(0); opacity: 1; visibility: visible; }
  .settings-panel form { margin: 0; }
  .settings-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
  .network-configuration { padding: 16px; }
  .network-configuration .primary, .network-configuration .error { grid-column: 1 / -1; }
  @media (max-width: 560px) {
    .network-configuration { grid-template-columns: minmax(0, 1fr); }
  }
  .settings-grid .signal-check { display: flex; flex-direction: row; align-items: center; gap: 7px; align-self: end; min-height: 34px; }
  .signal-check input { width: auto; }
  .settings-grid [aria-disabled="true"] { opacity: .65; cursor: not-allowed; }
  .settings-grid label:has(.receive-filename) { grid-column: span 2; }
  .receive-newlines { grid-column: 1 / -1; }
  .toolbar-item { position: relative; display: inline-flex; flex: 0 0 auto; align-items: center; }
  .toolbar-item[data-control="stats"] { margin-left: auto; }
  .uart-bar[data-customizing="true"] .toolbar-item { cursor: grab; user-select: none;
    outline: 1px dashed var(--coterm-border-input, #454545); outline-offset: 2px; border-radius: 3px; }
  .uart-bar[data-customizing="true"] .toolbar-item:focus-visible { outline: 2px solid var(--coterm-focus, #007fd4); }
  .uart-bar[data-customizing="true"] .toolbar-item > * { transform-origin: 50% 50%;
    animation: serial-toolbar-wiggle var(--wiggle-period, 280ms) ease-in-out var(--wiggle-delay, 0ms) infinite; will-change: transform; }
  .uart-bar[data-customizing="true"] .toolbar-item[data-dragging="true"] > * { animation-play-state: paused; }
  @keyframes serial-toolbar-wiggle {
    0%, 100% { transform: translate3d(-.4px, 0, 0) rotate(-1.4deg); }
    25% { transform: translate3d(0, -.4px, 0) rotate(.5deg); }
    50% { transform: translate3d(.4px, 0, 0) rotate(1.4deg); }
    75% { transform: translate3d(0, .4px, 0) rotate(-.5deg); }
  }
  .toolbar-item[data-dragging="true"] { opacity: .5; cursor: grabbing; }
  .toolbar-item[data-drop-side]::before { content: ""; position: absolute; top: -3px; bottom: -3px; width: 2px;
    background: var(--coterm-focus, #007fd4); pointer-events: none; }
  .toolbar-item[data-drop-side="before"]::before { left: -5px; }
  .toolbar-item[data-drop-side="after"]::before { right: -5px; }
  .toolbar-customization { position: absolute; z-index: 3; inset: var(--toolbar-height, 0px) 0 auto; width: 100%;
    max-height: calc(100% - var(--toolbar-height, 0px)); overflow: auto; padding: 8px 14px; display: flex; flex-wrap: wrap; align-items: center; gap: 8px;
    border-bottom: 1px solid var(--coterm-border, #333333); background: var(--coterm-surface-raised, #252526); }
  .toolbar-customization > .hint { flex: 1 1 180px; }
  .toolbar-edit-actions { display: flex; flex-wrap: wrap; gap: 6px; margin-left: auto; }
  .toolbar-customization button { font-size: 11px; padding: 4px 8px; }
  .toolbar-removed { flex: 1 1 100%; display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .toolbar-removed > span { color: var(--coterm-text-muted, #969696); font-size: 11px; }
  .toolbar-removed button { display: inline-flex; align-items: center; gap: 5px; }
  .toolbar-removed svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-width: 1.5; }
  .toolbar-trash { position: absolute; z-index: 3; bottom: 16px; left: 50%; transform: translateX(-50%);
    display: flex; align-items: center; gap: 10px; width: max-content; max-width: calc(100% - 28px); min-height: 54px; padding: 12px 18px;
    border: 1px dashed var(--coterm-border-input, #454545); border-radius: var(--radius, 8px);
    background: var(--coterm-surface-raised, #252526); box-shadow: 0 4px 12px #0002; }
  .toolbar-trash > * { pointer-events: none; }
  .toolbar-trash svg { flex: 0 0 22px; width: 22px; height: 22px; fill: none; stroke: currentColor; stroke-width: 1.5; }
  .toolbar-trash[data-over="true"] { color: var(--coterm-danger, #e86464); border-color: currentColor; border-style: solid;
    background: color-mix(in srgb, var(--coterm-danger, #e86464) 13%, var(--coterm-surface-raised, #252526)); }
  .terminal { flex: 1; min-height: 0; overflow: hidden; padding: 10px 4px 0 10px; }
  .uart { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .uart-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--coterm-border, #333333);
    font-size: 11px;
    color: var(--coterm-text-subtle, #aaaaaa);
  }
  .uart-bar .stats { margin-left: auto; color: var(--coterm-text-muted, #969696); font-variant-numeric: tabular-nums; }
  .uart-bar label { display: inline-flex; align-items: center; gap: 5px; color: var(--coterm-text-subtle, #aaaaaa); }
  .uart-bar input { width: auto; }
  .uart-bar button { font-size: 11px; padding: 3px 9px; }
  .uart-output {
    flex: 1;
    min-height: 0;
    margin: 0;
    overflow: auto;
    padding: 8px 14px;
    background: var(--coterm-surface, #1e1e1e);
    color: var(--coterm-text-primary, #cccccc);
    font: var(--coterm-terminal-font-size, 12px)/1.45 var(--coterm-font-mono, Consolas, "Liberation Mono", monospace);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    tab-size: 4;
  }
  .uart-output:empty::before {
    content: attr(data-empty);
    color: var(--coterm-text-disabled, #6c6c6c);
  }
  .uart-send {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 34px minmax(190px, auto);
    gap: 8px;
    align-items: stretch;
    border-top: 1px solid var(--coterm-border, #333333);
    padding: 8px 14px;
  }
  .uart-send label { gap: 4px; }
  .uart-send .entry { position: relative; min-width: 0; }
  .uart-send label.entry { height: 100%; }
  .uart-send .entry textarea { height: 100%; border-radius: var(--radius, 6px); font-size: var(--coterm-terminal-font-size, 13px); }
  .file-actions { display: grid; gap: 4px; grid-template-rows: 1fr 1fr; }
  .file-actions button { display: grid; place-items: center; padding: 4px; }
  .file-actions svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 1.5; }
  .send-controls { display: grid; gap: 8px; align-content: center; }
  .send-controls .inline { gap: 8px; }
  .send-controls button { white-space: nowrap; }
  .send-history { position: absolute; z-index: 3; inset: auto 0 calc(100% + 8px); max-height: 240px; overflow: auto;
    border: 1px solid var(--coterm-border-input, #454545); background: var(--coterm-surface-raised, #252526); padding: 6px;
    border-radius: var(--radius, 6px); box-shadow: 0 -6px 18px #0003; }
  .send-history button { display: block; width: 100%; text-align: left; margin-bottom: 4px; font-family: var(--coterm-font-mono, Consolas, monospace);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .send-history p { margin: 8px; }
  .uart-send .inline { display: inline-flex; align-items: center; gap: 6px; }
  .uart-send .inline input { width: auto; }
  .uart-send input[type="number"] { width: 82px; border-radius: var(--radius, 6px); }
  @container serial-console (max-width: 520px) {
    .uart-send { grid-template-columns: minmax(0, 1fr) 34px; }
    .send-controls { grid-column: 1 / -1; display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; }
    .settings-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .bar { padding-inline: 8px; gap: 6px; }
    .uart-bar { padding-inline: 8px; gap: 6px; }
  }
  .layout { container: serial-console / inline-size; }
  @media (prefers-reduced-motion: reduce) {
    .settings-panel, .format-thumb { transition: none; }
    .uart-bar[data-customizing="true"] .toolbar-item > * { animation: none; will-change: auto; }
  }
`;

export function surface(container: HTMLElement): ShadowRoot {
  // A new host element keeps React StrictMode remounts independent.
  const element = document.createElement("div");
  element.style.height = "100%";
  container.replaceChildren(element);
  const root = element.attachShadow({ mode: "open" });
  const style = document.createElement("style");
  style.textContent = css;
  root.append(style);
  return root;
}

export function option(value: string, label: string): HTMLOptionElement {
  const element = document.createElement("option");
  element.value = value;
  element.textContent = label;
  return element;
}

export function caption(text: string): HTMLLabelElement {
  const label = document.createElement("label");
  const span = document.createElement("span");
  span.textContent = text;
  label.append(span);
  return label;
}

/** 只改标签里的文案节点，语言切换不重建控件。 */
export function retitle(label: HTMLLabelElement, text: string) {
  const span = label.querySelector("span");
  if (span) span.textContent = text;
  else label.prepend(document.createTextNode(text));
}
