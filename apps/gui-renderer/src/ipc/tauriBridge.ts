import type {
  CommandReceipt,
  EventCursor,
  GuiCommandInput,
  GuiQueryInput,
  GuiStreamNotification,
  GuiSubscribeInput,
  MarkdownBlockNode,
  MarkdownDocumentAst,
  MarkdownInlineNode,
  OperationName,
  QueryResponse,
  StreamSubscription,
  TransportAdapter,
} from "./types";

/**
 * Opérations sans groupe PAR dans `.claude/catalog/operations.json` :
 * jamais exposées au renderer GUI (`ADR-03` amendement R7).
 */
export const FORBIDDEN_GUI_OPERATIONS: ReadonlySet<OperationName> = new Set<OperationName>([
  "Hello",
  "RecordClaim",
  "RecordObservation",
  "RecordJudgment",
]);

export function operationToTauriCommand(operation: OperationName): string {
  const snake = operation
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase();
  return `op_${snake}`;
}

export function isExposedGuiOperation(operation: OperationName): boolean {
  return !FORBIDDEN_GUI_OPERATIONS.has(operation);
}

export function assertExposedGuiOperation(operation: OperationName): void {
  if (!isExposedGuiOperation(operation)) {
    throw new Error(
      `Opération interdite côté renderer GUI (${operation}) : réservée au canal daemon/worker.`,
    );
  }
}

type TauriChannelMessage<T> = {
  message?: T;
  index?: number;
  end?: boolean;
};

type TauriInternals = {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  transformCallback?(
    callback: (response: TauriChannelMessage<GuiStreamNotification>) => void,
    once?: boolean,
  ): number;
  unregisterCallback?(id: number): void;
};

function getTauriInternals(): TauriInternals | null {
  if (typeof window === "undefined") {
    return null;
  }
  const candidate = (window as unknown as { __TAURI_INTERNALS__?: TauriInternals })
    .__TAURI_INTERNALS__;
  if (candidate && typeof candidate.invoke === "function") {
    return candidate;
  }
  return null;
}

export function sanitizeHttpUrl(rawUrl: string): string | null {
  const trimmed = rawUrl.trim();
  if (!trimmed) {
    return null;
  }
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol === "https:" || parsed.protocol === "http:") {
      return parsed.toString();
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Fallback local purement présentationnel utilisé uniquement hors hôte Tauri (Vitest / aperçu navigateur).
 * En runtime Tauri, l'AST Markdown provient exclusivement du parseur Rust `pulldown-cmark`.
 */
export function buildSafeFallbackMarkdownAst(markdown: string): MarkdownDocumentAst {
  const lines = markdown.split(/\r?\n/);
  const blocks: MarkdownBlockNode[] = [];
  let blockedRawHtml = 0;
  let blockedUrls = 0;

  const parseInlines = (text: string): MarkdownInlineNode[] => {
    const nodes: MarkdownInlineNode[] = [];
    const linkPattern = /\[([^\]]+)\]\(([^)]+)\)/g;
    let lastIndex = 0;
    let match: RegExpExecArray | null = linkPattern.exec(text);
    while (match !== null) {
      if (match.index > lastIndex) {
        nodes.push({ kind: "text", text: text.slice(lastIndex, match.index) });
      }
      const label = match[1] ?? "";
      const rawHref = match[2] ?? "";
      const safeHref = sanitizeHttpUrl(rawHref);
      if (safeHref === null) {
        blockedUrls += 1;
        nodes.push({ kind: "text", text: `${label} [URL bloquée]` });
      } else {
        nodes.push({
          kind: "link",
          href: safeHref,
          title: null,
          children: [{ kind: "text", text: label }],
        });
      }
      lastIndex = match.index + match[0].length;
      match = linkPattern.exec(text);
    }
    if (lastIndex < text.length) {
      nodes.push({ kind: "text", text: text.slice(lastIndex) });
    }
    return nodes.length > 0 ? nodes : [{ kind: "text", text: "" }];
  };

  let inCode = false;
  let codeLang: string | null = null;
  let codeLines: string[] = [];

  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("```")) {
      if (inCode) {
        blocks.push({ kind: "code_block", language: codeLang, code: codeLines.join("\n") });
        inCode = false;
        codeLang = null;
        codeLines = [];
      } else {
        inCode = true;
        const lang = trimmed.slice(3).trim();
        codeLang = lang.length > 0 ? lang : null;
      }
      continue;
    }
    if (inCode) {
      codeLines.push(line);
      continue;
    }
    if (!trimmed) {
      continue;
    }
    if (/^<\/?[a-zA-Z!]/.test(trimmed)) {
      blockedRawHtml += 1;
      blocks.push({
        kind: "paragraph",
        children: [{ kind: "text", text: `[contenu HTML/SVG neutralisé] ${trimmed}` }],
      });
      continue;
    }
    const headingMatch = /^(#{1,6})\s+(.*)$/.exec(trimmed);
    if (headingMatch) {
      const level = Math.min(6, Math.max(1, (headingMatch[1] ?? "#").length));
      blocks.push({
        kind: "heading",
        level,
        children: parseInlines(headingMatch[2] ?? ""),
      });
      continue;
    }
    if (trimmed.startsWith("- ")) {
      blocks.push({
        kind: "list",
        ordered: false,
        start: 1,
        items: [[{ kind: "paragraph", children: parseInlines(trimmed.slice(2)) }]],
      });
      continue;
    }
    if (trimmed.startsWith("> ")) {
      blocks.push({
        kind: "blockquote",
        children: [{ kind: "paragraph", children: parseInlines(trimmed.slice(2)) }],
      });
      continue;
    }
    blocks.push({ kind: "paragraph", children: parseInlines(trimmed) });
  }

  return {
    truncated: false,
    total_nodes: blocks.length,
    blocked_raw_html_count: blockedRawHtml,
    blocked_url_count: blockedUrls,
    blocks,
  };
}

