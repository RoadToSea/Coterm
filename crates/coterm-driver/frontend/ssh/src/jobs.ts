import type { DriverAPI } from "../../../../../apps/shared/index";
import type { SshBlock } from "./blocks";
import type { createTranslator } from "./i18n";
interface Job {
  preview?: {stdout: string; stderr: string};
  operationId: string;
  text: string;
  index?: number;
  done: boolean;
  exitCode: number | null;
  cancelled: boolean;
  timedOut: boolean;
  stdout: string;
  stderr: string;
  reading: boolean;
}
const css = `
.ssh-workbench { display:flex; flex:1; min-width:0; min-height:0; overflow:hidden; }
.ssh-workbench[data-dock="bottom"] { flex-direction:column; }
.jobs { display:flex; flex:none; gap:8px; padding:6px 10px; min-height:40px; border-top:1px solid var(--coterm-border,#333); background:var(--coterm-surface-raised,#252526); }
.jobs[hidden], .job-view[hidden], .exec-splitter[hidden], .job-follow[hidden] { display:none; }
.jobs-label { flex:none; line-height:26px; font-size:10px; color:var(--coterm-text-muted,#969696); }
.jobs-list { display:flex; flex:1; min-width:0; gap:6px; overflow-x:auto; overflow-y:hidden; scrollbar-width:none; overscroll-behavior:contain; }
.jobs[data-expanded="true"] .jobs-list { flex-wrap:wrap; overflow-x:hidden; overflow-y:auto; max-height:min(180px,30vh); scrollbar-width:thin; }
.jobs-toggle { flex:none; width:26px; height:26px; padding:0; }
.job { display:inline-flex; flex:none; align-items:center; height:26px; max-width:min(280px,100%); padding:0; border:1px solid color-mix(in srgb,var(--coterm-text-primary,#ccc) 18%,transparent); border-radius:14px; background:linear-gradient(180deg,color-mix(in srgb,var(--coterm-surface-raised,#252526) 90%,white),var(--coterm-surface,#1e1e1e)); box-shadow:inset 0 1px 0 rgba(255,255,255,.12); font-size:11px; transition:transform 120ms ease; }
.job[hidden] { display:none; }
.job:active { transform:scale(.97); }
.job[data-selected="true"] { border-color:var(--coterm-focus,#38bdf8); box-shadow:0 0 0 1px var(--coterm-surface,#1e1e1e),0 0 0 2px color-mix(in srgb,var(--coterm-focus,#38bdf8) 55%,transparent),inset 0 1px 0 rgba(255,255,255,.12); }
.job-select { display:flex; align-items:center; gap:7px; height:100%; min-width:0; border:0; padding:0 7px; border-radius:inherit; background:transparent; }
.job .name { max-width:145px; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
.job-led { width:6px; height:6px; flex:none; border-radius:50%; background:#94a3b8; }
.job-led[data-state="running"] { background:#38bdf8; box-shadow:0 0 5px #38bdf866; animation:exec-pulse 1.5s ease-in-out infinite; }
.job-led[data-state="success"] { background:#34d399; box-shadow:0 0 4px #34d39944; }
.job-led[data-state="error"] { background:#f87171; box-shadow:0 0 4px #f8717144; }
.job-block-number { border:0; padding:0 0 0 7px; background:transparent; font:10px var(--coterm-font-mono,monospace); }
.job-block-number[aria-pressed="true"] { color:var(--coterm-focus,#38bdf8); }
.job .dismiss { padding:0 5px; border:0; border-radius:50%; margin-right:3px; background:transparent; opacity:.6; }
.job .dismiss svg { width:10px; height:10px; }
.job-view { display:flex; flex-direction:column; flex:none; min-width:0; min-height:100px; background:var(--coterm-surface,#1e1e1e); }
.job-head { display:flex; flex-wrap:wrap; align-items:center; gap:8px; min-height:36px; padding:6px 10px; border-bottom:1px solid var(--coterm-border,#333); font-size:11px; }
.job-head strong { font-weight:500; }
.job-dock { margin-left:auto; }
.job-head button { padding:2px 6px; }
.job-head .job-close { display:grid; place-items:center; flex:none; width:26px; height:26px; padding:4px; }
.job-close svg { width:14px; height:14px; }
.job-state[data-state="error"], .job-state[data-state="timed out"] { color:var(--coterm-danger,#cf6868); }
.job-follow { display:flex; flex-wrap:wrap; align-items:center; gap:6px; padding:7px 10px; font-size:11px; background:var(--coterm-selection,rgba(56,189,248,.1)); }
.job-follow button { font-size:11px; padding:2px 5px; }
.job-body { flex:1; min-height:0; overflow:auto; padding:8px 10px; }
.job-command-section { margin-bottom:12px; }
.job-command-section h4, .job-stream h4 { margin:0 0 6px; font-size:10px; font-weight:500; color:var(--coterm-text-muted,#969696); }
.job-command, .job-stream pre { margin:0; white-space:pre-wrap; overflow-wrap:anywhere; font:var(--coterm-terminal-font-size,13px)/var(--coterm-terminal-line-height,1.4) var(--coterm-font-mono,Consolas,monospace); }
.job-stream { --stream-color:var(--terminal-blue,#5294cf); border:1px solid color-mix(in srgb,var(--stream-color) 65%,var(--coterm-border,#333)); border-left:3px solid var(--stream-color); border-radius:6px; padding:9px 10px; margin-bottom:10px; background:color-mix(in srgb,var(--stream-color) 5%,var(--coterm-surface,#1e1e1e)); }
.job-stream[data-stream="stderr"] { --stream-color:var(--coterm-danger,#cf6868); }
.job-stream[hidden] { display:none; }
.job-stream h4 { color:var(--stream-color); }
.exec-splitter { flex:none; width:6px; min-height:0; cursor:col-resize; background:var(--coterm-border,#333); touch-action:none; }
.ssh-workbench[data-dock="bottom"] .exec-splitter { width:auto; height:6px; cursor:row-resize; }
.exec-splitter:focus-visible { outline:2px solid var(--coterm-focus,#38bdf8); outline-offset:-2px; }
@keyframes exec-pulse { 50% { opacity:.35; } }
@media(prefers-reduced-motion:reduce) { .job,.job-led { transition:none; animation:none !important; } }
`;
export function mountJobs(workbench: HTMLElement, terminalPane: HTMLElement, jobsBar: HTMLElement, api: DriverAPI, t: ReturnType<typeof createTranslator>, toggleBlock: (id: string) => void, showError: (reason: unknown) => void) {
  const style = document.createElement("style");
  style.textContent = css;
  workbench.append(style);
  const label = document.createElement("span");
  label.className = "jobs-label";
  const list = document.createElement("div");
  list.className = "jobs-list";
  list.setAttribute("role", "list");
  const expand = document.createElement("button");
  expand.type = "button";
  expand.className = "jobs-toggle";
  jobsBar.className = "jobs";
  jobsBar.hidden = true;
  jobsBar.append(label, list, expand);
  const splitter = document.createElement("div");
  splitter.className = "exec-splitter";
  splitter.tabIndex = 0;
  splitter.setAttribute("role", "separator");
  splitter.hidden = true;
  const panel = document.createElement("section");
  panel.className = "job-view";
  panel.tabIndex = -1;
  panel.hidden = true;
  const head = document.createElement("div");
  head.className = "job-head";
  const title = document.createElement("strong");
  title.textContent = "Exec";
  const state = document.createElement("span");
  state.className = "job-state";
  state.setAttribute("role", "status");
  const dockButton = document.createElement("button");
  dockButton.type = "button";
  dockButton.className = "job-dock";
  const close = document.createElement("button");
  close.type = "button";
  close.className = "job-close";
  close.innerHTML = '<svg viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9"/></svg>';
  head.append(title, state, dockButton, close);
  const followBar = document.createElement("div");
  followBar.className = "job-follow";
  followBar.hidden = true;
  const followLabel = document.createElement("span");
  const resume = document.createElement("button");
  resume.type = "button";
  followBar.append(followLabel, resume);
  const body = document.createElement("div");
  body.className = "job-body";
  body.tabIndex = 0;
  const commandSection = document.createElement("section");
  commandSection.className = "job-command-section";
  const commandLabel = document.createElement("h4");
  const command = document.createElement("pre");
  command.className = "job-command";
  commandSection.append(commandLabel, command);
  body.append(commandSection);
  const streams = (["stdout", "stderr"] as const).map(name => {
    const section = document.createElement("section");
    section.className = "job-stream";
    section.dataset.stream = name;
    section.setAttribute("aria-label", name.toUpperCase());
    const heading = document.createElement("h4");
    heading.textContent = name.toUpperCase();
    const pre = document.createElement("pre");
    section.append(heading, pre);
    body.append(section);
    return { name, section, pre };
  });
  panel.append(head, followBar, body);
  workbench.append(splitter, panel);
  const controller = new AbortController();
  const jobs = new Map<string, Job>();
  const dismissed = new Set<string>();
  const elements = new Map<string, {
    item: HTMLDivElement;
    select: HTMLButtonElement;
    led: HTMLSpanElement;
    name: HTMLSpanElement;
    number: HTMLButtonElement;
    dismiss: HTMLButtonElement;
  }>();
  let selected: string | null = null;
  let latest: string | null = null;
  let lastRendered: string | null = null;
  let follow = true;
  let expanded = false;
  let disposed = false;
  let contextIds = new Set<string>();
  let dock: "right" | "bottom" = "right";
  let sideSize = 380;
  let bottomSize = 240;
  let dragging = false;
  const storageKey = `coterm.ssh.exec-layout.v1.${api.driverInstanceId}`;
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(storageKey) ?? "null");
    if (saved && typeof saved === "object") {
      const value = saved as Record<string, unknown>;
      if (value.dock === "bottom" || value.dock === "right")
        dock = value.dock;
      if (typeof value.sideSize === "number" && Number.isFinite(value.sideSize))
        sideSize = Math.max(220, Math.min(900, value.sideSize));
      if (typeof value.bottomSize === "number" && Number.isFinite(value.bottomSize))
        bottomSize = Math.max(120, Math.min(900, value.bottomSize));
    }
  }
  catch { /* Storage may be unavailable; keep usable defaults. */ }
  const persist = () => { try {
    localStorage.setItem(storageKey, JSON.stringify({ dock, sideSize, bottomSize }));
  }
  catch { /* Optional persistence. */ } };
  const effectiveDock = () => workbench.clientWidth < 760 ? "bottom" : dock;
  const layout = () => {
    const mode = effectiveDock();
    workbench.dataset.dock = mode;
    panel.style.width = mode === "right" ? `${Math.max(200, Math.min(sideSize, workbench.clientWidth * .6))}px` : "auto";
    panel.style.height = mode === "bottom" ? `${Math.max(100, Math.min(bottomSize, workbench.clientHeight * .65))}px` : "auto";
    splitter.setAttribute("aria-orientation", mode === "right" ? "vertical" : "horizontal");
    splitter.setAttribute("aria-label", t("Resize Exec panel"));
    splitter.setAttribute("aria-valuemin", "100");
    splitter.setAttribute("aria-valuemax", String(Math.round(mode === "right" ? workbench.clientWidth * .6 : workbench.clientHeight * .65)));
    splitter.setAttribute("aria-valuenow", String(Math.round(mode === "right" ? panel.clientWidth : panel.clientHeight)));
  };
  const status = (job: Job) => !job.done ? "running" : job.cancelled ? "cancelled" : job.timedOut ? "timed out" : job.exitCode === null ? "unknown" : job.exitCode !== 0 ? "error" : "success";
  const short = (job: Job) => (job.text.trim().split("\n")[0] || job.operationId).slice(0, 48);
  const renderPanel = () => {
    if (disposed)
      return;
    const job = selected ? jobs.get(selected) : undefined;
    panel.hidden = splitter.hidden = !job;
    if (!job) {
      lastRendered = null;
      return;
    }
    command.textContent = job.text;
    commandLabel.textContent = t("Command");
    const currentState = status(job);
    state.hidden = currentState === "success";
    state.dataset.state = currentState;
    state.textContent = state.hidden ? "" : currentState === "error" ? t("exit {code}", { code:job.exitCode ?? "?" }) : t(currentState);
    close.title = t("Close view");
    close.setAttribute("aria-label", close.title);
    dockButton.textContent = t(dock === "right" ? "Dock bottom" : "Dock right");
    dockButton.setAttribute("aria-label", dockButton.textContent);
    panel.setAttribute("aria-label", t("Exec output"));
    followBar.hidden = follow;
    followLabel.textContent = t("Viewing history #{index}", { index: job.index ?? job.operationId.slice(0, 8) });
    resume.textContent = t("Follow latest");
    const atBottom = body.scrollTop + body.clientHeight >= body.scrollHeight - 24;
    const switched = lastRendered !== selected;
    for (const stream of streams) {
      const text = job[stream.name] || job.preview?.[stream.name] || "";
      if (stream.pre.textContent !== text)
        stream.pre.textContent = text;
      stream.section.hidden = !text;
    }
    if (switched)
      body.scrollTop = follow ? body.scrollHeight : 0;
    else if (follow && atBottom)
      body.scrollTop = body.scrollHeight;
    lastRendered = selected;
    layout();
  };
  const render = (align = false) => {
    if (disposed)
      return;
    const wasExpanded = jobsBar.dataset.expanded === "true";
    const bounds = list.getBoundingClientRect();
    const anchor = Array.from(list.children).find(node => node instanceof HTMLElement && !node.hidden && (wasExpanded ? node.getBoundingClientRect().bottom > bounds.top : node.getBoundingClientRect().right > bounds.left));
    const before = anchor?.getBoundingClientRect();
    const atEnd = list.scrollWidth - list.clientWidth - list.scrollLeft < 2;
    jobsBar.hidden = jobs.size === 0;
    jobsBar.dataset.expanded = String(expanded);
    label.textContent = `${t("JOBS")} ${jobs.size}`;
    list.setAttribute("aria-label", t("Background jobs"));
    expand.textContent = expanded ? "▴" : "▾";
    expand.setAttribute("aria-expanded", String(expanded));
    expand.setAttribute("aria-label", t(expanded ? "Collapse jobs" : "Expand jobs"));
    for (const [id, view] of elements)
      if (!jobs.has(id)) {
        view.item.remove();
        elements.delete(id);
      }
    let after = false;
    let next = list.firstElementChild;
    for (const job of [...jobs.values()].reverse()) {
      let view = elements.get(job.operationId);
      if (!view) {
        const item = document.createElement("div");
        item.className = "job";
        item.setAttribute("role", "listitem");
        item.dataset.operationId = job.operationId;
        const number = document.createElement("button");
        number.type = "button";
        number.className = "job-block-number";
        number.addEventListener("click", () => toggleBlock(job.operationId), { signal: controller.signal });
        const select = document.createElement("button");
        select.type = "button";
        select.className = "job-select";
        const led = document.createElement("span");
        led.className = "job-led";
        led.setAttribute("aria-hidden", "true");
        const name = document.createElement("span");
        name.className = "name";
        select.append(led, name);
        const dismiss = document.createElement("button");
        dismiss.type = "button";
        dismiss.className = "dismiss";
        dismiss.innerHTML = '<svg viewBox="0 0 12 12" fill="none" stroke="currentColor" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9"/></svg>';
        select.addEventListener("click", () => { selected = job.operationId; follow = selected === latest; expanded = false; render(true); renderPanel(); void readJob(job); }, { signal: controller.signal });
        dismiss.addEventListener("click", () => {
          dismissed.add(job.operationId);
          if (dismissed.size > 64) dismissed.delete(dismissed.values().next().value ?? "");
          jobs.delete(job.operationId);
          if (selected === job.operationId) {
            selected = null;
            follow = true;
          }
          if (latest === job.operationId)
            latest = Array.from(jobs.keys()).slice(-1)[0] ?? null;
          render(true);
          renderPanel();
        }, { signal: controller.signal });
        item.append(number, select, dismiss);
        view = { item, select, led, name, number, dismiss };
        elements.set(job.operationId, view);
      }
      view.item.hidden = !expanded && after;
      view.item.dataset.selected = String(selected === job.operationId);
      view.select.title = job.text;
      view.select.setAttribute("aria-current", String(selected === job.operationId));
      view.select.setAttribute("aria-label", `${job.text} · ${t(status(job))}`);
      view.led.dataset.state = status(job);
      view.name.textContent = short(job);
      view.number.hidden = job.index === undefined;
      view.number.textContent = String(job.index);
      view.number.setAttribute("aria-pressed", String(contextIds.has(job.operationId)));
      view.number.setAttribute("aria-label", t("Select block #{index} for Agent", { index: job.index ?? "?" }));
      view.dismiss.setAttribute("aria-label", `${t("Remove job from list")}: ${short(job)}`);
      if (view.item !== next)
        list.insertBefore(view.item, next);
      next = view.item.nextElementSibling;
      if (selected === job.operationId)
        after = true;
    }
    if (!expanded && (align || (selected && atEnd))) {
      list.scrollLeft = selected ? list.scrollWidth : 0;
      list.scrollTop = 0;
    }
    else if (expanded !== wasExpanded) {
      const item = selected ? elements.get(selected)?.item : undefined;
      list.scrollTop = item ? item.getBoundingClientRect().top - list.getBoundingClientRect().top : 0;
    }
    else if (anchor instanceof HTMLElement && !anchor.hidden && before) {
      const now = anchor.getBoundingClientRect();
      if (expanded)
        list.scrollTop += now.top - before.top;
      else
        list.scrollLeft += now.left - before.left;
    }
  };
  const readJob = async (job: Job) => {
    if (disposed || job.reading || jobs.get(job.operationId) !== job)
      return;
    job.reading = true;
    try {
        const receipt = await api.invoke<{
          status: "running" | "completed" | "cancelled" | "timed_out" | "unknown";
          exitCode: number | null;
          stdout: string;
          stderr: string;
          stdoutEncoding: "utf-8" | "base64";
          stderrEncoding: "utf-8" | "base64";
          truncated: boolean;
        }>("ssh.output", { operationId: job.operationId });
        if (disposed || jobs.get(job.operationId) !== job)
          return;
        if (receipt.error || receipt.status !== "completed" || !receipt.output)
          throw new Error(receipt.error?.message ?? "SSH output unavailable");
        const output = receipt.output;
        for (const name of ["stdout", "stderr"] as const) {
          const value = output[name];
          const encoding = name === "stdout" ? output.stdoutEncoding : output.stderrEncoding;
          job[name] = encoding === "base64" ? new TextDecoder().decode(Uint8Array.from(atob(value), c => c.charCodeAt(0))) : value;
        }
        job.done = output.status !== "running";
        job.exitCode = output.exitCode;
        job.cancelled = output.status === "cancelled";
        job.timedOut = output.status === "timed_out";
        if (output.truncated) job.stdout += "\n" + t("Output gap: older data expired.");
      render();
      renderPanel();
    }
    catch (reason) {
      if (!disposed && selected === job.operationId)
        showError(reason);
    }
    finally {
      job.reading = false;
    }
  };
  const track = (id: string, text: string, announce = true) => {
    let job = jobs.get(id);
    if (!job) {
      while (jobs.size >= 64) {
        const oldest = [...jobs.keys()].find(key => key !== selected);
        if (!oldest)
          break;
        jobs.delete(oldest);
      }
      job = { operationId: id, text, done: false, exitCode: null, cancelled: false, timedOut: false, stdout: "", stderr: "", reading: false };
      jobs.set(id, job);
      if (announce) {
        latest = id;
        if (selected && follow)
          selected = id;
      }
    }
    if (announce) {
      render();
      renderPanel();
    }
    return job;
  };
  const resumeLatest = () => { follow = true; if (selected && latest) {
    selected = latest;
    render(true);
    renderPanel();
    const job = jobs.get(latest);
    if (job)
      void readJob(job);
  } };
  close.addEventListener("click", () => { selected = null; follow = true; render(true); renderPanel(); }, { signal: controller.signal });
  resume.addEventListener("click", resumeLatest, { signal: controller.signal });
  panel.addEventListener("keydown", event => { if (event.key === "Escape" && !follow) {
    event.preventDefault();
    event.stopPropagation();
    resumeLatest();
  } }, { signal: controller.signal });
  dockButton.addEventListener("click", () => { dock = dock === "right" ? "bottom" : "right"; persist(); layout(); }, { signal: controller.signal });
  expand.addEventListener("click", () => { expanded = !expanded; render(!expanded); }, { signal: controller.signal });
  list.addEventListener("wheel", event => {
    if (expanded || event.ctrlKey || list.scrollWidth <= list.clientWidth)
      return;
    const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
    list.scrollLeft += delta * (event.deltaMode === 1 ? 26 : event.deltaMode === 2 ? list.clientWidth : 1);
    event.preventDefault();
  }, { passive: false, signal: controller.signal });
  splitter.addEventListener("pointerdown", event => { if (event.button !== 0)
    return; event.preventDefault(); dragging = true; splitter.setPointerCapture(event.pointerId); }, { signal: controller.signal });
  splitter.addEventListener("pointermove", event => {
    if (!dragging)
      return;
    const bounds = workbench.getBoundingClientRect();
    if (effectiveDock() === "right")
      sideSize = Math.max(220, Math.min(bounds.width * .6, bounds.right - event.clientX));
    else
      bottomSize = Math.max(120, Math.min(bounds.height * .65, bounds.bottom - event.clientY));
    layout();
  }, { signal: controller.signal });
  const endDrag = () => { if (dragging) {
    dragging = false;
    persist();
  } };
  splitter.addEventListener("pointerup", endDrag, { signal: controller.signal });
  splitter.addEventListener("pointercancel", endDrag, { signal: controller.signal });
  splitter.addEventListener("lostpointercapture", endDrag, { signal: controller.signal });
  splitter.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key))
      return;
    event.preventDefault();
    const delta = event.key === "ArrowLeft" || event.key === "ArrowUp" ? 20 : -20;
    if (effectiveDock() === "right")
      sideSize = Math.max(220, sideSize + delta);
    else
      bottomSize = Math.max(120, bottomSize + delta);
    layout();
    persist();
  }, { signal: controller.signal });
  const observer = new ResizeObserver(() => { layout(); if (!expanded && selected)
    list.scrollLeft = list.scrollWidth; });
  observer.observe(workbench);
  observer.observe(list);
  const poll = window.setInterval(() => { const job = selected ? jobs.get(selected) : undefined; if (job && !job.done)
    void readJob(job); }, 300);
  layout();
  render();
  // Keep the terminal's allocation explicit when the panel changes orientation.
  terminalPane.style.flex = "1";
  return {
    async event(payload: {
      operationId?: string;
      text?: string;
    }) { if (payload.operationId && !dismissed.has(payload.operationId)) {
      const job = track(payload.operationId, payload.text ?? payload.operationId);
      await readJob(job);
    } },
    setBlocks(blocks: SshBlock[]) {
      let newest: string | null = null;
      for (const block of blocks) {
        if (block.channel !== "exec" || dismissed.has(block.id))
          continue;
        const job = track(block.id, block.command, false);
        job.text = block.command;
        job.preview = {stdout: block.output ?? "", stderr: block.stderr};
        job.index = block.index;
        job.done = block.state !== "running";
        job.exitCode = block.exitCode ?? null;
        job.cancelled = block.state === "cancelled";
        job.timedOut = block.state === "timed_out";
        newest = block.id;
      }
      if (newest && (!latest || jobs.get(latest)?.index !== undefined)) {
        latest = newest;
        if (selected && follow)
          selected = latest;
      }
      render();
      renderPanel();
    },
    closed() { for (const job of jobs.values()) {
      if (!job.done) {
        job.done = true;
        job.exitCode = null;
      }
    } render(); renderPanel(); },
    selection(ids: string[]) { contextIds = new Set(ids); render(); },
    reset() { jobs.clear(); dismissed.clear(); selected = latest = null; follow = true; expanded = false; render(); renderPanel(); },
    refreshLocale() { render(); renderPanel(); },
    unmount() { disposed = true; controller.abort(); observer.disconnect(); window.clearInterval(poll); style.remove(); panel.remove(); splitter.remove(); jobsBar.replaceChildren(); jobs.clear(); elements.clear(); },
  };
}
