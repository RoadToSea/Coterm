/**
 * 注入到页面里的宿主替身：实现 Tauri v2 的 IPC 契约（`window.__TAURI_INTERNALS__`），
 * 让真实 App / HostApprovals / HostInteractions / host-api 在不启动 Rust 的情况下跑起来。
 *
 * 只模拟契约：命令名、参数名、返回形状都与 `apps/desktop/src-tauri/src/main.rs` 对齐。
 * 事件的 transport 细节（callback 注册、emit → `__TAURI_EVENT_PLUGIN_INTERNALS__`）按
 * @tauri-apps/api 的实现走，所以用例验证的是真实前端代码路径。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const sshFrontendBundlePath = path.join(rootDir, "plugins/plugin-ssh/dist/frontend/index.js");
const sshFrontendSource = fs.existsSync(sshFrontendBundlePath)
  ? fs.readFileSync(sshFrontendBundlePath, "utf8")
  : null;

export function makeHostMockSource(initial) {
  return `(() => {
    const scenario = ${JSON.stringify(initial)};
    const callbacks = new Map();
    const listeners = new Map();
    const calls = [];
    let callbackId = 0;
    let attentionVersion = scenario.attention.version;
    let attention = scenario.attention;
    let failures = Object.assign({}, scenario.failures);

    const pendingSecrets = [];
    const approveCalls = [];
    const rejectCalls = [];

    window.__cotermTest = {
      calls,
      approveCalls,
      rejectCalls,
      pendingSecrets,
      setAttention(next, { emit = true } = {}) {
        attention = next;
        attentionVersion = next.version;
        if (emit) window.__cotermTest.emitAttention(next.version);
      },
      emitAttention(version = ++attentionVersion) {
        attentionVersion = version;
        if (attention) attention.version = version;
        const handlerIds = [...(listeners.get("host://attention-changed") ?? [])];
        for (const id of handlerIds) {
          const callback = callbacks.get(id);
          if (typeof callback === "function") callback({ event: "host://attention-changed", id, payload: { version } });
        }
      },
      setFailure(command, enabled, sequence) {
        if (enabled) failures[command] = { sequence: sequence ?? [1] };
        else delete failures[command];
      },
      listenerCount() {
        return [...listeners.values()].reduce((total, ids) => total + ids.size, 0);
      },
      secretDialog() {
        const dialog = document.querySelector("dialog.host-secret-dialog");
        if (!dialog) return null;
        return {
          title: dialog.querySelector("h2")?.textContent ?? "",
          prompt: dialog.querySelector("label span")?.textContent ?? "",
          open: dialog.open,
        };
      },
    };

    window.__cotermTestMountBlockList = (container, initialData) => {
      const blockStyle = \`
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
        .coterm-block-actor { margin-right: 8px; color: var(--coterm-text-muted, #9da0a3); font-size: 10px; }
        .coterm-block-actor[data-actor="robot"] { color: var(--coterm-accent-glow, #75beff); }
        .coterm-block-status { min-width: 20px; color: var(--coterm-danger, #f48771); text-align: center; }
      \`;
      const root = document.createElement("div");
      root.className = "coterm-blocks";
      const sheet = document.createElement("style");
      sheet.textContent = blockStyle;
      container.replaceChildren(sheet, root);

      for (const block of initialData.blocks) {
        const row = document.createElement("div");
        row.className = "coterm-block";
        const gutter = document.createElement("span");
        gutter.className = "coterm-block-gutter";
        gutter.textContent = String(block.index);
        const command = document.createElement("span");
        command.className = "coterm-block-command";
        command.textContent = block.command;
        const actor = document.createElement("span");
        actor.className = "coterm-block-actor";
        actor.dataset.actor = block.actor;
        actor.textContent = block.actor;
        const status = document.createElement("span");
        status.className = "coterm-block-status";
        status.textContent = block.exitCode ? "!" : "";
        row.append(gutter, command, actor, status);
        root.append(row);
      }
    };

    const failNext = (command) => {
      const entry = failures[command];
      if (!entry) return false;
      if (entry.sequence[0] <= 0) {
        entry.sequence.shift();
        if (entry.sequence.length === 0) delete failures[command];
        return false;
      }
      entry.sequence[0] -= 1;
      return true;
    };

    const handlers = {
      list_plugins: () => scenario.catalog,
      list_instances: () => scenario.instances,
      get_bypass: () => scenario.bypass,
      list_operations: () => scenario.operations ?? [],
      recent_operations: () => scenario.history ?? [],
      recent_plugin_operations: () => scenario.history ?? [],
      host_attention: () => attention,
      plugin_frontend_source: ({ pluginId }) => (pluginId === "ssh" ? scenario.sshFrontendSource : null),
      invoke_operation: ({ operation, args }) => {
        if (operation === "ssh.status") {
          return {
            status: "completed",
            output: {
              connected: true,
              connectionId: "conn-1",
              config: { username: "root", host: "192.168.1.10", port: 22 },
            },
          };
        }
        if (operation === "ssh.read") {
          return {
            status: "completed",
            output: {
              connectionId: "conn-1",
              cursor: "0",
              data: btoa("Connected to Prod-Server (SSH)\\r\\nLast login: Thu Oct 22 20:00:00 2026 from 192.168.1.2\\r\\nroot@prod:~# "),
              more: false,
              gap: false,
            },
          };
        }
        return { status: "completed", output: {} };
      },
      subscribe_plugin_events: () => "sub-1",
      ack_plugin_event: () => null,
      unsubscribe_plugin_events: () => null,
      get_operation: ({ pluginInstanceId, operationId }) =>
        (scenario.history ?? []).find((record) => record.operationId === operationId) ?? null,
      approve_proposal: (args) => { approveCalls.push(args); return { operationId: args.proposalId, status: "completed" }; },
      reject_proposal: (args) => { rejectCalls.push(args); return null; },
      approve_plugin_proposal: (args) => { approveCalls.push(args); return { operationId: args.proposalId, status: "completed" }; },
      reject_plugin_proposal: (args) => { rejectCalls.push(args); return null; },
      respond_secret: (args) => {
        const dialog = document.querySelector("dialog.host-secret-dialog");
        pendingSecrets.push(args);
        if (dialog && dialog.open) dialog.close();
        return null;
      },
      "plugin:event|listen": ({ event, handler }) => {
        if (!listeners.has(event)) listeners.set(event, new Set());
        listeners.get(event).add(handler);
        return handler;
      },
      "plugin:event|unlisten": ({ event, eventId }) => {
        callbacks.delete(eventId);
        listeners.get(event)?.delete(eventId);
        return null;
      },
    };

    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" } },
      convertFileSrc: (value) => value,
      transformCallback(callback) {
        const id = callbackId++;
        callbacks.set(id, callback);
        return id;
      },
      invoke(command, args) {
        calls.push(command);
        if (failNext(command)) return Promise.reject("simulated " + command + " failure");
        const handler = handlers[command];
        if (!handler) return Promise.reject("unknown command " + command);
        try {
          return Promise.resolve(handler(args ?? {}));
        } catch (error) {
          return Promise.reject(String(error));
        }
      },
    };

    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener(event, eventId) {
        callbacks.delete(eventId);
        listeners.get(event)?.delete(eventId);
      },
    };
  })();`;
}

/** 与 main.rs / plugin_manager.rs 一致的默认场景：有效插件 + 损坏包诊断 + 实例 + 待处理项。 */
export const defaultScenario = {
  catalog: {
    plugins: [{ id: "ssh", name: "SSH", version: "0.1.0", hasFrontend: true, running: true }],
    issues: [
      { directory: "broken-plugin", pluginId: null, code: "plugin_manifest_invalid", message: "Package manifest is not valid JSON" },
      { directory: "legacy-plugin", pluginId: "legacy", code: "plugin_manifest_legacy", message: "Package predates the current manifest schema" },
    ],
  },
  instances: [
    { pluginId: "ssh", pluginInstanceId: "inst-1", sessionId: "sess-1", label: "Prod-Server", status: "ready" },
  ],
  bypass: false,
  attention: { version: 1, approvals: [], interactions: [] },
  failures: {},
  sshFrontendSource,
};
