/**
 * 注入到页面里的宿主替身：实现 Tauri v2 的 IPC 契约（`window.__TAURI_INTERNALS__`），
 * 让真实 App / HostApprovals / HostInteractions / host-api 在不启动 Rust 的情况下跑起来。
 *
 * 只模拟契约：命令名、参数名、返回形状都与 `apps/desktop/src-tauri/src/main.rs` 对齐。
 * 事件的 transport 细节（callback 注册、emit → `__TAURI_EVENT_PLUGIN_INTERNALS__`）按
 * @tauri-apps/api 的实现走，所以用例验证的是真实前端代码路径。
 *
 * 不模拟产品组件：SDK 的 block-list 由构建产物注入 `window.__cotermTestSdk`，
 * 这里只负责把真实的模块搬到页面上。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const sshFrontendBundlePath = path.join(rootDir, "plugins/plugin-ssh/dist/frontend/index.js");
const sshFrontendSource = fs.existsSync(sshFrontendBundlePath)
  ? fs.readFileSync(sshFrontendBundlePath, "utf8")
  : null;
const serialFrontendBundlePath = path.join(rootDir, "plugins/plugin-serial/dist/frontend/index.js");
const serialFrontendSource = fs.existsSync(serialFrontendBundlePath)
  ? fs.readFileSync(serialFrontendBundlePath, "utf8") : null;

/**
 * 真实 Plugin SDK 模块的 ES 源码，供页面以动态 import 加载。
 * 由 scripts/build-sdk-browser.mjs 从 `packages/plugin-sdk/src/components/block-list.ts` 生成；
 * 缺失时直接报错，绝不回退到手工替身。
 */
export function sdkModuleSource() {
  const modulePath = path.join(rootDir, "packages/plugin-sdk/dist/browser/block-list.js");
  if (!fs.existsSync(modulePath)) {
    throw new Error(
      `packages/plugin-sdk/dist/browser/block-list.js is missing; run "pnpm build:sdk"`,
    );
  }
  return fs.readFileSync(modulePath, "utf8");
}

