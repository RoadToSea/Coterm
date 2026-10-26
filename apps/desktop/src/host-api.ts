/** 桌面壳的接口；不传给插件前端，不包含通信协议或插件内部页面模型。 */
import type {
  OperationDescriptor,
  OperationRecord,
  OperationReceipt,
  PluginAPI,
  PluginFrontend,
  PluginFrontendModule,
  InteractionOptions,
  InteractionOutcome,
  SelectionOption,
  SecretHandle,
  StreamDelivery,
  StreamOptions,
  StreamSubscription,
  PluginInstanceEvent,
} from "@coterm/plugin-sdk";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cancelSecretInput, promptForSecret, promptForText, promptForConfirmation, promptForSelection } from "./secret-input";
import { FRONTEND_API_VERSION } from "@coterm/plugin-sdk";
import { mountBlockList } from "@coterm/plugin-sdk/components/block-list";

/** 宿主待处理状态变化的全局事件；payload 只有版本号。 */
export const ATTENTION_CHANGED_EVENT = "host://attention-changed";

export interface AttentionApproval {
  operationId: string;
  proposalId: string;
  pluginId: string;
  /** null 表示插件级操作。 */
  pluginInstanceId: string | null;
  operation: string;
  targetLabel: string;
  createdAt: string;
}

export interface AttentionInteraction {
  interactionId: string;
  pluginInstanceId: string;
  operationId: string | null;
  /** 基座只路由 secret 交互；普通文本/确认由插件前端自行处理。 */
  kind: { kind: "secret"; purpose: string } | { kind: "text" | "confirmation" } | { kind: "selection"; options: SelectionOption[] };
  title: string;
  prompt: string;
  allowCancel: boolean;
  /** 由基座按绝对期限计算。 */
  remainingMs: number;
  deadline: string;
}

export interface HostAttention {
  version: number;
  approvals: AttentionApproval[];
  interactions: AttentionInteraction[];
}

export interface PluginSummary {
  id: string;
  name: string;
  version: string;
  hasFrontend: boolean;
  running: boolean;
}

/** 单个插件包的可定位诊断；不含绝对路径、秘密或内部错误明文。 */
export interface PluginLoadIssue {
  directory: string;
  pluginId: string | null;
  code: string;
  message: string;
}

/** 有效插件与逐包诊断分开返回：单个坏包不影响其余插件列出。 */
export interface PluginCatalog {
  plugins: PluginSummary[];
  issues: PluginLoadIssue[];
}

export interface InstanceInfo {
  pluginId: string;
  pluginInstanceId: string;
  sessionId: string;
  label: string;
  status: "starting" | "ready" | "stopping" | "stopped" | "failed";
}

export interface SavedWorkspace {
  entries: { configRef: string; pluginId: string; label: string; instanceId: string | null }[];
  layout: { activeConfigRef: string | null; instancesCollapsed: boolean };
  historyPolicy: "session_only";
  persistence: "windows_dpapi" | "memory_only";
}

export interface BackendSecretRequest extends InteractionOptions {
  interactionId: string;
  pluginInstanceId: string;
  operationId: string;
  kind: { kind: "secret"; purpose: string };
}

export interface McpClientConfiguration {
  url: string;
}

