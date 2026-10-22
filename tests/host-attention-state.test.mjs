/**
 * host-api 订阅层的状态与契约测试（无 DOM、无浏览器）。
 *
 * 证明范围：
 * - 本文件：`subscribeHostAttention` 的并发注册、注销顺序、失败回滚、代次隔离与版本补取。
 *   这些是 host-api 的纯状态机行为，用手写 IPC 支架就能完整覆盖。
 * - `tests/host-interactions-component.test.mjs`：真实 HostInteractions 组件 + 真实 secret-input + 真实 IPC
 *   契约下的弹窗队列与凭据归属，在浏览器里跑。
 * - `tests/browser-layout.test.mjs`：真实构建产物的布局、真实 SDK 组件与真实 SSH 前端。
 * - Rust 测试（`plugins/plugin-ssh/tests/*`、`crates/*`）：协议核心与基座安全边界。
 *
 * 本文件不声称覆盖任何 UI 渲染或组件生命周期。
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
  const gen2Requested = createDeferred();
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") {
      invokeCalls++;
      if (invokeCalls === 1) return snapshotDeferred.promise;
      gen2Requested.resolve();
      return { version: 200, approvals: [], interactions: [] };
    }
    return null;
  };

  const stop1 = await subscribeHostAttention({ snapshot: () => {}, error: () => {} });
  assert.equal(invokeCalls, 1);

  // 在第一次快照还在途时注销，然后让旧快照迟到返回。
  stop1();
  snapshotDeferred.resolve({ version: 999, approvals: [], interactions: [] });

  // 新代次订阅：它的请求必须立刻发出，且不能拿到旧代次的 999。
  const gen2Snapshots = [];
  const stop2 = await subscribeHostAttention({
    snapshot: (att) => gen2Snapshots.push(att),
    error: () => {},
  });
  await gen2Requested.promise;
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


// --- 3. 审批请求的宿主级路由（不用手写 map/filter 冒充 UI） ---
// 真实卡片的渲染、命令名/参数与未决身份断言在 `browser-layout.test.mjs` 里跑。
// 这里只验证 hostAPI 这一层：实例级与插件级分别路由到正确的命令和参数。

test("hostAPI routes instance-level and plugin-level approvals to distinct commands", async () => {
  const approved = [];
  const rejected = [];
  mockInvoke = async (cmd, args) => {
    if (cmd === "approve_proposal" || cmd === "reject_proposal"
      || cmd === "approve_plugin_proposal" || cmd === "reject_plugin_proposal") {
      (cmd.startsWith("approve") ? approved : rejected).push({ cmd, args });
      return cmd.startsWith("approve") ? { operationId: args.proposalId, status: "completed" } : null;
    }
    return null;
  };

  await hostAPI.approveProposal("inst-1", "prop-inst-1", { command: "systemctl restart nginx" });
  await hostAPI.rejectProposal("inst-1", "prop-inst-2");
  await hostAPI.approvePluginProposal("ssh", "prop-plugin-1", { action: "scan" });
  await hostAPI.rejectPluginProposal("ssh", "prop-plugin-2");

  assert.deepEqual(approved.map((item) => item.cmd), ["approve_proposal", "approve_plugin_proposal"]);
  assert.deepEqual(approved[0].args, {
    pluginInstanceId: "inst-1",
    proposalId: "prop-inst-1",
    args: { command: "systemctl restart nginx" },
  }, "instance-level approval binds its own instance, proposal and draft args");
  assert.deepEqual(approved[1].args, {
    pluginId: "ssh",
    proposalId: "prop-plugin-1",
    args: { action: "scan" },
  }, "plugin-level approval binds the plugin id instead of an instance");
  assert.deepEqual(rejected.map((item) => item.cmd), ["reject_proposal", "reject_plugin_proposal"]);
  assert.deepEqual(rejected[0].args, { pluginInstanceId: "inst-1", proposalId: "prop-inst-2" });
  assert.deepEqual(rejected[1].args, { pluginId: "ssh", proposalId: "prop-plugin-2" });
});

// --- 4. secret-input 自身行为（不经过组件，也不声称覆盖组件生命周期） ---
// 组件级的弹窗队列、取消/过期/销毁/卸载与迟到 IPC 在
// `tests/host-interactions-component.test.mjs` 里用真实浏览器和真实组件验证。

test("promptForSecret: resolves submitted once, ignores later submits, and binds the cancellation key", async () => {
  // 最小 DOM：secret-input 只用到 createElement/append/showModal/close/remove 和窗口计时器。
  const elements = [];
  const createElement = (tagName) => {
    const element = {
      tagName: tagName.toUpperCase(),
      children: [],
      attributes: new Map(),
      listeners: new Map(),
      className: "",
      textContent: "",
      value: "",
      open: false,
      disabled: false,
      hidden: false,
      append(...nodes) { this.children.push(...nodes); },
      setAttribute(name, value) { this.attributes.set(name, String(value)); },
      addEventListener(event, handler) {
        if (!this.listeners.has(event)) this.listeners.set(event, []);
        this.listeners.get(event).push(handler);
      },
      dispatch(event) { for (const handler of this.listeners.get(event.type) ?? []) handler(event); },
      showModal() { this.open = true; },
      close() { this.open = false; },
      remove() { elements.splice(elements.indexOf(this), 1); },
      find(predicate) {
        for (const child of this.children) {
          if (predicate(child)) return child;
          const found = child.find?.(predicate);
          if (found) return found;
        }
        return null;
      },
    };
    elements.push(element);
    return element;
  };
  globalThis.document = { body: { append: () => {} }, createElement };

  const { promptForSecret, cancelSecretInput } = await import("../apps/desktop/src/secret-input.ts");

  /** 取最近一次 promptForSecret 创建的那个 dialog（第一个新建节点）。 */
  const takeDialog = (startIndex) => elements[startIndex];

  const deliveries = [];
  const startIndex = elements.length;
  const pending = promptForSecret("inst-1", {
    title: "Web-Server SSH Password",
    prompt: "Enter password for root@web",
    timeoutMs: 60_000,
    allowCancel: true,
    purpose: "ssh.password",
  }, async (instance, purpose, value) => {
    deliveries.push({ instance, purpose, value });
    return { handle: "secret-1" };
  }, "backend:req-1");

  const dialog = takeDialog(startIndex);
  assert.equal(dialog.open, true, "dialog is shown modally");
  const input = dialog.find((node) => node.tagName === "INPUT");
  const form = dialog.find((node) => node.tagName === "FORM");
  input.value = "pw-for-web";

  form.dispatch({ type: "submit", preventDefault() {} });
  const outcome = await pending;

  assert.equal(outcome.outcome, "submitted");
  assert.deepEqual(deliveries, [{ instance: "inst-1", purpose: "ssh.password", value: "pw-for-web" }],
    "deliver receives the instance, the declared purpose and the typed value");
  assert.equal(dialog.open, false, "dialog closes on submit");

  // 已结算之后再提交：deliver 绝不第二次执行。
  form.dispatch({ type: "submit", preventDefault() {} });
  assert.equal(deliveries.length, 1, "a settled dialog never delivers twice");
  assert.equal(input.value, "", "the typed secret is cleared on settlement");

  // 取消通过 cancellationKey 生效，而不是 instanceId。
  const cancelIndex = elements.length;
  const cancelDeliveries = [];
  const cancelTarget = promptForSecret("inst-2", {
    title: "Second",
    prompt: "Password",
    timeoutMs: 60_000,
    allowCancel: true,
    purpose: "ssh.password",
  }, async (_instance, _purpose, value) => {
    cancelDeliveries.push(value);
    return { handle: "unused" };
  }, "backend:req-2");
  const cancelledDialog = takeDialog(cancelIndex);
  const cancelledInput = cancelledDialog.find((node) => node.tagName === "INPUT");
  const cancelledForm = cancelledDialog.find((node) => node.tagName === "FORM");
  cancelSecretInput("backend:req-2");
  assert.equal((await cancelTarget).outcome, "cancelled", "cancel uses the per-request cancellation key");

  // 取消之后迟到的 submit 不得提交任何秘密。
  // 取消路径上 busy 始终是 false，只有 settled 能挡住这次提交，
  // 所以这条断言测的确实是「结算后不再投递」，而不是顺带被 busy 拦住。
  cancelledInput.value = "late-secret";
  cancelledForm.dispatch({ type: "submit", preventDefault() {} });
  await Promise.resolve();
  assert.deepEqual(cancelDeliveries, [], "a late submit on a cancelled dialog never delivers the secret");
});
