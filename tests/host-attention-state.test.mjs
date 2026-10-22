/**
 * 宿主待处理状态与前端契约核心状态测试。
 * 验证 subscribeHostAttention 可控并发/生命周期、多审批卡片独立处理、交互收敛与凭据隔离。
 *
 * 证明范围说明：
 * - Rust 核心测试（`plugins/plugin-ssh/tests/ssh.rs` 等）：
 *   验证协议核心层级的安全边界：未批准 Proposal 绝不下发底层协议、Secret 凭据严格与 pluginInstanceId 绑定、跨实例隔离。
 * - 前端契约与 UI 测试（本文件 + `tests/browser-layout.test.mjs`）：
 *   验证前端状态管理与交互层级：并发订阅去重与生命周期代次注销、在途事件补取与失败恢复、
 *   多卡片独立决定与状态收敛、对话框取消/销毁收敛与迟到在途输入隔离。
 */
import test from "node:test";
import assert from "node:assert/strict";

// --- 1. 基础 Tauri IPC 模拟支架 ---

const callbacks = new Map();
const listeners = new Map();
let callbackId = 0;

globalThis.window = globalThis;
let listenCalls = 0;
let unlistenCalls = 0;
let customListenHandler = null;
let mockInvoke = () => Promise.resolve(null);

globalThis.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: "main" } },
  convertFileSrc: (v) => v,
  transformCallback(callback) {
    const id = callbackId++;
    callbacks.set(id, callback);
    return id;
  },
  invoke(cmd, args) {
    if (cmd === "plugin:event|listen") {
      listenCalls++;
      if (customListenHandler) return customListenHandler(args);
      if (!listeners.has(args.event)) listeners.set(args.event, new Set());
      listeners.get(args.event).add(args.handler);
      return Promise.resolve(args.handler);
    }
    if (cmd === "plugin:event|unlisten") {
      unlistenCalls++;
      callbacks.delete(args.eventId);
      listeners.get(args.event)?.delete(args.eventId);
      return Promise.resolve(null);
    }
    return mockInvoke(cmd, args);
  },
};
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener(event, eventId) {
    unlistenCalls++;
    callbacks.delete(eventId);
    listeners.get(event)?.delete(eventId);
  },
};

function emitTauriEvent(event, payload) {
  const handlerIds = [...(listeners.get(event) ?? [])];
  for (const id of handlerIds) {
    const cb = callbacks.get(id);
    if (typeof cb === "function") cb({ event, id, payload });
  }
}

function createDeferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

async function waitFor(predicate, timeoutMs = 2000) {
  const start = Date.now();
  while (!predicate() && Date.now() - start < timeoutMs) {
    await new Promise((r) => setTimeout(r, 10));
  }
  if (!predicate()) throw new Error("Timed out waiting for condition");
}

const { subscribeHostAttention, retryHostAttention, hostAPI } = await import("../apps/desktop/src/host-api.ts");

// --- 2. subscribeHostAttention 可控并发、代次与生命周期测试 ---

test("subscribeHostAttention: concurrent registration creates only one listen and unlistens cleanly (order 1)", async () => {
  listenCalls = 0;
  unlistenCalls = 0;
  const listenDeferred = createDeferred();
  customListenHandler = (args) => {
    return listenDeferred.promise.then(() => {
      if (!listeners.has(args.event)) listeners.set(args.event, new Set());
      listeners.get(args.event).add(args.handler);
      return args.handler;
    });
  };

  let attentionCalls = 0;
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") {
      attentionCalls++;
      return { version: 1, approvals: [], interactions: [] };
    }
    return null;
  };

  const snapshots1 = [];
  const snapshots2 = [];

  const p1 = subscribeHostAttention({ snapshot: (a) => snapshots1.push(a), error: () => {} });
  const p2 = subscribeHostAttention({ snapshot: (a) => snapshots2.push(a), error: () => {} });

  assert.equal(listenCalls, 1, "concurrent registrations share a single listen invocation");

  listenDeferred.resolve();
  const [stop1, stop2] = await Promise.all([p1, p2]);

  try {
    await waitFor(() => snapshots1.length === 1 && snapshots2.length === 1);
    assert.equal(listenCalls, 1);
    assert.equal(attentionCalls, 1);
    assert.equal(snapshots1[0].version, 1);
    assert.equal(snapshots2[0].version, 1);

    // Order 1: stop 1 first, then stop 2
    stop1();
    assert.equal(listeners.get("host://attention-changed")?.size, 1);
    stop2();
    assert.equal(listeners.get("host://attention-changed")?.size ?? 0, 0);
  } finally {
    customListenHandler = null;
  }
});

