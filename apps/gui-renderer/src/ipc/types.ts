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

export type {
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

export type MarkdownInlineNode =
  | { kind: "text"; text: string }
  | { kind: "strong"; children: MarkdownInlineNode[] }
  | { kind: "emphasis"; children: MarkdownInlineNode[] }
  | { kind: "strikethrough"; children: MarkdownInlineNode[] }
  | { kind: "code"; text: string }
  | { kind: "link"; href: string; title: string | null; children: MarkdownInlineNode[] }
  | { kind: "soft_break" }
  | { kind: "hard_break" };

export type MarkdownBlockNode =
  | { kind: "heading"; level: 1 | 2 | 3 | 4 | 5 | 6; children: MarkdownInlineNode[] }
  | { kind: "paragraph"; children: MarkdownInlineNode[] }
  | { kind: "list"; ordered: boolean; start: number; items: MarkdownBlockNode[][] }
  | { kind: "blockquote"; children: MarkdownBlockNode[] }
  | { kind: "code_block"; language: string | null; code: string }
  | { kind: "thematic_break" }
  | { kind: "table"; headers: MarkdownInlineNode[][]; rows: MarkdownInlineNode[][][] };

export type MarkdownDocumentAst = {
  truncated: boolean;
  total_nodes: number;
  blocked_raw_html_count: number;
  blocked_url_count: number;
  blocks: MarkdownBlockNode[];
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

export type GuiQueryRequest = {
  project_id: string | null;
  payload: Record<string, unknown>;
};

export type GuiCommandRequest = {
  project_id: string | null;
  aggregate_id: string;
  expected_revision: number;
  command_id: string;
  payload: Record<string, unknown>;
};

export interface TransportAdapter {
  executeQuery(operation: OperationName, request: GuiQueryRequest): Promise<QueryResponse>;
  submitCommand(operation: OperationName, request: GuiCommandRequest): Promise<CommandReceipt>;
  renderMarkdownAst(markdown: string): Promise<MarkdownDocumentAst>;
  listExposedOperations(): Promise<OperationName[]>;
}