export interface HostAPI {
  runtimeStatus(): Promise<{ instances: InstanceInfo[]; runningPlugins: string[] }>;
  workspace(): Promise<SavedWorkspace>;
  saveLayout(activeInstanceId: string | null, instancesCollapsed: boolean, order: string[]): Promise<void>;
  restoreInstance(configRef: string): Promise<InstanceInfo>;
  forgetInstance(configRef: string): Promise<void>;
  resetWorkspace(): Promise<void>;
  pendingInteractions(): Promise<BackendSecretRequest[]>;
  respondSecret(interactionId: string, value: string | null): Promise<void>;
  mcpConfiguration(): Promise<McpClientConfiguration>;
  listPlugins(): Promise<PluginCatalog>;
  startPlugin(pluginId: string): Promise<void>;
  stopPlugin(pluginId: string): Promise<void>;
  listInstances(pluginId?: string): Promise<InstanceInfo[]>;
  createInstance(pluginId: string, config: unknown): Promise<InstanceInfo>;
  destroyInstance(pluginInstanceId: string): Promise<void>;
  listOperations(pluginId: string): Promise<OperationDescriptor[]>;
  invokeOperation<T = unknown>(
    pluginInstanceId: string,
    operation: string,
    args: unknown,
    waitTimeoutMs?: number,
  ): Promise<OperationReceipt<T>>;
  invokePluginOperation<T = unknown>(pluginId: string, operation: string, args: unknown, waitTimeoutMs?: number): Promise<OperationReceipt<T>>;
  recentPluginOperations(pluginId: string, limit?: number): Promise<OperationRecord[]>;
  approvePluginProposal<T = unknown>(pluginId: string, proposalId: string, args: unknown): Promise<OperationReceipt<T>>;
  rejectPluginProposal(pluginId: string, proposalId: string): Promise<void>;
  getOperation(pluginInstanceId: string, operationId: string): Promise<OperationRecord>;
  recentOperations(pluginInstanceId: string, limit?: number): Promise<OperationRecord[]>;
  approveProposal<T = unknown>(
    pluginInstanceId: string,
    proposalId: string,
    args: unknown,
  ): Promise<OperationReceipt<T>>;
  rejectProposal(pluginInstanceId: string, proposalId: string): Promise<void>;
  cancelOperation(pluginInstanceId: string, operationId: string): Promise<OperationReceipt>;
  setBypass(enabled: boolean): Promise<void>;
  getBypass(): Promise<boolean>;
  frontendSource(pluginId: string): Promise<string | null>;
  provideSecret(pluginInstanceId: string, purpose: string, value: string): Promise<SecretHandle>;
  /** 一次取回全部实例级与插件级待处理项，替代逐实例扫描历史。 */
  hostAttention(): Promise<HostAttention>;
}

export interface AttentionSubscriptionHandlers {
  snapshot(attention: HostAttention): void;
  /** 快照失败时给出可恢复提示；调用方应保留上一次有效快照。 */
  error(message: string): void;
}

interface SharedAttentionState {
  generation: number;
  subscribers: Set<AttentionSubscriptionHandlers>;
  unlisten: (() => void) | null;
  listenPromise: Promise<(() => void) | null> | null;
  current: number;
  lastSnapshot: HostAttention | null;
  lastError: string | null;
  loading: boolean;
  reload: boolean;
  requestToken: number;
  buffered: number[];
}

const sharedAttention: SharedAttentionState = {
  generation: 0,
  subscribers: new Set(),
  unlisten: null,
  listenPromise: null,
  current: -1,
  lastSnapshot: null,
  lastError: null,
  loading: false,
  reload: false,
  requestToken: 0,
  buffered: [],
};

async function refreshSharedAttention(gen: number) {
  if (sharedAttention.generation !== gen || sharedAttention.subscribers.size === 0) return;
  if (sharedAttention.loading) {
    sharedAttention.reload = true;
    return;
  }
  sharedAttention.loading = true;
  const token = ++sharedAttention.requestToken;
  try {
    const snapshot = await hostAPI.hostAttention();
    if (sharedAttention.generation !== gen || token !== sharedAttention.requestToken || sharedAttention.subscribers.size === 0) {
      return;
    }
    sharedAttention.current = snapshot.version;
    sharedAttention.lastSnapshot = snapshot;
    sharedAttention.lastError = null;
    for (const handler of [...sharedAttention.subscribers]) {
      try { handler.snapshot(snapshot); } catch (e) { console.error(e); }
    }
    if (sharedAttention.buffered.some((version) => version > sharedAttention.current)) {
      sharedAttention.buffered = sharedAttention.buffered.filter((v) => v > sharedAttention.current);
      sharedAttention.reload = true;
    } else {
      sharedAttention.buffered.length = 0;
    }
  } catch (reason) {
    if (sharedAttention.generation === gen && token === sharedAttention.requestToken && sharedAttention.subscribers.size > 0) {
      const message = String(reason);
      sharedAttention.lastError = message;
      // 错误后不永久阻断同一版本的重试与重新通知
      sharedAttention.buffered.length = 0;
      for (const handler of [...sharedAttention.subscribers]) {
        try { handler.error(message); } catch (e) { console.error(e); }
      }
    }
  } finally {
    if (sharedAttention.generation === gen) {
      sharedAttention.loading = false;
      if (sharedAttention.reload && sharedAttention.subscribers.size > 0) {
        sharedAttention.reload = false;
        void refreshSharedAttention(gen);
      }
    }
  }
}

