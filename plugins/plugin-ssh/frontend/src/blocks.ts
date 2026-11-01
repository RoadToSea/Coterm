import { mountBlockList } from "@coterm/plugin-sdk/components/block-list";
import type { CommandBlock, PluginAPI } from "@coterm/plugin-sdk";
import type { createTranslator } from "./i18n";

type Translate = ReturnType<typeof createTranslator>;
interface SshBlock {
  id: string;
  index: number;
  command: string;
  actor: "human" | "robot";
  channel: "pty" | "exec";
  state: "running" | "completed" | "cancelled" | "timed_out" | "unknown";
  exitCode: number | null;
  output: string;
  stderr: string;
  commandTruncated: boolean;
  outputTruncated: boolean;
}
interface Page {
  epoch: string;
  revision: string;
  dropped: number;
  blocks: SshBlock[];
  more: boolean;
  nextBefore: number | null;
}
const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null;
function isBlock(value: unknown): value is SshBlock {
  return record(value) && typeof value.id === "string" && value.id.length > 0
    && typeof value.index === "number" && Number.isSafeInteger(value.index) && value.index > 0
    && typeof value.command === "string" && typeof value.output === "string" && typeof value.stderr === "string"
    && (value.actor === "human" || value.actor === "robot") && (value.channel === "pty" || value.channel === "exec")
    && typeof value.state === "string" && ["running", "completed", "cancelled", "timed_out", "unknown"].includes(value.state)
    && (value.exitCode === null || (typeof value.exitCode === "number" && Number.isInteger(value.exitCode)))
    && typeof value.commandTruncated === "boolean" && typeof value.outputTruncated === "boolean";
}
function page(value: unknown): Page {
  if (!record(value) || typeof value.epoch !== "string" || typeof value.revision !== "string" || !/^\d+$/.test(value.revision)
    || typeof value.dropped !== "number" || !Number.isSafeInteger(value.dropped) || value.dropped < 0
    || !Array.isArray(value.blocks) || !value.blocks.every(isBlock) || typeof value.more !== "boolean"
    || !(value.nextBefore === null || (typeof value.nextBefore === "number" && Number.isSafeInteger(value.nextBefore) && value.nextBefore > 0))) {
    throw new Error("Invalid SSH command block snapshot");
  }
  return { epoch: value.epoch, revision: value.revision, dropped: value.dropped,
    blocks: value.blocks, more: value.more, nextBefore: value.nextBefore };
}