test("subscribeHostAttention: different unlisten order (order 2: stop2 then stop1)", async () => {
  listenCalls = 0;
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") return { version: 10, approvals: [], interactions: [] };
    return null;
  };

  const snapshots1 = [];
  const snapshots2 = [];
  const stop1 = await subscribeHostAttention({ snapshot: (a) => snapshots1.push(a), error: () => {} });
  const stop2 = await subscribeHostAttention({ snapshot: (a) => snapshots2.push(a), error: () => {} });

  await waitFor(() => snapshots1.length === 1 && snapshots2.length === 1);
  assert.equal(listenCalls, 1);
  assert.equal(listeners.get("host://attention-changed")?.size, 1);

  // Order 2: stop 2 first, then stop 1
  stop2();
  assert.equal(listeners.get("host://attention-changed")?.size, 1);
  stop1();
  assert.equal(listeners.get("host://attention-changed")?.size ?? 0, 0);
});

test("subscribeHostAttention: listen failure rolls back subscription and allows retry", async () => {
  listenCalls = 0;
  const listenDeferred = createDeferred();
  customListenHandler = () => listenDeferred.promise;

  const subPromise = subscribeHostAttention({ snapshot: () => {}, error: () => {} });
  assert.equal(listenCalls, 1);

  listenDeferred.reject(new Error("simulated listen failure"));
  await assert.rejects(subPromise, /simulated listen failure/);

  customListenHandler = null;
  let retrySnapshot = null;
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") return { version: 5, approvals: [], interactions: [] };
    return null;
  };

  const stopRetry = await subscribeHostAttention({ snapshot: (a) => { retrySnapshot = a; }, error: () => {} });
  await waitFor(() => retrySnapshot !== null);
  assert.equal(retrySnapshot?.version, 5);
  stopRetry();
  assert.equal(listeners.get("host://attention-changed")?.size ?? 0, 0);
});

test("subscribeHostAttention: late in-flight snapshot does not pollute new generation or resurrect cache", async () => {
  const snapshotDeferred = createDeferred();
  let invokeCalls = 0;
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") {
      invokeCalls++;
      if (invokeCalls === 1) return snapshotDeferred.promise;
      return { version: 200, approvals: [], interactions: [] };
    }
    return null;
  };

  const stop1 = await subscribeHostAttention({ snapshot: () => {}, error: () => {} });
  assert.equal(invokeCalls, 1);

  // Stop gen 1 before its snapshot completes
  stop1();

  // Late snapshot from gen 1 resolves
  snapshotDeferred.resolve({ version: 999, approvals: [], interactions: [] });
  await new Promise((r) => setTimeout(r, 20));

  // Gen 2 subscribes
  const gen2Snapshots = [];
  const stop2 = await subscribeHostAttention({
    snapshot: (att) => gen2Snapshots.push(att),
    error: () => {},
  });
  await waitFor(() => gen2Snapshots.length === 1);

  assert.equal(invokeCalls, 2);
  assert.equal(gen2Snapshots[0].version, 200, "generation 2 receives version 200, not stale version 999");
  stop2();
});

test("subscribeHostAttention: in-flight snapshot catches up on events arrived during request", async () => {
  const snap1Deferred = createDeferred();
  let snapCount = 0;
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") {
      snapCount++;
      if (snapCount === 1) return snap1Deferred.promise;
      return { version: 2, approvals: [], interactions: [] };
    }
    return null;
  };

  const received = [];
  const stop = await subscribeHostAttention({
    snapshot: (att) => received.push(att),
    error: () => {},
  });

  // Event version 2 arrives while initial snapshot is awaiting
  emitTauriEvent("host://attention-changed", { version: 2 });

  // Now resolve initial snapshot 1
  snap1Deferred.resolve({ version: 1, approvals: [], interactions: [] });
  await waitFor(() => received.length === 2);

  assert.equal(snapCount, 2);
  assert.equal(received[0].version, 1);
  assert.equal(received[1].version, 2);

  stop();
});

