import type { CommandBlock, PluginAPI } from "@coterm/plugin-sdk";
import type { createTranslator } from "./i18n";
export interface SshBlock extends CommandBlock {
  channel: "pty" | "exec";
  state: "running" | "completed" | "cancelled" | "timed_out" | "unknown";
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
/** Protocol adapter; trusted snapshots do not reconstruct or replace the terminal. */
export function mountSshBlocks(container: HTMLElement, api: PluginAPI, t: ReturnType<typeof createTranslator>, onBlocks: (blocks: SshBlock[]) => void, onEpoch: () => void) {
  const errors = document.createElement("div");
  errors.className = "ssh-blocks-error";
  errors.hidden = true;
  const error = document.createElement("span");
  error.className = "error";
  error.setAttribute("role", "alert");
  const retry = document.createElement("button");
  retry.type = "button";
  errors.append(error, retry);
  const history = document.createElement("div");
  history.className = "ssh-block-history";
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
  form.append(caption, input, submit);
  container.append(errors, history, form);
  let blocks: SshBlock[] = [];
  let epoch: string | null = null;
  let selected: string[] = [];
  let dropped = 0;
  let connected = false;
  let disposed = false;
  let submitting = false;
  let fetching = false;
  let syncing = false;
  let selectionDirty = false;
  let dirty = true;
  let timer = 0;
  const controller = new AbortController();
  const failure = (reason: unknown) => { if (!disposed) {
    error.textContent = reason instanceof Error ? reason.message : String(reason);
    errors.hidden = false;
  } };
  const controls = () => {
    retry.textContent = t("Retry");
    caption.textContent = t("Command in shared terminal");
    submit.textContent = t("Run command");
    input.disabled = !connected || submitting;
    submit.disabled = !connected || submitting || blocks.some(block => block.channel === "pty" && block.state === "running");
    history.textContent = dropped ? t("Older blocks expired: {count}", { count: dropped }) : "";
  };
  const syncSelection = async () => {
    if (disposed || !epoch || syncing || !selectionDirty)
      return;
    syncing = true;
    try {
      while (selectionDirty && !disposed) {
        selectionDirty = false;
        const receipt = await api.invoke("ssh.select_blocks", { epoch, ids: [...selected] });
        if (receipt.error || receipt.status !== "completed")
          throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
      }
    }
    catch (reason) {
      selectionDirty = true;
      failure(reason);
    }
    finally {
      syncing = false;
    }
  };
  const refresh = async () => {
    if (disposed || fetching)
      return;
    fetching = true;
    dirty = false;
    try {
      let first: Page | undefined;
      const collected: SshBlock[] = [];
      const ids = new Set<string>();
      let before: number | undefined;
      for (let batch = 0; batch < 32; batch++) {
        const receipt = await api.invoke<unknown>("ssh.blocks", before === undefined ? {} : { before });
        if (disposed)
          return;
        if (receipt.error || receipt.status !== "completed")
          throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
        const snapshot = page(receipt.output);
        if (first && (snapshot.epoch !== first.epoch || snapshot.revision !== first.revision)) {
          dirty = true;
          return;
        }
        first ??= snapshot;
        for (const block of snapshot.blocks) {
          if (ids.has(block.id) || (before !== undefined && block.index >= before))
            throw new Error("Invalid SSH block pagination");
          ids.add(block.id);
          collected.push(block);
          if (collected.length > 32)
            throw new Error("SSH block snapshot exceeds its bound");
        }
        if (!snapshot.more)
          break;
        if (snapshot.nextBefore === null || !snapshot.blocks.length || (before !== undefined && snapshot.nextBefore >= before) || batch === 31)
          throw new Error("Invalid SSH block pagination");
        before = snapshot.nextBefore;
      }
      if (!first)
        return;
      const previousEpoch = epoch;
      epoch = first.epoch;
      if (previousEpoch !== null && previousEpoch !== epoch) {
        selected = [];
        selectionDirty = true;
        onEpoch();
      }
      dropped = first.dropped;
      blocks = collected.sort((a, b) => a.index - b.index);
      const kept = selected.filter(id => ids.has(id));
      if (kept.length !== selected.length)
        selectionDirty = true;
      selected = kept;
      errors.hidden = true;
      onBlocks(blocks);
      controls();
      void syncSelection();
    }
    catch (reason) {
      failure(reason);
    }
    finally {
      fetching = false;
      if (dirty && !disposed)
        schedule();
    }
  };
  const schedule = () => {
    if (disposed || timer || fetching)
      return;
    timer = window.setTimeout(() => { timer = 0; void refresh(); }, 100);
  };
  const invalidate = () => { if (!disposed) {
    dirty = true;
    schedule();
  } };
  retry.addEventListener("click", () => { invalidate(); void syncSelection(); }, { signal: controller.signal });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (disposed || submit.disabled || !input.value.trim())
      return;
    const text = input.value;
    submitting = true;
    controls();
    try {
      const receipt = await api.invoke("ssh.command", { text }, { waitTimeoutMs: 0 });
      if (disposed)
        return;
      if (receipt.error || ["failed", "rejected", "cancelled", "unknown"].includes(receipt.status))
        throw new Error(receipt.error?.message ?? `Operation ${receipt.status}`);
      if (input.value === text)
        input.value = "";
      invalidate();
    }
    catch (reason) {
      failure(reason);
    }
    finally {
      if (!disposed) {
        submitting = false;
        controls();
        input.focus();
      }
    }
  }, { signal: controller.signal });
  controls();
  // Wait until mount has installed the terminal and its callbacks.
  schedule();
  return {
    async command(id: string) {
      const existing = blocks.find(block => block.id === id);
      if (existing) return existing.command;
      const receipt = await api.invoke<unknown>("ssh.blocks", {});
      if (disposed || receipt.error || receipt.status !== "completed") return "";
      const snapshot = page(receipt.output);
      if (epoch !== null && snapshot.epoch !== epoch) return "";
      return snapshot.blocks.find(block => block.id === id)?.command ?? "";
    },
    invalidate,
    select(ids: string[]) { selected = ids; selectionDirty = true; void syncSelection(); },
    setConnected(value: boolean) { connected = value; controls(); },
    refreshLocale: controls,
    unmount() {
      disposed = true;
      controller.abort();
      window.clearTimeout(timer);
      if (epoch)
        void api.invoke("ssh.select_blocks", { epoch, ids: [] }).catch(() => { });
      container.replaceChildren();
    },
  };
}