/** The plugin owns the DTO/refresh adapter; the reusable SDK owns block rendering/selection. */
export function mountSshBlocks(container: HTMLElement, api: PluginAPI, t: Translate) {
  container.classList.add("ssh-blocks-view");
  const style = document.createElement("style");
  style.textContent = `
    .ssh-blocks-view { display:flex; flex-direction:column; flex:1; min-height:0; min-width:0; }
    .ssh-blocks-view[hidden] { display:none; }
    .ssh-blocks-tools { display:flex; flex-wrap:wrap; align-items:center; gap:8px; padding:6px 12px; border-bottom:1px solid var(--coterm-border-subtle,#333); }
    .ssh-blocks-tools span { color:var(--coterm-text-muted,#969696); font-size:11px; }
    .ssh-blocks-list { flex:1; min-height:0; min-width:0; }
    .ssh-blocks-error { padding:6px 12px; }
    .ssh-blocks-error[hidden] { display:none; }
    .ssh-blocks-error p { margin:0 0 6px; }
    .ssh-blocks-compose { display:grid; grid-template-columns:minmax(0,1fr) auto; gap:8px; margin:0; padding:10px 12px; border-top:1px solid var(--coterm-border-subtle,#333); }
    .ssh-blocks-compose label { grid-column:1 / -1; font-size:11px; }
    .ssh-blocks-compose small { grid-column:1 / -1; color:var(--coterm-text-muted,#969696); }
  `;
  const tools = document.createElement("div");
  tools.className = "ssh-blocks-tools";
  const selection = document.createElement("span");
  selection.setAttribute("role", "status");
  const copy = document.createElement("button");
  copy.type = "button";
  const stop = document.createElement("button");
  stop.type = "button";
  const history = document.createElement("span");
  tools.append(selection, copy, stop, history);
  const errors = document.createElement("div");
  errors.className = "ssh-blocks-error";
  errors.hidden = true;
  const error = document.createElement("p");
  error.className = "error";
  error.setAttribute("role", "alert");
  const retry = document.createElement("button");
  retry.type = "button";
  errors.append(error, retry);
  const list = document.createElement("div");
  list.className = "ssh-blocks-list";
  const form = document.createElement("form");
  form.className = "ssh-blocks-compose";
  const caption = document.createElement("label");
  caption.htmlFor = "ssh-block-command";
  const input = document.createElement("input");
  input.id = caption.htmlFor;
  input.maxLength = 4096;
  input.autocomplete = "off";
  input.spellcheck = false;
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "primary";
  const hint = document.createElement("small");
  form.append(caption, input, submit, hint);
  container.append(style, tools, errors, list, form);

  let blocks: SshBlock[] = [];
  let selected: string[] = [];
  let epoch: string | null = null;
  let dropped = 0;
  let active = false;
  let connected = false;
  let disposed = false;
  let submitting = false;
  let cancelling = false;
  let loading = true;
  let fetching = false;
  let dirty = true;
  let timer = 0;
  const controller = new AbortController();
  const failure = (reason: unknown) => {
    if (disposed) return;
    error.textContent = reason instanceof Error ? reason.message : String(reason);
    errors.hidden = false;
  };
  const display = (block: SshBlock): CommandBlock => ({
    id: block.id, index: block.index, command: block.command, actor: block.actor, exitCode: block.exitCode,
    statusText: `${block.channel.toUpperCase()} · ${t(!connected && block.state === "running" ? "unknown"
      : block.state === "timed_out" ? "timed out" : block.state)}`,
    output: [block.output, block.stderr ? `stderr:\n${block.stderr}` : "",
      block.commandTruncated ? t("Command preview truncated.") : "",
      block.outputTruncated ? t("Reply preview truncated; use the terminal or job output for more.") : ""].filter(Boolean).join("\n"),
  });
  const controls = () => {
    selection.textContent = t("{count} blocks selected", { count: selected.length });
    copy.textContent = t("Copy selected");
    copy.disabled = selected.length === 0;
    stop.textContent = t("Stop selected");
    stop.disabled = !connected || cancelling || !blocks.some(block => selected.includes(block.id) && block.state === "running");
    history.textContent = dropped ? t("Older blocks expired: {count}", { count: dropped }) : "";
    caption.textContent = t("Command in shared terminal");
    submit.textContent = t("Run command");
    hint.textContent = t("Single-line commands share cd/export. Use Terminal for interactive programs and passwords.");
    input.disabled = !connected || submitting;
    submit.disabled = !connected || submitting || blocks.some(block => block.channel === "pty" && block.state === "running");
    retry.textContent = t("Retry");
  };
  const props = () => ({ blocks: blocks.map(display), ariaLabel: t("Command blocks"),
    emptyText: t(loading ? "Loading…" : "No command blocks yet. Run a command here or through MCP."),
    onSelectionChange(ids: string[]) { selected = ids; controls(); },
  });
  const view = mountBlockList(list, props());

  const refresh = async () => {
    if (disposed || !active || fetching) return;
    fetching = true;
    dirty = false;
    try {
      let first: Page | undefined;
      const collected: SshBlock[] = [];
      const ids = new Set<string>();
      let before: number | undefined;
      for (let batch = 0; batch < 32; batch++) {
        const receipt = await api.invoke<unknown>("ssh.blocks", before === undefined ? {} : { before });
        if (disposed) return;
        if (receipt.error || receipt.status !== "completed") throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
        const snapshot = page(receipt.output);
        if (first && (snapshot.epoch !== first.epoch || snapshot.revision !== first.revision)) { dirty = true; return; }
        first ??= snapshot;
        for (const block of snapshot.blocks) {
          if (ids.has(block.id) || (before !== undefined && block.index >= before)) throw new Error("Invalid SSH block pagination");
          ids.add(block.id);
          collected.push(block);
          if (collected.length > 32) throw new Error("SSH block snapshot exceeds its bound");
        }
        if (!snapshot.more) break;
        if (snapshot.nextBefore === null || !snapshot.blocks.length || (before !== undefined && snapshot.nextBefore >= before) || batch === 31) {
          throw new Error("Invalid SSH block pagination");
        }
        before = snapshot.nextBefore;
      }
      if (!first) return;
      if (epoch !== first.epoch) selected = [];
      epoch = first.epoch;
      dropped = first.dropped;
      blocks = collected.sort((a, b) => a.index - b.index);
      selected = selected.filter(id => ids.has(id));
      loading = false;
      errors.hidden = true;
      view.update({ ...props(), blocks: blocks.map(block => ({ ...display(block), selected: selected.includes(block.id) })) });
      controls();
    } catch (reason) { loading = false; view.update(props()); failure(reason); }
    finally {
      fetching = false;
      if (dirty && !disposed && active) schedule();
    }
  };
  const schedule = () => {
    if (disposed || !active || timer || fetching) return;
    timer = window.setTimeout(() => { timer = 0; void refresh(); }, 100);
  };
  const invalidate = () => { if (!disposed) { dirty = true; schedule(); } };
  retry.addEventListener("click", invalidate, { signal: controller.signal });
  form.addEventListener("submit", async event => {
    event.preventDefault();
    if (disposed || submit.disabled || !input.value.trim()) return;
    const text = input.value;
    submitting = true;
    controls();
    try {
      const receipt = await api.invoke("ssh.command", { text }, { waitTimeoutMs: 0 });
      if (disposed) return;
      if (receipt.error || ["failed", "rejected", "cancelled", "unknown"].includes(receipt.status)) throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
      if (input.value === text) input.value = "";
      invalidate();
    } catch (reason) { failure(reason); }
    finally { if (!disposed) { submitting = false; controls(); input.focus(); } }
  }, { signal: controller.signal });
  copy.addEventListener("click", () => {
    const text = blocks.filter(block => selected.includes(block.id)).map(block => {
      const shown = display(block);
      return `${shown.command}\n${shown.output ?? ""}`;
    }).join("\n\n");
    if (!navigator.clipboard?.writeText) { failure(t("Clipboard unavailable. Select and copy the block text.")); return; }
    void navigator.clipboard.writeText(text).catch(failure);
  }, { signal: controller.signal });
  stop.addEventListener("click", async () => {
    if (disposed || stop.disabled) return;
    const targets = blocks.filter(block => selected.includes(block.id) && block.state === "running");
    cancelling = true;
    controls();
    try {
      for (const block of targets) {
        if (disposed) return;
        const receipt = await api.cancelOperation(block.id);
        if (receipt.error) throw new Error(receipt.error.message);
      }
      invalidate();
    } catch (reason) { failure(reason); }
    finally { if (!disposed) { cancelling = false; controls(); } }
  }, { signal: controller.signal });
  controls();
  return {
    invalidate,
    setActive(value: boolean) { active = value; if (active && dirty) void refresh(); },
    setConnected(value: boolean) { connected = value; controls(); view.update(props()); },
    refreshLocale() { controls(); view.update(props()); },
    unmount() { disposed = true; controller.abort(); window.clearTimeout(timer); view.unmount(); container.replaceChildren(); },
  };
}
