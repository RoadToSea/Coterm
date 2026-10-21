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
} from "./plugin-api";
import { Channel, invoke } from "@tauri-apps/api/core";
import { promptForSecret } from "./secret-input";
import { mountBlockList } from "./common-components";

export interface PluginSummary {
  id: string;
  name: string;
  version: string;
  hasFrontend: boolean;
  running: boolean;
}

export interface InstanceInfo {
  pluginId: string;
  pluginInstanceId: string;
  sessionId: string;
  label: string;
  status: "starting" | "ready" | "stopping" | "stopped" | "failed";
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
  pendingInteractions(): Promise<BackendSecretRequest[]>;
  respondSecret(interactionId: string, value: string | null): Promise<void>;
  mcpConfiguration(): Promise<McpClientConfiguration>;
  listPlugins(): Promise<PluginSummary[]>;
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
}

/** 仅管理挂载容器；unmount 不隐式销毁后端实例。模块查找不暴露任意路径。 */
export interface PluginFrontendLoader {
  load(pluginId: string): Promise<PluginFrontendModule | null>;
  mount(container: HTMLElement, pluginInstanceId: string): Promise<PluginFrontend>;
}

/** Tauri-backed desktop API. Package paths stay inside the app data plugins directory. */
export const hostAPI: HostAPI = {
  pendingInteractions: () => invoke<BackendSecretRequest[]>("pending_interactions"),
  respondSecret: (interactionId, value) => invoke<void>("respond_secret", { interactionId, value }),
  mcpConfiguration: () => invoke<McpClientConfiguration>("mcp_connection_info"),
  frontendSource: (pluginId) => invoke<string | null>("plugin_frontend_source", { pluginId }),
  provideSecret: (pluginInstanceId, purpose, value) => invoke<SecretHandle>("provide_secret", { pluginInstanceId, purpose, value }),
  listPlugins: () => invoke<PluginSummary[]>("list_plugins"),
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
export function createPluginAPI(instance: InstanceInfo): PluginAPI {
  const unsupported = (feature: string): Error => new Error(`${feature} is not connected yet`);
  const subscribeEvents = async (handler: (event: PluginInstanceEvent) => void | Promise<void>): Promise<() => void> => {
    let active = true;
    const channel = new Channel<PluginInstanceEvent>();
    const ready = invoke<string>("subscribe_plugin_events", { pluginInstanceId: instance.pluginInstanceId, onEvent: channel });
    channel.onmessage = (event) => {
      void (async () => {
        const subscriptionId = await ready;
        if (!active) return;
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
    return () => {
      if (!active) return;
      active = false;
      void invoke("unsubscribe_plugin_events", { pluginInstanceId: instance.pluginInstanceId, subscriptionId }).catch(() => {});
    };
  };
  return {
    pluginId: instance.pluginId,
    pluginInstanceId: instance.pluginInstanceId,
    sessionId: instance.sessionId,
    components: { blocks: mountBlockList },
    invoke: <T = unknown>(operation: string, args: unknown, options?: { waitTimeoutMs?: number }) =>
      hostAPI.invokeOperation<T>(
        instance.pluginInstanceId,
        operation,
        args,
        options?.waitTimeoutMs,
      ),
    getOperation: (operationId) =>
      hostAPI.getOperation(instance.pluginInstanceId, operationId),
    recentOperations: (limit) => hostAPI.recentOperations(instance.pluginInstanceId, limit),
    cancelOperation: (operationId) =>
      hostAPI.cancelOperation(instance.pluginInstanceId, operationId),
    approveProposal: <T = unknown>(proposalId: string, args: unknown) =>
      hostAPI.approveProposal<T>(instance.pluginInstanceId, proposalId, args),
    rejectProposal: (proposalId) =>
      hostAPI.rejectProposal(instance.pluginInstanceId, proposalId),
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
    requestText: async (_request: InteractionOptions): Promise<InteractionOutcome<string>> => {
      throw unsupported("user interactions");
    },
    requestSecret: async (
      request: InteractionOptions & { purpose: string },
    ): Promise<InteractionOutcome<SecretHandle>> => {
      return promptForSecret(instance.pluginInstanceId, request, hostAPI.provideSecret);
    },
    requestConfirmation: async (request: InteractionOptions): Promise<InteractionOutcome<boolean>> =>
      ({ outcome: "submitted", value: window.confirm(`${request.title}\n\n${request.prompt}`) }),
    requestSelection: async (
      _request: InteractionOptions & { options: SelectionOption[] },
    ): Promise<InteractionOutcome<string>> => {
      throw unsupported("selection interactions");
    },
  };
}
