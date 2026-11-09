/** 桌面壳的接口；不传给驱动前端，不包含通信协议或驱动内部页面模型。 */
import type {
  OperationDescriptor,
  OperationRecord,
  OperationReceipt,
  DriverAPI,
  DriverFrontend,
  DriverFrontendModule,
  InteractionOptions,
  InteractionOutcome,
  SelectionOption,
  SecretHandle,
  StreamDelivery,
  StreamOptions,
  StreamSubscription,
  DriverInstanceEvent,
} from "@coterm/driver-sdk";
import { Channel, invoke } from "@tauri-apps/api/core";
import { driverI18n } from "./i18n";
import { listen } from "@tauri-apps/api/event";
import { cancelSecretInput, promptForSecret, promptForText, promptForConfirmation, promptForSelection } from "./secret-input";
import { FRONTEND_API_VERSION } from "@coterm/driver-sdk";
import { mountBlockList } from "@coterm/driver-sdk/components/block-list";

/** 宿主待处理状态变化的全局事件；payload 只有版本号。 */
export const ATTENTION_CHANGED_EVENT = "host://attention-changed";

export interface AttentionApproval {
  operationId: string;
  proposalId: string;
  driverId: string;
  /** null 表示驱动级操作。 */
  driverInstanceId: string | null;
  operation: string;
  targetLabel: string;
  createdAt: string;
}

