/** 插件前端契约；桌面 SDK 适配器位于 host-api.ts，接口尚未冻结。 */

export type OperationSource = "human" | "agent" | "system";
export type OperationEffect = "read" | "draft" | "commit";
export type OperationStatus =
  | "accepted" | "awaiting_approval" | "dispatched" | "running"
  | "completed" | "failed" | "rejected" | "cancelled" | "unknown";

export interface OperationDescriptor {
  name: string;
  scope: "plugin" | "instance";
  effect: OperationEffect;
  inputSchema: unknown;
  outputSchema: unknown | null;
  sensitiveInput?: boolean;
  transient?: boolean;
}

export interface RuntimeError {
  category: "invalid_request" | "not_found" | "unsupported" | "conflict"
    | "interaction" | "transport" | "domain" | "internal";
  code: string;
  message: string;
  details: unknown | null;
}

export interface OperationReceipt<T = unknown> {
  operationId: string;
  proposalId: string | null;
  status: OperationStatus;
  waitExpired: boolean;
  output: T | null;
  error: RuntimeError | null;
}

export interface OperationRecord<T = unknown> extends OperationReceipt<T> {
  pluginId: string;
  pluginInstanceId: string | null;
  source: OperationSource;
  effect: OperationEffect;
  operation: string;
  input: unknown;
  createdAt: string;
  updatedAt: string;
  transient: boolean;
}

export interface InvokeOptions {
  /** 不提供=默认等待；0=立即返回。超时不会取消操作。 */
  waitTimeoutMs?: number;
}

export interface InteractionOptions {
  title: string;
  prompt: string;
  timeoutMs?: number;
  allowCancel?: boolean;
}

export interface SelectionOption { value: string; label: string }

export interface SecretHandle {
  readonly id: string;
  readonly pluginInstanceId: string;
  readonly purpose: string;
  readonly expiresAt: string;
}

export type InteractionOutcome<T> =
  | { outcome: "submitted"; value: T }
  | { outcome: "rejected" | "cancelled" | "timed_out" | "unavailable" };

export interface FlowWindow { chunks: number; bytes: number }
export type OverflowPolicy = "pause" | "drop_with_gap" | "close";

/** sequence 是十进制字符串；u64 不应直接转换成 JS number。 */
export type StreamDelivery<T = unknown> =
  | { type: "data"; streamId: string; sequence: string; timestampMs: number; payload: T }
  | { type: "gap"; streamId: string; fromSequence: string; toSequence: string;
      reason: string; recoverable: boolean }
  | { type: "closed"; streamId: string; reason: string | null };

export interface StreamOptions {
  window: FlowWindow;
  overflow: OverflowPolicy;
}

/** 取消投递不会销毁实例、物理连接或 Broker 的协议订阅。 */
export interface StreamSubscription {
  readonly id: string;
  grant(credit: FlowWindow): Promise<void>;
  cancel(): Promise<void>;
}

/** 可选组件的领域数据仍由使用该组件的插件定义，组件不访问基座内部。 */
export type CommonComponent = (container: HTMLElement, props: unknown) => PluginFrontend;

/** 绑定当前实例；不允许从调用参数改目标、伪造来源或选择低风险 effect。 */
export interface PluginAPI {
  readonly pluginId: string;
  readonly pluginInstanceId: string;
  readonly sessionId: string;
  readonly components?: Readonly<Record<string, CommonComponent>>;

  invoke<T = unknown>(operation: string, args: unknown, options?: InvokeOptions): Promise<OperationReceipt<T>>;
  getOperation(operationId: string): Promise<OperationRecord>;
  recentOperations(limit?: number): Promise<OperationRecord[]>;
  cancelOperation(operationId: string): Promise<OperationReceipt>;

  /** args 是用户确认的当前草稿快照，基座绑定 proposalId 与原操作。 */
  approveProposal(proposalId: string, args: unknown): Promise<OperationReceipt>;
  rejectProposal(proposalId: string): Promise<void>;

  subscribe<T = unknown>(event: string, handler: (event: T) => void | Promise<void>): () => void;
  /** Resolves after subscription is installed. Handler completion acknowledges delivery. */
  subscribeEvents(handler: (event: PluginInstanceEvent) => void | Promise<void>): Promise<() => void>;
  subscribeStream<T = Uint8Array>(streamId: string, options: StreamOptions,
    handler: (event: StreamDelivery<T>) => void): Promise<StreamSubscription>;

  requestText(request: InteractionOptions): Promise<InteractionOutcome<string>>;
  requestSecret(request: InteractionOptions & { purpose: string }): Promise<InteractionOutcome<SecretHandle>>;
  requestConfirmation(request: InteractionOptions): Promise<InteractionOutcome<boolean>>;
  requestSelection(request: InteractionOptions & { options: SelectionOption[] }): Promise<InteractionOutcome<string>>;
}

export interface PluginInstanceEvent { name: string; payload: unknown }

export interface PluginFrontend { unmount(): void }

/** 前端模块导出 mount，容器内部 DOM 与状态全部由插件负责。 */
export type PluginMount = (container: HTMLElement, api: PluginAPI) => PluginFrontend;
export interface PluginConfigurationAPI {
  /** Completes plugin-owned configuration; the host starts/creates the instance. */
  submit(config: unknown): Promise<void>;
}

export interface PluginFrontendModule {
  mount: PluginMount;
  /** Optional plugin-owned configuration screen, before an instance exists. */
  configure?(container: HTMLElement, api: PluginConfigurationAPI): PluginFrontend;
}