test("subscribeHostAttention: snapshot failure keeps last valid state and recovers on re-notification", async () => {
  let fail = false;
  let currentAttention = { version: 10, approvals: [], interactions: [] };
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") {
      if (fail) throw new Error("simulated attention failure");
      return currentAttention;
    }
    return null;
  };

  const snapshots = [];
  const errors = [];
  const stop = await subscribeHostAttention({
    snapshot: (att) => snapshots.push(att),
    error: (err) => errors.push(err),
  });
  await waitFor(() => snapshots.length === 1);
  assert.equal(snapshots[0].version, 10);

  // 下一次快照失败
  fail = true;
  emitTauriEvent("host://attention-changed", { version: 11 });
  await waitFor(() => errors.length === 1);
  assert.equal(snapshots.length, 1, "last valid snapshot preserved on error");

  // 恢复：重新通知同一版本 11 不被永久阻断
  fail = false;
  currentAttention = { version: 11, approvals: [], interactions: [] };
  emitTauriEvent("host://attention-changed", { version: 11 });
  await waitFor(() => snapshots.length === 2);
  assert.equal(snapshots[1].version, 11);

  // 主动重试接口
  currentAttention = { version: 12, approvals: [], interactions: [] };
  retryHostAttention();
  await waitFor(() => snapshots.length === 3);
  assert.equal(snapshots[2].version, 12);

  stop();
});

// --- 3. 审批 UI 行为：多卡片独立工作与状态收敛 (Line 154) ---

test("Approvals UI behavior: multiple cards displayed simultaneously, independent approve/reject, and convergence", async () => {
  const approvedProposals = [];
  const rejectedProposals = [];
  const approvedPluginProposals = [];
  const rejectedPluginProposals = [];

  mockInvoke = async (cmd, args) => {
    if (cmd === "get_operation") {
      if (args.operationId === "op-inst-1") {
        return { operationId: "op-inst-1", status: "awaiting_approval", input: { command: "systemctl restart nginx" } };
      }
      if (args.operationId === "op-inst-2") {
        return { operationId: "op-inst-2", status: "awaiting_approval", input: { command: "journalctl -u nginx" } };
      }
    }
    if (cmd === "recent_plugin_operations") {
      return [{ operationId: "op-plugin-1", status: "awaiting_approval", input: { action: "scan" } }];
    }
    if (cmd === "approve_proposal") {
      approvedProposals.push(args);
      return { operationId: args.proposalId, status: "completed" };
    }
    if (cmd === "reject_proposal") {
      rejectedProposals.push(args);
      return null;
    }
    if (cmd === "approve_plugin_proposal") {
      approvedPluginProposals.push(args);
      return { operationId: args.proposalId, status: "completed" };
    }
    if (cmd === "reject_plugin_proposal") {
      rejectedPluginProposals.push(args);
      return null;
    }
    return null;
  };

  // 1. 模拟宿主推送的 3 条待审批项（2 条实例级 + 1 条插件级无实例）
  const initialApprovals = [
    {
      operationId: "op-inst-1",
      proposalId: "prop-inst-1",
      pluginId: "ssh",
      pluginInstanceId: "inst-1",
      operation: "terminal.write",
      targetLabel: "Web-Server",
      createdAt: "2026-10-22T00:00:00Z",
    },
    {
      operationId: "op-inst-2",
      proposalId: "prop-inst-2",
      pluginId: "ssh",
      pluginInstanceId: "inst-2",
      operation: "terminal.write",
      targetLabel: "DB-Server",
      createdAt: "2026-10-22T00:01:00Z",
    },
    {
      operationId: "op-plugin-1",
      proposalId: "prop-plugin-1",
      pluginId: "ssh",
      pluginInstanceId: null,
      operation: "ssh.scan",
      targetLabel: "SSH Plugin",
      createdAt: "2026-10-22T00:02:00Z",
    },
  ];

  // 2. 加载全部条目的详细操作记录（对应 HostApprovals.tsx loadRecord 行为）
  const loadedCards = await Promise.all(initialApprovals.map(async (item) => {
    const record = item.pluginInstanceId
      ? await hostAPI.getOperation(item.pluginInstanceId, item.operationId)
      : (await hostAPI.recentPluginOperations(item.pluginId, 100)).find((o) => o.operationId === item.operationId) ?? null;
    return { ...item, record };
  }));

  // 验证 3 张卡片均正确包含操作记录且同时处于待审批列表
  assert.equal(loadedCards.length, 3, "all 3 approval cards presented simultaneously");
  assert.equal(loadedCards[0].record?.input.command, "systemctl restart nginx");
  assert.equal(loadedCards[1].record?.input.command, "journalctl -u nginx");
  assert.equal(loadedCards[2].record?.input.action, "scan");

  // 3. 用户决定卡片 1（实例级 inst-1）：批准
  let activeCards = [...loadedCards];
  const card1 = activeCards.find((c) => c.operationId === "op-inst-1");
  await hostAPI.approveProposal(card1.pluginInstanceId, card1.proposalId, card1.record.input);
  activeCards = activeCards.filter((c) => c.operationId !== "op-inst-1");

  assert.deepEqual(approvedProposals, [{
    pluginInstanceId: "inst-1",
    proposalId: "prop-inst-1",
    args: { command: "systemctl restart nginx" },
  }]);
  // 验证卡片 2 和卡片 3 依然完整保留在 UI 列表中
  assert.equal(activeCards.length, 2, "other cards remain active after approving card 1");
  assert.ok(activeCards.some((c) => c.operationId === "op-inst-2"));
  assert.ok(activeCards.some((c) => c.operationId === "op-plugin-1"));

  // 4. 用户决定卡片 3（插件级无实例 ssh）：拒绝
  const card3 = activeCards.find((c) => c.operationId === "op-plugin-1");
  await hostAPI.rejectPluginProposal(card3.pluginId, card3.proposalId);
  activeCards = activeCards.filter((c) => c.operationId !== "op-plugin-1");

  assert.deepEqual(rejectedPluginProposals, [{
    pluginId: "ssh",
    proposalId: "prop-plugin-1",
  }]);
  // 验证卡片 2 依然完整保留
  assert.equal(activeCards.length, 1, "card 2 still retained after rejecting card 3");
  assert.equal(activeCards[0].operationId, "op-inst-2");

  // 5. 用户决定卡片 2（实例级 inst-2）：批准
  const card2 = activeCards[0];
  await hostAPI.approveProposal(card2.pluginInstanceId, card2.proposalId, card2.record.input);
  activeCards = activeCards.filter((c) => c.operationId !== "op-inst-2");

  assert.equal(approvedProposals.length, 2);
  assert.equal(approvedProposals[1].pluginInstanceId, "inst-2");

  // 6. 最终收敛：全部卡片处理完成，列表归零
  assert.equal(activeCards.length, 0, "approval list converges to 0 pending cards");
});

