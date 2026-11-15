import { viewNodes } from "../view";
import { mountJobsList } from "./jobs-list";
import type { DriverAPI } from "../../../../shared/index";
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

export function mountJobs(workbench: HTMLElement, terminalPane: HTMLElement, jobsBar: HTMLElement, api: DriverAPI, t: ReturnType<typeof createTranslator>, toggleBlock: (id: string) => void, showError: (reason: unknown) => void) {

  const node = viewNodes(workbench);
  const label = node<HTMLElement>('jobsLabel'), list = node<HTMLElement>('jobsList');
  const expand = node<HTMLButtonElement>('jobsExpand');
  const splitter = node<HTMLElement>('jobSplitter'), panel = node<HTMLElement>('jobPanel');
  const state = node<HTMLElement>('jobState'), dockButton = node<HTMLButtonElement>('jobDock');
  const close = node<HTMLButtonElement>('jobClose'), followBar = node<HTMLElement>('jobFollow');
  const followLabel = node<HTMLElement>('jobFollowLabel'), resume = node<HTMLButtonElement>('jobResume');
  const body = node<HTMLElement>('jobBody'), commandLabel = node<HTMLElement>('jobCommandLabel');
  const command = node<HTMLElement>('jobCommand');
  const streams = (["stdout", "stderr"] as const).map(name => {
    const pre = node<HTMLElement>(`job-${name}`);
    return { name, pre, section: pre.closest<HTMLElement>('.job-stream')! };
  });
  const jobList = mountJobsList(list);
  const controller = new AbortController();
  const jobs = new Map<string, Job>();
  const dismissed = new Set<string>();
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
    jobList.update({
      items: [...jobs.values()].reverse().map(job => ({ id: job.operationId, text: job.text, name: short(job), state: status(job), index: job.index })),
      selected, expanded, contextIds, t, toggle: toggleBlock,
      select(id) { const job = jobs.get(id); if (!job) return; selected = id; follow = selected === latest; expanded = false; render(true); renderPanel(); void readJob(job); },
      dismiss(id) {
        dismissed.add(id); if (dismissed.size > 64) dismissed.delete(dismissed.values().next().value ?? "");
        jobs.delete(id); if (selected === id) { selected = null; follow = true; }
        if (latest === id) latest = Array.from(jobs.keys()).slice(-1)[0] ?? null;
        render(true); renderPanel();
      },
      layout() {
    if (!expanded && (align || (selected && atEnd))) {
      list.scrollLeft = selected ? list.scrollWidth : 0;
      list.scrollTop = 0;
    }
    else if (expanded !== wasExpanded) {
      const item = selected ? Array.from(list.children).find(item => item instanceof HTMLElement && item.dataset.operationId === selected) : undefined;
      list.scrollTop = item ? item.getBoundingClientRect().top - list.getBoundingClientRect().top : 0;
    }
    else if (anchor instanceof HTMLElement && !anchor.hidden && before) {
      const now = anchor.getBoundingClientRect();
      if (expanded)
        list.scrollTop += now.top - before.top;
      else
        list.scrollLeft += now.left - before.left;
    }
      },
    });
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
    unmount() { disposed = true; controller.abort(); observer.disconnect(); window.clearInterval(poll);  panel.hidden = splitter.hidden = jobsBar.hidden = true; jobList.unmount(); jobs.clear(); },
  };
}