export function makeHostMockSource(initial) {
  return `(() => {
    const scenario = ${JSON.stringify(initial)};
    const defaultLayout = { activeConfigRef:null, instancesCollapsed:innerWidth < 900, activeWorkspaceId:"default" };
    scenario.workspace = {
      groups:[{id:"default",name:"Default workspace"}], allowedOperations:[], historyPolicy:"session_only", persistence:"windows_dpapi",
      ...scenario.workspace,
      entries:(scenario.workspace?.entries ?? scenario.instances.map(instance => ({
        configRef:"ref-" + instance.pluginInstanceId, pluginId:instance.pluginId,label:instance.label,instanceId:instance.pluginInstanceId,
      }))).map(entry => ({workspaceId:"default",...entry})),
      layout:{...defaultLayout,...scenario.workspace?.layout},
    };
    let nextInstance = 0;
    let nextSubscription = 0;
    if (!localStorage.getItem("coterm.locale")) localStorage.setItem("coterm.locale", "en");
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

    // 可挂起的 IPC：命令名 → 队列。用例显式推进，不靠固定 sleep 赌时序。
    const gates = new Map();
    const invocations = [];

    window.__cotermTest = {
      calls,
      approveCalls,
      rejectCalls,
      pendingSecrets,
      invocations,
      workspace: () => structuredClone(scenario.workspace),
      replaceInstances: next => { scenario.instances = structuredClone(next); },
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
      /** 挂起同一命令的后续调用，返回一个可手动放行的闸门。 */
      gate(command) {
        const queue = [];
        gates.set(command, queue);
        return {
          /** 当前被挂起的调用数，用于等待代码真的走到这一步。 */
          size: () => queue.length,
          pending: () => invocations.filter((item) => item.command === command && !item.done),
          release(result, index = 0) {
            const entry = queue[index];
            if (!entry) throw new Error("no gated invocation for " + command);
            queue.splice(index, 1);
            if (queue.length === 0) gates.delete(command);
            entry.done = true;
            entry.resolve(result);
          },
          fail(reason, index = 0) {
            const entry = queue[index];
            if (!entry) throw new Error("no gated invocation for " + command);
            queue.splice(index, 1);
            if (queue.length === 0) gates.delete(command);
            entry.done = true;
            entry.reject(reason);
          },
        };
      },
      /** 解除全部闸门，后续调用直接走真实 handler。 */
      clearGates() {
        const entries = [...gates.values()].flat();
        gates.clear();
        for (const entry of entries) entry.reject("已取消挂起");
      },
      /** 所有已发出的该命令调用数（含已完成）。 */
      countCalls(command) {
        return invocations.filter((item) => item.command === command).length;
      },
      listenerCount() {
        return [...listeners.values()].reduce((total, ids) => total + ids.size, 0);
      },
      dialogs() {
        return [...document.querySelectorAll("dialog.host-secret-dialog")].map((dialog) => ({
          title: dialog.querySelector("h2")?.textContent ?? "",
          prompt: dialog.querySelector("label span")?.textContent ?? "",
          open: dialog.open,
        }));
      },
      submitSecret(value) {
        const dialog = document.querySelector("dialog.host-secret-dialog");
        if (!dialog) throw new Error("no secret dialog open");
        const input = dialog.querySelector("input");
        input.value = value;
        dialog.querySelector("form").dispatchEvent(new Event("submit", { cancelable: true, bubbles: true }));
      },
      clickCancel() {
        const dialog = document.querySelector("dialog.host-secret-dialog");
        if (!dialog) throw new Error("no secret dialog open");
        const button = [...dialog.querySelectorAll("button")].find((item) => item.type === "button");
        button.click();
      },
      /** 真实 SDK 组件的动态 import；不用页面里的任何手写替身。 */
      async loadSdk() {
        return import("/__sdk/block-list.js");
      },
      /** 真实 SSH 前端的构建产物源码，供隔离 iframe 自行 import。 */
      sshFrontendSource() {
        return scenario.sshFrontendSource;
      },
      /**
       * 真实 SSH 前端构建产物：与产品同一条加载路径（Blob → 动态 import），
       * 让用例可以把它挂进一个不含任何主题变量的隔离容器里读它自己的 fallback。
       */
      async loadSshFrontend() {
        const source = scenario.sshFrontendSource;
        if (typeof source !== "string") throw new Error("SSH frontend bundle is unavailable");
        const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
        try {
          return await import(url);
        } finally {
          URL.revokeObjectURL(url);
        }
      },
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
      runtime_snapshot: () => ({ instances: scenario.instances, runningPlugins: scenario.catalog.plugins.filter((plugin) => plugin.running).map((plugin) => plugin.id) }),
      workspace_snapshot: () => structuredClone(scenario.workspace),
      save_workspace_layout: ({activeInstanceId,instancesCollapsed,activeWorkspaceId}) => {
        scenario.workspace.layout = {activeConfigRef:scenario.workspace.entries.find(entry => entry.instanceId === activeInstanceId)?.configRef ?? null,
          instancesCollapsed, activeWorkspaceId:activeWorkspaceId ?? scenario.workspace.layout.activeWorkspaceId};
        return null;
      },
      create_workspace_group: ({name}) => {
        name = name.trim();
        if (!name || [...name].length > 64) throw new Error("workspace_group_name_invalid");
        if (scenario.workspace.groups.some(group => group.name.toLowerCase() === name.toLowerCase())) throw new Error("workspace_group_name_exists");
        const group = {id:"group-" + ++nextInstance,name};
        scenario.workspace.groups = [...scenario.workspace.groups,group];
        scenario.workspace.layout.activeWorkspaceId = group.id;
        return group;
      },
      delete_workspace_group: ({workspaceId}) => {
        const groups = scenario.workspace.groups.filter(group => group.id !== workspaceId);
        if (!groups.length) throw new Error("workspace_last_group");
        scenario.workspace.groups = groups;
        scenario.workspace.entries = scenario.workspace.entries.map(entry => entry.workspaceId === workspaceId ? {...entry,workspaceId:groups[0].id} : entry);
        if (scenario.workspace.layout.activeWorkspaceId === workspaceId) scenario.workspace.layout.activeWorkspaceId = groups[0].id;
        return null;
      },
      move_workspace_entry: ({configRef,workspaceId}) => {
        if (!scenario.workspace.groups.some(group => group.id === workspaceId)) throw new Error("workspace_group_not_found");
        scenario.workspace.entries = scenario.workspace.entries.map(entry => entry.configRef === configRef ? {...entry,workspaceId} : entry);
        return null;
      },
      forget_saved_instance: ({configRef}) => {
        scenario.workspace.entries = scenario.workspace.entries.filter(entry => entry.configRef !== configRef);
        return null;
      },
      restore_saved_instance: ({configRef}) => {
        const entry = scenario.workspace.entries.find(item => item.configRef === configRef);
        if (!entry) throw new Error("workspace_entry_not_found");
        const instance = {pluginId:entry.pluginId,pluginInstanceId:"restored-" + ++nextInstance,sessionId:"session-" + nextInstance,label:entry.label,status:"ready"};
        scenario.instances = [...scenario.instances,instance];
        scenario.workspace.entries = scenario.workspace.entries.map(item => item.configRef === configRef ? {...item,instanceId:instance.pluginInstanceId} : item);
        return instance;
      },
      destroy_instance: ({pluginInstanceId}) => {
        scenario.instances = scenario.instances.filter(instance => instance.pluginInstanceId !== pluginInstanceId);
        scenario.workspace.entries = scenario.workspace.entries.filter(entry => entry.instanceId !== pluginInstanceId);
        return null;
      },
      create_instance: ({pluginId,config,workspaceId}) => {
        const plugin = scenario.catalog.plugins.find(item => item.id === pluginId);
        if (!plugin?.running) throw new Error("plugin_not_running");
        const instance = {pluginId,pluginInstanceId:"created-" + ++nextInstance,sessionId:"session-" + nextInstance,
          label:config.host ? config.username + "@" + config.host : config.port ?? plugin.name,status:"ready"};
        scenario.instances = [...scenario.instances,instance];
        scenario.workspace.entries = [...scenario.workspace.entries,{configRef:"created-ref-" + nextInstance,
          pluginId,instanceId:instance.pluginInstanceId,label:instance.label,workspaceId:workspaceId ?? "default"}];
        return instance;
      },
      start_plugin: ({pluginId}) => {
        scenario.catalog.plugins = scenario.catalog.plugins.map(plugin => plugin.id === pluginId ? {...plugin,running:true} : plugin);
        return null;
      },
      provide_secret: ({ pluginInstanceId, purpose }) => ({ id: "secret-test", pluginInstanceId, purpose, expiresAt: "2099-01-01T00:00:00Z" }),
      list_plugins: () => scenario.catalog,
      prepare_plugin_install: () => scenario.installPreview ?? null,
      install_plugin_package: ({ token }) => {
        if (!scenario.installPreview || token !== scenario.installPreview.token) throw new Error("plugin_install_unavailable");
        const { id, name, version } = scenario.installPreview;
        const plugin = { id, name, version, hasFrontend: false, running: false };
        // Real Tauri IPC deserializes a fresh catalog on the following refresh.
        scenario.catalog = { ...scenario.catalog, plugins: [...scenario.catalog.plugins, plugin] };
        return plugin;
      },
      discard_plugin_install: () => null,
      list_instances: () => scenario.instances,
      get_bypass: () => scenario.bypass,
      list_operations: () => scenario.operations ?? [],
      recent_operations: () => scenario.history ?? [],
      recent_plugin_operations: () => scenario.history ?? [],
      host_attention: () => attention,
      plugin_frontend_source: ({ pluginId }) => pluginId === "ssh" ? scenario.sshFrontendSource : pluginId === "serial" ? scenario.serialFrontendSource : null,
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
        if (operation === "serial.status") return {status:"completed",output:{config:{port:"COM1",baudRate:115200,dtr:true,rts:true},
          connected:true,connectionId:"serial-conn",signals:{dtr:true,rts:true},rxBytes:0,txBytes:0,commandQueue:0}};
        if (operation === "serial.read") return {status:"completed",output:{text:"serial ready\\r\\n",bytes:14,cursor:"14",gap:false,closed:false,more:false,connectionId:"serial-conn"}};
        return { status: "completed", output: {} };
      },
      subscribe_plugin_events: () => "sub-" + ++nextSubscription,
      ack_plugin_event: () => null,
      unsubscribe_plugin_events: () => null,
      get_operation: ({ pluginInstanceId, operationId }) =>
        (scenario.history ?? []).find((record) => record.operationId === operationId) ?? null,
      approve_proposal: (args) => { approveCalls.push({ command: "approve_proposal", ...args }); return { operationId: args.proposalId, status: "completed" }; },
      reject_proposal: (args) => { rejectCalls.push({ command: "reject_proposal", ...args }); return null; },
      approve_plugin_proposal: (args) => { approveCalls.push({ command: "approve_plugin_proposal", ...args }); return { operationId: args.proposalId, status: "completed" }; },
      reject_plugin_proposal: (args) => { rejectCalls.push({ command: "reject_plugin_proposal", ...args }); return null; },
      respond_secret: (args) => {
        pendingSecrets.push(args);
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
        const record = { command, args: args ?? {}, done: false };
        invocations.push(record);
        const queue = gates.get(command);
        if (queue) {
          return new Promise((resolve, reject) => {
            queue.push({ record, resolve: (value) => { record.done = true; resolve(value); }, reject: (reason) => { record.done = true; reject(reason); } });
          });
        }
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
  serialFrontendSource,
};

/** 构造一条基座会路由的 secret 交互（kind=secret）。 */
export function secretInteraction(id, instance, { title = `${instance} password`, remainingMs = 60_000 } = {}) {
  return {
    interactionId: id,
    pluginInstanceId: instance,
    operationId: null,
    kind: { kind: "secret", purpose: "ssh.password" },
    title,
    prompt: `Enter password for ${instance}`,
    allowCancel: true,
    remainingMs,
    deadline: "2026-10-22T00:00:00Z",
  };
}
