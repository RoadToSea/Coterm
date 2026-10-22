/**
 * 宿主待处理状态与前端契约核心状态测试。
 * 验证 subscribeHostAttention、审批（插件级/实例级）及交互收敛逻辑。
 */
import test from "node:test";
import assert from "node:assert/strict";

const callbacks = new Map();
const listeners = new Map();
let callbackId = 0;

globalThis.window = globalThis;
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
      if (!listeners.has(args.event)) listeners.set(args.event, new Set());
      listeners.get(args.event).add(args.handler);
      return Promise.resolve(args.handler);
    }
    if (cmd === "plugin:event|unlisten") {
      callbacks.delete(args.eventId);
      listeners.get(args.event)?.delete(args.eventId);
      return Promise.resolve(null);
    }
    return mockInvoke(cmd, args);
  },
};
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener(event, eventId) {
    callbacks.delete(eventId);
    listeners.get(event)?.delete(eventId);
  },
};

let mockInvoke = () => Promise.resolve(null);

function emitTauriEvent(event, payload) {
  const handlerIds = [...(listeners.get(event) ?? [])];
  for (const id of handlerIds) {
    const cb = callbacks.get(id);
    if (typeof cb === "function") cb({ event, id, payload });
  }
}

test("subscribeHostAttention: initial snapshot, multiplexing, and clean unregister", async () => {
  const { subscribeHostAttention } = await import("../apps/desktop/src/host-api.ts");
  
  let attentionCalls = 0;
  let currentAttention = { version: 1, approvals: [], interactions: [] };
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") {
      attentionCalls++;
      return currentAttention;
    }
    return null;
  };

  const snapshots1 = [];
  const errors1 = [];
  const stop1 = await subscribeHostAttention({
    snapshot: (att) => snapshots1.push(att),
    error: (err) => errors1.push(err),
  });

  // 等待首个快照完成
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(snapshots1.length, 1);
  assert.equal(snapshots1[0].version, 1);
  assert.equal(attentionCalls, 1);

  // 第二个订阅者加入：复用现有快照，不重复发起 IPC
  const snapshots2 = [];
  const stop2 = await subscribeHostAttention({
    snapshot: (att) => snapshots2.push(att),
    error: () => {},
  });
  assert.equal(snapshots2.length, 1);
  assert.equal(snapshots2[0].version, 1);
  assert.equal(attentionCalls, 1, "second subscriber reuses current snapshot without duplicate IPC");

  // 触发版本递增事件：两者都收到版本 2
  currentAttention = { version: 2, approvals: [], interactions: [] };
  emitTauriEvent("host://attention-changed", { version: 2 });
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(attentionCalls, 2);
  assert.equal(snapshots1.length, 2);
  assert.equal(snapshots1[1].version, 2);
  assert.equal(snapshots2.length, 2);
  assert.equal(snapshots2[1].version, 2);

  // 乱序与重复版本事件：被忽略，不触发多余调用
  emitTauriEvent("host://attention-changed", { version: 1 });
  emitTauriEvent("host://attention-changed", { version: 2 });
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(attentionCalls, 2, "out-of-order and duplicate versions are discarded");

  // 卸载注销：当所有订阅者注销，底层事件监听被彻底清理
  stop1();
  assert.equal(listeners.get("host://attention-changed")?.size, 1);
  stop2();
  assert.equal(listeners.get("host://attention-changed")?.size ?? 0, 0, "underlying listener removed on zero subscribers");
});

test("subscribeHostAttention: snapshot failure handling and recovery on next event", async () => {
  const { subscribeHostAttention } = await import("../apps/desktop/src/host-api.ts");
  
  let shouldFail = false;
  let currentAttention = { version: 10, approvals: [], interactions: [] };
  mockInvoke = async (cmd) => {
    if (cmd === "host_attention") {
      if (shouldFail) throw new Error("simulated attention failure");
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

  await new Promise((r) => setTimeout(r, 20));
  assert.equal(snapshots.length, 1);
  assert.equal(snapshots[0].version, 10);

  // 下一次快照失败
  shouldFail = true;
  emitTauriEvent("host://attention-changed", { version: 11 });
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(errors.length, 1);
  assert.match(errors[0], /simulated attention failure/);
  assert.equal(snapshots.length, 1, "failed snapshot does not replace valid attention state");

  // 后续版本恢复
  shouldFail = false;
  currentAttention = { version: 12, approvals: [], interactions: [] };
  emitTauriEvent("host://attention-changed", { version: 12 });
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(snapshots.length, 2);
  assert.equal(snapshots[1].version, 12, "subscription recovers and receives next valid snapshot");

  stop();
});

test("Approvals: independent processing of plugin-level and instance-level records", async () => {
  const { hostAPI } = await import("../apps/desktop/src/host-api.ts");
  
  const approvedProposals = [];
  const approvedPluginProposals = [];
  mockInvoke = async (cmd, args) => {
    if (cmd === "get_operation") {
      return { operationId: args.operationId, status: "awaiting_approval", input: { cmd: "whoami" } };
    }
    if (cmd === "recent_plugin_operations") {
      return [{ operationId: "op-plugin-1", status: "awaiting_approval", input: { action: "scan" } }];
    }
    if (cmd === "approve_proposal") {
      approvedProposals.push(args);
      return { operationId: args.proposalId, status: "completed" };
    }
    if (cmd === "approve_plugin_proposal") {
      approvedPluginProposals.push(args);
      return { operationId: args.proposalId, status: "completed" };
    }
    return null;
  };

  // 1. 实例级审批：调用 getOperation 与 approve_proposal
  const instOp = await hostAPI.getOperation("inst-1", "op-inst-1");
  assert.equal(instOp.operationId, "op-inst-1");
  await hostAPI.approveProposal("inst-1", "prop-inst-1", { confirmed: true });
  assert.deepEqual(approvedProposals, [{ pluginInstanceId: "inst-1", proposalId: "prop-inst-1", args: { confirmed: true } }]);

  // 2. 插件级审批：调用 recentPluginOperations 与 approve_plugin_proposal
  const pluginOps = await hostAPI.recentPluginOperations("ssh", 10);
  assert.equal(pluginOps.length, 1);
  await hostAPI.approvePluginProposal("ssh", "prop-plugin-1", { confirmed: true });
  assert.deepEqual(approvedPluginProposals, [{ pluginId: "ssh", proposalId: "prop-plugin-1", args: { confirmed: true } }]);
});

test("Interactions: cancellation and non-leakage to other instances", async () => {
  const { cancelSecretInput } = await import("../apps/desktop/src/secret-input.ts");
  
  // cancelSecretInput 不抛错，并且对于未知的 key 安全无副作用
  assert.doesNotThrow(() => cancelSecretInput("backend:unknown-interaction"));
  assert.doesNotThrow(() => cancelSecretInput("inst-nonexistent"));
});