/**
 * 先订阅、再取快照，丢弃不新于快照的缓冲版本。
 * 多订阅者共享底层事件监听与快照请求，使用初始化 promise 与生命周期代次消除并发竞态。
 */
export async function subscribeHostAttention(handlers: AttentionSubscriptionHandlers): Promise<() => void> {
  const gen = sharedAttention.generation;
  sharedAttention.subscribers.add(handlers);

  if (!sharedAttention.listenPromise) {
    const listenGen = sharedAttention.generation;
    sharedAttention.listenPromise = (async () => {
      try {
        const unlistenFn = await listen<{ version: number }>(ATTENTION_CHANGED_EVENT, (event) => {
          if (sharedAttention.generation !== listenGen || sharedAttention.subscribers.size === 0) return;
          const version = event.payload?.version;
          if (typeof version !== "number") return;
          if (version <= sharedAttention.current) return;
          if (sharedAttention.buffered.includes(version)) return;
          sharedAttention.buffered.push(version);
          if (sharedAttention.buffered.length > 32) sharedAttention.buffered.shift();
          void refreshSharedAttention(listenGen);
        });
        if (sharedAttention.generation !== listenGen || sharedAttention.subscribers.size === 0) {
          unlistenFn();
          sharedAttention.unlisten = null;
          sharedAttention.listenPromise = null;
          return null;
        }
        sharedAttention.unlisten = unlistenFn;
        return unlistenFn;
      } catch (err) {
        sharedAttention.listenPromise = null;
        sharedAttention.unlisten = null;
        throw err;
      }
    })();
  }

  try {
    await sharedAttention.listenPromise;
  } catch (err) {
    sharedAttention.subscribers.delete(handlers);
    throw err;
  }

  if (!sharedAttention.subscribers.has(handlers) || sharedAttention.generation !== gen) {
    return () => {};
  }

  if (sharedAttention.lastSnapshot && !sharedAttention.loading) {
    handlers.snapshot(sharedAttention.lastSnapshot);
  } else if (!sharedAttention.loading) {
    void refreshSharedAttention(gen);
  }

  let stopped = false;
  return () => {
    if (stopped) return;
    stopped = true;
    sharedAttention.subscribers.delete(handlers);
    if (sharedAttention.subscribers.size === 0) {
      sharedAttention.generation++;
      const prevUnlisten = sharedAttention.unlisten;
      sharedAttention.unlisten = null;
      sharedAttention.listenPromise = null;
      sharedAttention.current = -1;
      sharedAttention.lastSnapshot = null;
      sharedAttention.lastError = null;
      sharedAttention.buffered.length = 0;
      sharedAttention.reload = false;
      sharedAttention.loading = false;
      if (prevUnlisten) {
        prevUnlisten();
      }
    }
  };
}

/** 供上层 UI 与失败恢复主动重试最近的宿主状态。 */
export function retryHostAttention(): void {
  if (sharedAttention.subscribers.size > 0) {
    sharedAttention.buffered.length = 0;
    void refreshSharedAttention(sharedAttention.generation);
  }
}

/** 仅管理挂载容器；unmount 不隐式销毁后端实例。模块查找不暴露任意路径。 */
export interface PluginFrontendLoader {
  load(pluginId: string): Promise<PluginFrontendModule | null>;
  mount(container: HTMLElement, pluginInstanceId: string): Promise<PluginFrontend>;
}