export function createTauriTransportAdapter(): TransportAdapter {
  return {
    async executeQuery(operation: OperationName, request: GuiQueryInput): Promise<QueryResponse> {
      assertExposedGuiOperation(operation);
      const tauri = getTauriInternals();
      if (!tauri) {
        return {
          request_id: `req-${operation}`,
          watermark: { stream_id: "00000000-0000-0000-0000-000000000001", sequence: 100000 },
          result: { operation, fixture: true, payload: request.payload },
        };
      }
      return tauri.invoke<QueryResponse>(operationToTauriCommand(operation), { input: request });
    },

    async submitCommand(
      operation: OperationName,
      request: GuiCommandInput,
    ): Promise<CommandReceipt> {
      assertExposedGuiOperation(operation);
      const tauri = getTauriInternals();
      if (!tauri) {
        return {
          receipt_id: `rcpt-${request.command_id}`,
          command_id: request.command_id,
          accepted_at: "2026-11-17T09:00:00Z",
          aggregate_id: request.aggregate_id,
          new_revision: request.expected_revision + 1,
          operation_id: null,
          state: "accepted",
          event_cursor: {
            stream_id: "00000000-0000-0000-0000-000000000001",
            sequence: 100001,
          },
        };
      }
      return tauri.invoke<CommandReceipt>(operationToTauriCommand(operation), { input: request });
    },

    async subscribeEvents(
      request: GuiSubscribeInput,
      onNotification: (notification: GuiStreamNotification) => void,
    ): Promise<StreamSubscription> {
      assertExposedGuiOperation("SubscribeEvents");
      const tauri = getTauriInternals();
      if (!tauri || typeof tauri.transformCallback !== "function") {
        const cursor: EventCursor = {
          stream_id: "00000000-0000-0000-0000-000000000001",
          sequence: 100000,
        };
        onNotification({
          kind: "snapshot_loaded",
          watermark: cursor,
          snapshot: { fixture: true, project_id: request.project_id },
          resync_reason: null,
        });
        return {
          cursor,
          close: () => {},
        };
      }

      let closed = false;
      let nextExpectedIndex = 0;
      const pending = new Map<number, GuiStreamNotification>();

      const callbackId = tauri.transformCallback((envelope) => {
        if (closed) {
          return;
        }
        if (envelope.end) {
          closed = true;
          tauri.unregisterCallback?.(callbackId);
          return;
        }
        if (envelope.message === undefined) {
          return;
        }
        const idx = envelope.index ?? nextExpectedIndex;
        pending.set(idx, envelope.message);
        while (pending.has(nextExpectedIndex)) {
          const msg = pending.get(nextExpectedIndex);
          pending.delete(nextExpectedIndex);
          nextExpectedIndex += 1;
          if (msg) {
            onNotification(msg);
          }
        }
      }, false);

      const channel = {
        toJSON: (): string => `__CHANNEL__:${String(callbackId)}`,
      };

      try {
        const cursor = await tauri.invoke<EventCursor>("op_subscribe_events", {
          input: request,
          channel,
        });
        return {
          cursor,
          close: () => {
            if (!closed) {
              closed = true;
              tauri.unregisterCallback?.(callbackId);
            }
          },
        };
      } catch (err) {
        closed = true;
        tauri.unregisterCallback?.(callbackId);
        throw err;
      }
    },

    async renderMarkdownAst(markdown: string): Promise<MarkdownDocumentAst> {
      const tauri = getTauriInternals();
      if (!tauri) {
        return buildSafeFallbackMarkdownAst(markdown);
      }
      return tauri.invoke<MarkdownDocumentAst>("gui_render_markdown_ast", {
        input: { markdown },
      });
    },

    async listExposedOperations(): Promise<OperationName[]> {
      const tauri = getTauriInternals();
      if (!tauri) {
        return ["GetOverview", "ListAgents", "SearchEvents", "GetWorkspaceDiff", "AcquireTerminal"];
      }
      return tauri.invoke<OperationName[]>("gui_list_exposed_operations");
    },
  };
}
