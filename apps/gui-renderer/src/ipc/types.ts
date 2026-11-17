import type {
  CommandEnvelope,
  CommandReceipt,
  ErrorCode,
  EventCausation,
  EventCursor,
  EventEnvelope,
  HelloResponse,
  IpcError,
  OperationKind,
  OperationName,
  ProtocolVersion,
  QueryEnvelope,
  QueryResponse,
  ReasonCode,
  ReceiptState,
  RetryClass,
  Sensitivity,
} from "../generated";
import type {
  GuiCommandInput,
  GuiErrorPayload,
  GuiMarkdownInput,
  GuiQueryInput,
  GuiStreamNotification,
  GuiSubscribeInput,
  MarkdownBlockNode,
  MarkdownDocumentAst,
  MarkdownInlineNode,
} from "../generated-gui";

export type {
  CommandEnvelope,
  CommandReceipt,
  ErrorCode,
  EventCausation,
  EventCursor,
  EventEnvelope,
  GuiCommandInput,
  GuiErrorPayload,
  GuiMarkdownInput,
  GuiQueryInput,
  GuiStreamNotification,
  GuiSubscribeInput,
  HelloResponse,
  IpcError,
  MarkdownBlockNode,
  MarkdownDocumentAst,
  MarkdownInlineNode,
  OperationKind,
  OperationName,
  ProtocolVersion,
  QueryEnvelope,
  QueryResponse,
  ReasonCode,
  ReceiptState,
  RetryClass,
  Sensitivity,
};

export type ThemeMode = "light" | "dark";

export type SpaceId =
  | "overview"
  | "agents"
  | "conversations"
  | "workspaces"
  | "workflows"
  | "skills"
  | "knowledge"
  | "activity"
  | "reports"
  | "settings";

export type LogicalAgentState =
  "active" | "dormant" | "blocked" | "queued" | "waiting_tool" | "error" | "retired";

export type InvocationState = "running" | "waiting" | "idle" | "none";

export type AgentNode = {
  id: string;
  parentId: string | null;
  level: "N+2" | "N+1" | "N";
  name: string;
  role: string;
  model: string;
  logicalState: LogicalAgentState;
  invocationState: InvocationState;
  depth: number;
  hasChildren: boolean;
  blockedReason: string | null;
};

export type PoolStatus = {
  activeSlots: number;
  configuredLimit: number;
  hardMaxLimit: number;
  controlReserve: number;
  queuedCount: number;
};

export type MissionCostSummary = {
  estimatedUsd: string;
  committedUsd: string;
  confirmedUsd: string;
  hasUnknownUsage: boolean;
};

export type MissionSummary = {
  missionId: string;
  title: string;
  objective: string;
  ownerLabel: string;
  ownerKind: "user" | "agent";
  terminationCondition: string;
  mode: "supervised" | "replay_forbidden";
  statusLabel: string;
  nextDecision: string;
  staleDocumentNote: string;
  blockedIntegrationNote: string;
  pool: PoolStatus;
  cost: MissionCostSummary;
};

export type TimelineEventItem = {
  envelope: EventEnvelope;
  severity: "info" | "attention" | "danger";
  summary: string;
};

export type EventPage = {
  items: TimelineEventItem[];
  offset: number;
  limit: number;
  totalCount: number;
  laggingCount: number;
  cursor: EventCursor;
};

export type DiffDocument = {
  title: string;
  path: string;
  baseRevision: string;
  targetRevision: string;
  totalLines: number;
  originalText: string;
  modifiedText: string;
};

export type TerminalProjection = {
  processRunUuid: string;
  worktree: string;
  leaseId: string | null;
  fence: number;
  attached: boolean;
  initialLines: string[];
};

export type ConnectionState = "connected" | "disconnected" | "resync_required";

export type StreamSubscription = {
  cursor: EventCursor;
  close: () => void;
};

export interface TransportAdapter {
  executeQuery(operation: OperationName, request: GuiQueryInput): Promise<QueryResponse>;
  submitCommand(operation: OperationName, request: GuiCommandInput): Promise<CommandReceipt>;
  subscribeEvents(
    request: GuiSubscribeInput,
    onNotification: (notification: GuiStreamNotification) => void,
  ): Promise<StreamSubscription>;
  renderMarkdownAst(markdown: string): Promise<MarkdownDocumentAst>;
  listExposedOperations(): Promise<OperationName[]>;
}