/** Tauri-backed desktop API. Package paths stay inside the app data plugins directory. */
export const hostAPI: HostAPI = {
  runtimeStatus: () => invoke("runtime_snapshot"),
  workspace: () => invoke<SavedWorkspace>("workspace_snapshot"),
  saveLayout: (activeInstanceId, instancesCollapsed, order) => invoke<void>("save_workspace_layout", { activeInstanceId, instancesCollapsed, order }),
  restoreInstance: (configRef) => invoke<InstanceInfo>("restore_saved_instance", { configRef }),
  forgetInstance: (configRef) => invoke<void>("forget_saved_instance", { configRef }),
  resetWorkspace: () => invoke<void>("reset_saved_workspace"),
  hostAttention: () => invoke<HostAttention>("host_attention"),
  pendingInteractions: () => invoke<BackendSecretRequest[]>("pending_interactions"),
  respondSecret: (interactionId, value) => invoke<void>("respond_secret", { interactionId, value }),
  mcpConfiguration: () => invoke<McpClientConfiguration>("mcp_connection_info"),
  frontendSource: (pluginId) => invoke<string | null>("plugin_frontend_source", { pluginId }),
  provideSecret: (pluginInstanceId, purpose, value) => invoke<SecretHandle>("provide_secret", { pluginInstanceId, purpose, value }),
  listPlugins: () => invoke<PluginCatalog>("list_plugins"),
  startPlugin: (pluginId) => invoke<void>("start_plugin", { pluginId }),
  stopPlugin: (pluginId) => invoke<void>("stop_plugin", { pluginId }),
  listInstances: (pluginId) => invoke<InstanceInfo[]>("list_instances", { pluginId }),
  createInstance: (pluginId, config) =>
    invoke<InstanceInfo>("create_instance", { pluginId, config }),
  destroyInstance: (pluginInstanceId) =>
    invoke<void>("destroy_instance", { pluginInstanceId }),
  listOperations: (pluginId) => invoke<OperationDescriptor[]>("list_operations", { pluginId }),
  invokeOperation: <T = unknown>(
    pluginInstanceId: string,
    operation: string,
    args: unknown,
    waitTimeoutMs?: number,
  ) =>
    invoke<OperationReceipt<T>>("invoke_operation", {
      pluginInstanceId,
      operation,
      args,
      waitTimeoutMs,
    }),
  invokePluginOperation: <T = unknown>(pluginId: string, operation: string, args: unknown, waitTimeoutMs?: number) => invoke<OperationReceipt<T>>("invoke_plugin_operation", {
    pluginId, operation, args, waitTimeoutMs,
  }),
  recentPluginOperations: (pluginId, limit) => invoke<OperationRecord[]>("recent_plugin_operations", { pluginId, limit }),
  approvePluginProposal: <T = unknown>(pluginId: string, proposalId: string, args: unknown) => invoke<OperationReceipt<T>>("approve_plugin_proposal", { pluginId, proposalId, args }),
  rejectPluginProposal: (pluginId, proposalId) => invoke<void>("reject_plugin_proposal", { pluginId, proposalId }),
  getOperation: (pluginInstanceId, operationId) =>
    invoke<OperationRecord>("get_operation", { pluginInstanceId, operationId }),
  recentOperations: (pluginInstanceId, limit) =>
    invoke<OperationRecord[]>("recent_operations", { pluginInstanceId, limit }),
  approveProposal: <T = unknown>(pluginInstanceId: string, proposalId: string, args: unknown) =>
    invoke<OperationReceipt<T>>("approve_proposal", {
      pluginInstanceId,
      proposalId,
      args,
    }),
  rejectProposal: (pluginInstanceId, proposalId) =>
    invoke<void>("reject_proposal", { pluginInstanceId, proposalId }),
  cancelOperation: (pluginInstanceId, operationId) =>
    invoke<OperationReceipt>("cancel_operation", { pluginInstanceId, operationId }),
  setBypass: (enabled) => invoke<void>("set_bypass", { enabled }),
  getBypass: () => invoke<boolean>("get_bypass"),
};