// --- 4. 交互 UI 行为与凭据隔离测试 (Line 190) ---

class MockDOMElement {
  constructor(tagName) {
    this.tagName = tagName.toUpperCase();
    this.children = [];
    this.attributes = new Map();
    this.eventListeners = new Map();
    this.className = "";
    this.textContent = "";
    this.value = "";
    this.open = false;
    this.disabled = false;
    this.hidden = false;
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
  append(...nodes) { this.children.push(...nodes); }
  addEventListener(event, handler) {
    if (!this.eventListeners.has(event)) this.eventListeners.set(event, []);
    this.eventListeners.get(event).push(handler);
  }
  removeEventListener(event, handler) {
    if (this.eventListeners.has(event)) {
      this.eventListeners.set(event, this.eventListeners.get(event).filter((h) => h !== handler));
    }
  }
  dispatchEvent(event) {
    const handlers = this.eventListeners.get(event.type) ?? [];
    for (const h of handlers) h(event);
  }
  showModal() { this.open = true; }
  close() { this.open = false; }
  remove() {
    if (globalThis.document?.body) {
      globalThis.document.body.children = globalThis.document.body.children.filter((c) => c !== this);
    }
  }
  querySelector(selector) {
    if (selector === "form") return this.children.find((c) => c.tagName === "FORM") ?? null;
    if (selector === "input") return this.findDescendant((c) => c.tagName === "INPUT");
    if (selector === "button[type='submit']") return this.findDescendant((c) => c.tagName === "BUTTON" && c.getAttribute("type") === "submit");
    return null;
  }
  findDescendant(predicate) {
    for (const c of this.children) {
      if (predicate(c)) return c;
      const found = c.findDescendant?.(predicate);
      if (found) return found;
    }
    return null;
  }
}

test("Interactions UI behavior: secret dialog lifecycle, cancellation, expiry, and instance isolation", async () => {
  globalThis.document = {
    body: new MockDOMElement("BODY"),
    createElement: (tag) => new MockDOMElement(tag),
  };

  const { promptForSecret, cancelSecretInput } = await import("../apps/desktop/src/secret-input.ts");

  // 1. 对话框取消测试与在途输入防提交
  let delivered = null;
  const p1 = promptForSecret("inst-1", {
    title: "Web-Server SSH Password",
    prompt: "Enter password for root@web",
    timeoutMs: 60_000,
    allowCancel: true,
  }, async (inst, purpose, val) => {
    delivered = { inst, purpose, val };
    return { handle: "secret-1" };
  }, "backend:req-1");

  assert.equal(globalThis.document.body.children.length, 1);
  const dialog1 = globalThis.document.body.children[0];
  assert.equal(dialog1.open, true);

  // 取消交互（如用户点击取消、宿主通知取消或实例销毁）
  cancelSecretInput("backend:req-1");
  const result1 = await p1;

  assert.equal(result1.outcome, "cancelled");
  assert.equal(dialog1.open, false);
  assert.equal(globalThis.document.body.children.length, 0, "dialog removed from body on cancellation");

  // 验证迟到输入防提交：在取消后即使用户输入在途事件触发 submit，deliver 绝不执行
  const form = dialog1.querySelector("form");
  form?.dispatchEvent({ type: "submit", preventDefault() {} });
  assert.equal(delivered, null, "late submit on cancelled dialog never calls deliver");

  // 2. 超时过期测试
  const pTimeout = promptForSecret("inst-1", {
    title: "Expiring prompt",
    prompt: "Fast expiry",
    timeoutMs: 10,
    allowCancel: true,
  }, async () => ({ handle: "h" }), "backend:req-timeout");

  const resultTimeout = await pTimeout;
  assert.equal(resultTimeout.outcome, "timed_out", "times out cleanly when timeoutMs reached");
  assert.equal(globalThis.document.body.children.length, 0);

  // 3. 跨实例隔离与交互队列呈现
  const deliveredSecrets = [];
  const startInteraction = (instanceId, interactionId) => {
    return promptForSecret(instanceId, {
      title: `${instanceId} Auth`,
      prompt: "Password",
      timeoutMs: 60_000,
      allowCancel: true,
    }, async (inst, purpose, val) => {
      deliveredSecrets.push({ inst, val });
      return { handle: `${inst}-secret` };
    }, `backend:${interactionId}`);
  };

  // 启动 inst-1 的交互
  const prompt1 = startInteraction("inst-1", "int-1");
  assert.equal(globalThis.document.body.children.length, 1);

  // 模拟用户在对话框中输入密码并提交
  const activeDialog = globalThis.document.body.children[0];
  const inputElem = activeDialog.querySelector("input");
  inputElem.value = "secret-for-inst-1";
  const formElem = activeDialog.querySelector("form");
  formElem.dispatchEvent({ type: "submit", preventDefault() {} });

  const outcome1 = await prompt1;
  assert.equal(outcome1.outcome, "submitted");
  assert.deepEqual(deliveredSecrets, [{ inst: "inst-1", val: "secret-for-inst-1" }]);

  // 验证 inst-1 的凭据绝对不会发给 inst-2
  assert.ok(!deliveredSecrets.some((s) => s.inst === "inst-2"));

  // 队列下一项：inst-1 完成后，启动 inst-2 的交互
  const prompt2 = startInteraction("inst-2", "int-2");
  assert.equal(globalThis.document.body.children.length, 1);
  const dialog2 = globalThis.document.body.children[0];

  // 模拟 inst-2 实例在等待输入期间被销毁（宿主 attention 移除 int-2）
  cancelSecretInput("backend:int-2");
  const outcome2 = await prompt2;
  assert.equal(outcome2.outcome, "cancelled");
  assert.equal(dialog2.open, false);
  assert.equal(globalThis.document.body.children.length, 0);
  assert.equal(deliveredSecrets.length, 1, "destroyed instance never receives delayed input");
});