export interface AttentionInteraction {
  interactionId: string;
  driverInstanceId: string;
  operationId: string | null;
  /** 基座只路由 secret 交互；普通文本/确认由驱动前端自行处理。 */
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

export interface DriverSummary {
  id: string;
  name: string;
  version: string;
  hasFrontend: boolean;
  running: boolean;
}

/** 单个驱动包的可定位诊断；不含绝对路径、秘密或内部错误明文。 */
export interface DriverLoadIssue {
  directory: string;
  driverId: string | null;
  code: string;
  message: string;
}

/** 有效驱动与逐包诊断分开返回：单个坏包不影响其余驱动列出。 */
export interface DriverCatalog {
  drivers: DriverSummary[];
  issues: DriverLoadIssue[];
}

export interface InstanceInfo {
  driverId: string;
  driverInstanceId: string;
  sessionId: string;
  label: string;
  status: "starting" | "ready" | "stopping" | "stopped" | "failed";
}

export interface WorkspaceGroup { id: string; name: string }

export interface SavedWorkspace {
  entries: { configRef: string; driverId: string; label: string; instanceId: string | null; workspaceId: string; approvalRuleGroupId: string | null }[];
  groups: WorkspaceGroup[];
  approvalRuleGroups: ApprovalRuleGroup[];
  layout: { activeConfigRef: string | null; instancesCollapsed: boolean; activeWorkspaceId: string };
  historyPolicy: "session_only";
  persistence: "windows_dpapi" | "memory_only";
}

export interface BackendSecretRequest extends InteractionOptions {
  interactionId: string;
  driverInstanceId: string;
  operationId: string;
  kind: { kind: "secret"; purpose: string };
}

/**
 * Stdio 适配器状态。适配器是桌面可执行文件的 `mcp stdio` 子命令；
 * `clientConfig` 是不含秘密的 Stdio 客户端配置，只用于用户复制。
 */
export type McpStartupMode = "auto" | "manual";

export interface McpStdioStatus {
  enabled: boolean;
  /** 适配器入口，即桌面可执行文件；路径不可用时为 null。 */
  command: string | null;
  address: string | null;
  clientConfig: string | null;
  port: number;
  startupMode: McpStartupMode;
  configurationError: string | null;
}

export interface ApprovalPolicy {
  mode: "allow_matches" | "approve_matches";
  patterns: string[];
}

export interface ApprovalRuleGroup extends ApprovalPolicy { id: string; name: string }
export type ApprovalGroupDraft = Omit<ApprovalRuleGroup, "id"> & { id: string | null };
export interface ApprovalSnapshot {
  groups: ApprovalRuleGroup[];
  instances: { configRef: string; label: string; driverId: string; instanceId: string | null; ruleGroupId: string | null; bypass: boolean }[];
}

export interface HostAPI {
  runtimeStatus(): Promise<{ instances: InstanceInfo[]; runningDrivers: string[] }>;
  workspace(): Promise<SavedWorkspace>;
  saveLayout(activeInstanceId: string | null, instancesCollapsed: boolean, order: string[], activeWorkspaceId?: string): Promise<void>;
  createWorkspace(name: string): Promise<WorkspaceGroup>;
  deleteWorkspace(workspaceId: string): Promise<void>;
  moveWorkspaceEntry(configRef: string, workspaceId: string): Promise<void>;
  restoreInstance(configRef: string): Promise<InstanceInfo>;
  forgetInstance(configRef: string): Promise<void>;
  resetWorkspace(): Promise<void>;
  saveInstanceConfiguration(instanceId: string, config: unknown): Promise<void>;
  pendingInteractions(): Promise<BackendSecretRequest[]>;
  respondSecret(interactionId: string, value: string | null): Promise<void>;
  mcpStdioStatus(): Promise<McpStdioStatus>;
  setMcpStdioEnabled(enabled: boolean): Promise<McpStdioStatus>;
  setMcpStdioPort(port: number): Promise<McpStdioStatus>;
  setMcpStartupMode(startupMode: McpStartupMode): Promise<McpStdioStatus>;
  listDrivers(): Promise<DriverCatalog>;
  startDriver(driverId: string): Promise<void>;
  stopDriver(driverId: string): Promise<void>;
  listInstances(driverId?: string): Promise<InstanceInfo[]>;
  createInstance(driverId: string, config: unknown, workspaceId?: string, name?: string): Promise<InstanceInfo>;
  destroyInstance(driverInstanceId: string): Promise<void>;
  listOperations(driverId: string): Promise<OperationDescriptor[]>;
  invokeOperation<T = unknown>(
    driverInstanceId: string,
    operation: string,
    args: unknown,
    waitTimeoutMs?: number,
  ): Promise<OperationReceipt<T>>;
  invokeDriverOperation<T = unknown>(driverId: string, operation: string, args: unknown, waitTimeoutMs?: number): Promise<OperationReceipt<T>>;
  recentDriverOperations(driverId: string, limit?: number): Promise<OperationRecord[]>;
  approveDriverProposal<T = unknown>(driverId: string, proposalId: string, args: unknown): Promise<OperationReceipt<T>>;
  rejectDriverProposal(driverId: string, proposalId: string): Promise<void>;
  getOperation(driverInstanceId: string, operationId: string): Promise<OperationRecord>;
  recentOperations(driverInstanceId: string, limit?: number): Promise<OperationRecord[]>;
  approveProposal<T = unknown>(
    driverInstanceId: string,
    proposalId: string,
    args: unknown,
  ): Promise<OperationReceipt<T>>;
  rejectProposal(driverInstanceId: string, proposalId: string): Promise<void>;
  cancelOperation(driverInstanceId: string, operationId: string): Promise<OperationReceipt>;
  setBypass(driverInstanceId: string, enabled: boolean): Promise<void>;
  getBypass(driverInstanceId: string): Promise<boolean>;
  /** Host-only configuration: rejects invalid rules, dangling references and failed writes. */
  approvalSnapshot(): Promise<ApprovalSnapshot>;
  saveApprovalGroup(group: ApprovalGroupDraft): Promise<ApprovalRuleGroup>;
  assignApprovalGroup(configRef: string, groupId: string | null): Promise<void>;
  deleteApprovalGroup(groupId: string): Promise<void>;
  previewApprovalPolicy(policy: ApprovalPolicy, text: string): Promise<boolean>;
  frontendSource(driverId: string): Promise<string | null>;
  provideSecret(driverInstanceId: string, purpose: string, value: string): Promise<SecretHandle>;
  /** 一次取回全部实例级与驱动级待处理项，替代逐实例扫描历史。 */
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
export interface DriverFrontendLoader {
  load(driverId: string): Promise<DriverFrontendModule | null>;
  mount(container: HTMLElement, driverInstanceId: string): Promise<DriverFrontend>;
}

/** Tauri-backed desktop API. Package paths stay inside the app data drivers directory. */
export const hostAPI: HostAPI = {
  runtimeStatus: () => invoke("runtime_snapshot"),
  workspace: () => invoke<SavedWorkspace>("workspace_snapshot"),
  saveLayout: (activeInstanceId, instancesCollapsed, order, activeWorkspaceId) => invoke<void>("save_workspace_layout", { activeInstanceId, instancesCollapsed, order, activeWorkspaceId }),
  createWorkspace: (name) => invoke<WorkspaceGroup>("create_workspace_group", { name }),
  deleteWorkspace: (workspaceId) => invoke<void>("delete_workspace_group", { workspaceId }),
  moveWorkspaceEntry: (configRef, workspaceId) => invoke<void>("move_workspace_entry", { configRef, workspaceId }),
  saveInstanceConfiguration: (instanceId, config) => invoke<void>("save_instance_configuration", {driverInstanceId:instanceId, config}),
  restoreInstance: (configRef) => invoke<InstanceInfo>("restore_saved_instance", { configRef }),
  forgetInstance: (configRef) => invoke<void>("forget_saved_instance", { configRef }),
  resetWorkspace: () => invoke<void>("reset_saved_workspace"),
  hostAttention: () => invoke<HostAttention>("host_attention"),
  pendingInteractions: () => invoke<BackendSecretRequest[]>("pending_interactions"),
  respondSecret: (interactionId, value) => invoke<void>("respond_secret", { interactionId, value }),
  mcpStdioStatus: () => invoke<McpStdioStatus>("mcp_stdio_status"),
  setMcpStdioEnabled: (enabled) => invoke<McpStdioStatus>("set_mcp_stdio_enabled", { enabled }),
  setMcpStdioPort: (port) => invoke<McpStdioStatus>("set_mcp_stdio_port", { request: { port } }),
  setMcpStartupMode: (startupMode) => invoke<McpStdioStatus>("set_mcp_startup_mode", { request: { startupMode } }),
  frontendSource: (driverId) => invoke<string | null>("driver_frontend_source", { driverId }),
  provideSecret: (driverInstanceId, purpose, value) => invoke<SecretHandle>("provide_secret", { driverInstanceId, purpose, value }),
  listDrivers: () => invoke<DriverCatalog>("list_drivers"),
  startDriver: (driverId) => invoke<void>("start_driver", { driverId }),
  stopDriver: (driverId) => invoke<void>("stop_driver", { driverId }),
  listInstances: (driverId) => invoke<InstanceInfo[]>("list_instances", { driverId }),
  createInstance: (driverId, config, workspaceId, name) =>
    invoke<InstanceInfo>("create_instance", { driverId, config, workspaceId, ...(name ? { name } : {}) }),
  destroyInstance: (driverInstanceId) =>
    invoke<void>("destroy_instance", { driverInstanceId }),
  listOperations: (driverId) => invoke<OperationDescriptor[]>("list_operations", { driverId }),
  invokeOperation: <T = unknown>(
    driverInstanceId: string,
    operation: string,
    args: unknown,
    waitTimeoutMs?: number,
  ) =>
    invoke<OperationReceipt<T>>("invoke_operation", {
      driverInstanceId,
      operation,
      args,
      waitTimeoutMs,
    }),
  invokeDriverOperation: <T = unknown>(driverId: string, operation: string, args: unknown, waitTimeoutMs?: number) => invoke<OperationReceipt<T>>("invoke_driver_operation", {
    driverId, operation, args, waitTimeoutMs,
  }),
  recentDriverOperations: (driverId, limit) => invoke<OperationRecord[]>("recent_driver_operations", { driverId, limit }),
  approveDriverProposal: <T = unknown>(driverId: string, proposalId: string, args: unknown) => invoke<OperationReceipt<T>>("approve_driver_proposal", { driverId, proposalId, args }),
  rejectDriverProposal: (driverId, proposalId) => invoke<void>("reject_driver_proposal", { driverId, proposalId }),
  getOperation: (driverInstanceId, operationId) =>
    invoke<OperationRecord>("get_operation", { driverInstanceId, operationId }),
  recentOperations: (driverInstanceId, limit) =>
    invoke<OperationRecord[]>("recent_operations", { driverInstanceId, limit }),
  approveProposal: <T = unknown>(driverInstanceId: string, proposalId: string, args: unknown) =>
    invoke<OperationReceipt<T>>("approve_proposal", {
      driverInstanceId,
      proposalId,
      args,
    }),
  rejectProposal: (driverInstanceId, proposalId) =>
    invoke<void>("reject_proposal", { driverInstanceId, proposalId }),
  cancelOperation: (driverInstanceId, operationId) =>
    invoke<OperationReceipt>("cancel_operation", { driverInstanceId, operationId }),
  setBypass: (driverInstanceId, enabled) => invoke<void>("set_bypass", { driverInstanceId, enabled }),
  getBypass: (driverInstanceId) => invoke<boolean>("get_bypass", { driverInstanceId }),
  approvalSnapshot: () => invoke<ApprovalSnapshot>("approval_snapshot"),
  saveApprovalGroup: (group) => invoke<ApprovalRuleGroup>("save_approval_group", { group }),
  assignApprovalGroup: (configRef, groupId) => invoke<void>("assign_approval_group", { configRef, groupId }),
  deleteApprovalGroup: (groupId) => invoke<void>("delete_approval_group", { groupId }),
  previewApprovalPolicy: (policy, text) => invoke<boolean>("preview_approval_policy", { policy, text }),
};

/** Binds driver-facing calls to one instance; unsupported event features fail explicitly. */
export function createDriverAPI(instance: InstanceInfo, instanceConfigRef?: string): DriverAPI & { dispose(): void } {
  const unsupported = (feature: string): Error => new Error(`${feature} is not connected yet`);
  let disposed = false;
  const whileMounted = <T>(call: () => Promise<T>): Promise<T> => disposed
    ? Promise.reject(new Error("Driver frontend is disposed")) : call();
  const scope = `frontend:${instance.driverInstanceId}`;
  const subscriptions = new Set<() => void>();
  const subscribeEvents = async (handler: (event: DriverInstanceEvent) => void | Promise<void>): Promise<() => void> => {
    if (disposed) throw new Error("Driver frontend is disposed");
    let active = true;
    const channel = new Channel<DriverInstanceEvent>();
    const ready = invoke<string>("subscribe_driver_events", { driverInstanceId: instance.driverInstanceId, onEvent: channel });
    channel.onmessage = (event) => {
      void (async () => {
        const subscriptionId = await ready;
        if (!active || disposed) return;
        try { await handler(event); }
        catch {
          active = false;
          await invoke("unsubscribe_driver_events", { driverInstanceId: instance.driverInstanceId, subscriptionId });
          return;
        }
        if (active) await invoke("ack_driver_event", { driverInstanceId: instance.driverInstanceId, subscriptionId });
      })().catch(() => { active = false; });
    };
    const subscriptionId = await ready;
    const cancel = () => {
      subscriptions.delete(cancel);
      if (!active) return;
      active = false;
      void invoke("unsubscribe_driver_events", { driverInstanceId: instance.driverInstanceId, subscriptionId }).catch(() => {});
    };
    subscriptions.add(cancel);
    if (disposed) cancel();
    return cancel;
  };
  return {
    apiVersion: FRONTEND_API_VERSION,
    capabilities: Object.freeze({ interactions: true, events: true, streams: false }),
    i18n: {
      get locale() { return driverI18n.locale; },
      subscribe: (listener) => {
        if (disposed) return () => {};
        const stop = driverI18n.subscribe(listener);
        const cancel = () => { subscriptions.delete(cancel); stop(); };
        subscriptions.add(cancel);
        return cancel;
      },
    },
    dispose: () => {
      disposed = true;
      cancelSecretInput(scope);
      for (const cancel of subscriptions) cancel();
    },
    driverId: instance.driverId,
    driverInstanceId: instance.driverInstanceId,
    instanceConfigRef,
    sessionId: instance.sessionId,
    components: { blocks: mountBlockList },
    invoke: <T = unknown>(operation: string, args: unknown, options?: { waitTimeoutMs?: number }) =>
      whileMounted(() => hostAPI.invokeOperation<T>(
        instance.driverInstanceId,
        operation,
        args,
        options?.waitTimeoutMs,
      )),
    saveConfiguration: (config) => whileMounted(() => hostAPI.saveInstanceConfiguration(instance.driverInstanceId, config)),
    getOperation: (operationId) =>
      hostAPI.getOperation(instance.driverInstanceId, operationId),
    recentOperations: (limit) => hostAPI.recentOperations(instance.driverInstanceId, limit),
    cancelOperation: (operationId) =>
      whileMounted(() => hostAPI.cancelOperation(instance.driverInstanceId, operationId)),
    approveProposal: <T = unknown>(proposalId: string, args: unknown) =>
      whileMounted(() => hostAPI.approveProposal<T>(instance.driverInstanceId, proposalId, args)),
    rejectProposal: (proposalId) =>
      whileMounted(() => hostAPI.rejectProposal(instance.driverInstanceId, proposalId)),
    subscribeEvents,
    subscribe: <T = unknown>(event: string, handler: (event: T) => void | Promise<void>) => {
      let disposed = false;
      let cancel: (() => void) | undefined;
      void subscribeEvents((message) => { if (!disposed && message.name === event) return handler(message.payload as T); })
        .then((unsubscribe) => { if (disposed) unsubscribe(); else cancel = unsubscribe; })
        .catch(() => { if (!disposed) console.error("Driver event subscription failed"); });
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
      return disposed ? { outcome: "unavailable" } : promptForSecret(instance.driverInstanceId, request, hostAPI.provideSecret, scope);
    },
    requestConfirmation: (request) => disposed ? Promise.resolve({ outcome: "unavailable" }) : promptForConfirmation(scope, request),
    requestSelection: (request) => disposed ? Promise.resolve({ outcome: "unavailable" }) : promptForSelection(scope, request),
  };
}