/** Binds plugin-facing calls to one instance; unsupported event features fail explicitly. */
export function createPluginAPI(instance: InstanceInfo): PluginAPI & { dispose(): void } {
  const unsupported = (feature: string): Error => new Error(`${feature} is not connected yet`);
  let disposed = false;
  const whileMounted = <T>(call: () => Promise<T>): Promise<T> => disposed
    ? Promise.reject(new Error("Plugin frontend is disposed")) : call();
  const scope = `frontend:${instance.pluginInstanceId}`;
  const subscriptions = new Set<() => void>();
  const subscribeEvents = async (handler: (event: PluginInstanceEvent) => void | Promise<void>): Promise<() => void> => {
    if (disposed) throw new Error("Plugin frontend is disposed");
    let active = true;
    const channel = new Channel<PluginInstanceEvent>();
    const ready = invoke<string>("subscribe_plugin_events", { pluginInstanceId: instance.pluginInstanceId, onEvent: channel });
    channel.onmessage = (event) => {
      void (async () => {
        const subscriptionId = await ready;
        if (!active || disposed) return;
        try { await handler(event); }
        catch {
          active = false;
          await invoke("unsubscribe_plugin_events", { pluginInstanceId: instance.pluginInstanceId, subscriptionId });
          return;
        }
        if (active) await invoke("ack_plugin_event", { pluginInstanceId: instance.pluginInstanceId, subscriptionId });
      })().catch(() => { active = false; });
    };
    const subscriptionId = await ready;
    const cancel = () => {
      subscriptions.delete(cancel);
      if (!active) return;
      active = false;
      void invoke("unsubscribe_plugin_events", { pluginInstanceId: instance.pluginInstanceId, subscriptionId }).catch(() => {});
    };
    subscriptions.add(cancel);
    if (disposed) cancel();
    return cancel;
  };
  return {
    apiVersion: FRONTEND_API_VERSION,
    capabilities: Object.freeze({ interactions: true, events: true, streams: false }),
    dispose: () => {
      disposed = true;
      cancelSecretInput(scope);
      for (const cancel of subscriptions) cancel();
    },
    pluginId: instance.pluginId,
    pluginInstanceId: instance.pluginInstanceId,
    sessionId: instance.sessionId,
    components: { blocks: mountBlockList },
    invoke: <T = unknown>(operation: string, args: unknown, options?: { waitTimeoutMs?: number }) =>
      whileMounted(() => hostAPI.invokeOperation<T>(
        instance.pluginInstanceId,
        operation,
        args,
        options?.waitTimeoutMs,
      )),
    getOperation: (operationId) =>
      hostAPI.getOperation(instance.pluginInstanceId, operationId),
    recentOperations: (limit) => hostAPI.recentOperations(instance.pluginInstanceId, limit),
    cancelOperation: (operationId) =>
      whileMounted(() => hostAPI.cancelOperation(instance.pluginInstanceId, operationId)),
    approveProposal: <T = unknown>(proposalId: string, args: unknown) =>
      whileMounted(() => hostAPI.approveProposal<T>(instance.pluginInstanceId, proposalId, args)),
    rejectProposal: (proposalId) =>
      whileMounted(() => hostAPI.rejectProposal(instance.pluginInstanceId, proposalId)),
    subscribeEvents,
    subscribe: <T = unknown>(event: string, handler: (event: T) => void | Promise<void>) => {
      let disposed = false;
      let cancel: (() => void) | undefined;
      void subscribeEvents((message) => { if (!disposed && message.name === event) return handler(message.payload as T); })
        .then((unsubscribe) => { if (disposed) unsubscribe(); else cancel = unsubscribe; })
        .catch(() => { if (!disposed) console.error("Plugin event subscription failed"); });
      return () => { disposed = true; cancel?.(); };
    },
    subscribeStream: async <T = Uint8Array>(
      _streamId: string,
      _options: StreamOptions,
      _handler: (event: StreamDelivery<T>) => void,
    ): Promise<StreamSubscription> => {
      throw unsupported("stream subscriptions");
    },
    requestText: (request) => disposed ? Promise.resolve({ outcome: "unavailable" }) : promptForText(scope, request),
    requestSecret: async (
      request: InteractionOptions & { purpose: string },
    ): Promise<InteractionOutcome<SecretHandle>> => {
      return disposed ? { outcome: "unavailable" } : promptForSecret(instance.pluginInstanceId, request, hostAPI.provideSecret, scope);
    },
    requestConfirmation: (request) => disposed ? Promise.resolve({ outcome: "unavailable" }) : promptForConfirmation(scope, request),
    requestSelection: (request) => disposed ? Promise.resolve({ outcome: "unavailable" }) : promptForSelection(scope, request),
  };
}
