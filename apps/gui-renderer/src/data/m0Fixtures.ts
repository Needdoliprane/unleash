import type {
  AgentNode,
  DiffDocument,
  EventPage,
  InvocationState,
  LogicalAgentState,
  MarkdownDocumentAst,
  MissionSummary,
  TerminalProjection,
  TimelineEventItem,
} from "../ipc/types";

export const M0_MISSION_SUMMARY: MissionSummary = {
  missionId: "mission-m0-vertical-slice",
  title: "Qualification M0 — Réconciliation multi-équipes & intégration",
  objective:
    "Qualifier la supervision bornée sur trois équipes, vérifier une intégration bloquée et inspecter un audit long sans déclencher d'appel modèle implicite.",
  ownerLabel: "Équipe Plateforme (user:local)",
  ownerKind: "user",
  terminationCondition:
    "Tous les lots intégrés avec preuves vérifiées et aucune issue OutcomeUnknown ouverte.",
  mode: "supervised",
  statusLabel: "En cours — 1 intégration bloquée, 1 document périmé",
  nextDecision: "Inspecter l'issue incertaine sur integration-payments puis réconcilier",
  staleDocumentNote: "Document périmé : ADR-09 Worktrees (index stale depuis 2h)",
  blockedIntegrationNote: "Intégration bloquée : target checkout sur dev (blocked_target_checkout)",
  pool: {
    activeSlots: 3,
    configuredLimit: 5,
    hardMaxLimit: 50,
    controlReserve: 1,
    queuedCount: 14,
  },
  cost: {
    estimatedUsd: "1.40 USD",
    committedUsd: "0.82 USD",
    confirmedUsd: "0.61 USD",
    hasUnknownUsage: true,
  },
};

const AGENT_STATES: readonly LogicalAgentState[] = [
  "active",
  "dormant",
  "dormant",
  "blocked",
  "queued",
  "waiting_tool",
  "dormant",
  "error",
];

/**
 * Génère de façon déterministe 1 000 agents logiques répartis en N+2 (3 équipes), N+1 (20 leaders) et N (workers).
 * Distingue explicitement état logique et invocation active pour éviter la confusion avec 1 000 appels LLM.
 */
export function buildAgentTree(totalAgents = 1000): AgentNode[] {
  const nodes: AgentNode[] = [];
  const teams = [
    { id: "team-core", name: "N+2 Core Platform", role: "director" },
    { id: "team-connectors", name: "N+2 Connecteurs & Docs", role: "director" },
    { id: "team-qa", name: "N+2 Audit & Contrats", role: "director" },
  ];

  for (const team of teams) {
    if (nodes.length >= totalAgents) {
      break;
    }
    nodes.push({
      id: team.id,
      parentId: null,
      level: "N+2",
      name: team.name,
      role: team.role,
      model: "claude-sonnet-4-5",
      logicalState: team.id === "team-connectors" ? "blocked" : "dormant",
      invocationState: "none",
      depth: 0,
      hasChildren: true,
      blockedReason:
        team.id === "team-connectors"
          ? "Intégration bloquée : cible checkout active (blocked_target_checkout)"
          : null,
    });
  }

  const leaderCount = Math.min(20, Math.max(0, totalAgents - nodes.length));
  for (let i = 0; i < leaderCount; i += 1) {
    const team = teams[i % teams.length];
    const parentId = team ? team.id : null;
    const isRunning = i < 2;
    const logicalState: LogicalAgentState = isRunning ? "active" : i === 2 ? "blocked" : "dormant";
    const invocationState: InvocationState = isRunning ? "running" : "idle";
    nodes.push({
      id: `leader-${String(i + 1).padStart(3, "0")}`,
      parentId,
      level: "N+1",
      name: `Leader-${String(i + 1).padStart(2, "0")} (${team?.name ?? "Equipe"})`,
      role: "team-leader",
      model: i % 2 === 0 ? "claude-haiku-5-5" : "ollama:qwen3-coder",
      logicalState,
      invocationState,
      depth: 1,
      hasChildren: true,
      blockedReason: i === 2 ? "En attente de preuve de test avant décision" : null,
    });
  }

  let workerIndex = 1;
  while (nodes.length < totalAgents) {
    const leaderIdx = (workerIndex - 1) % Math.max(1, leaderCount);
    const parentId = leaderCount > 0 ? `leader-${String(leaderIdx + 1).padStart(3, "0")}` : null;
    const state = AGENT_STATES[workerIndex % AGENT_STATES.length] ?? "dormant";
    const activeSlot = workerIndex === 1;
    const logicalState: LogicalAgentState = activeSlot ? "active" : state;
    const invocationState: InvocationState = activeSlot
      ? "running"
      : logicalState === "waiting_tool"
        ? "waiting"
        : "none";

    nodes.push({
      id: `worker-${String(workerIndex).padStart(4, "0")}`,
      parentId,
      level: "N",
      name: `Worker-${String(workerIndex).padStart(4, "0")} [lot-${String((workerIndex % 12) + 1)}]`,
      role: workerIndex % 3 === 0 ? "verifier" : "implementer",
      model: workerIndex % 4 === 0 ? "gpt-6-luna" : "claude-haiku-5-5",
      logicalState,
      invocationState,
      depth: 2,
      hasChildren: false,
      blockedReason: logicalState === "blocked" ? "Dépendance worktree non intégrée" : null,
    });
    workerIndex += 1;
  }

  return nodes;
}

const EVENT_TYPES = [
  "InvocationAdmitted",
  "ActionDispatched",
  "ObservationRecorded",
  "CheckpointReached",
  "IntegrationBlocked",
  "DocumentStaleDetected",
] as const;

/**
 * Retourne une page bornée (ex: 200 entrées) d'un journal de 100 000 événements,
 * sans jamais matérialiser les 100 000 enveloppes en mémoire simultanément.
 */
export function buildEventPage(
  offset: number,
  limit: number,
  filterText: string,
  totalCount = 100_000,
): EventPage {
  const safeLimit = Math.min(200, Math.max(1, limit));
  const safeOffset = Math.max(0, Math.min(offset, totalCount));
  const needle = filterText.trim().toLowerCase();

  const items: TimelineEventItem[] = [];
  let index = safeOffset;
  let scanned = 0;
  const maxScan = safeLimit * 10;

  while (items.length < safeLimit && index < totalCount && scanned < maxScan) {
    const seq = index + 1;
    const eventType = EVENT_TYPES[index % EVENT_TYPES.length] ?? "InvocationAdmitted";
    const severity: "info" | "attention" | "danger" =
      eventType === "IntegrationBlocked"
        ? "danger"
        : eventType === "DocumentStaleDetected"
          ? "attention"
          : "info";
    const summary = `#${String(seq)} ${eventType} — lot-${String((index % 12) + 1)} (agent worker-${String((index % 977) + 1).padStart(4, "0")})`;

    if (!needle || summary.toLowerCase().includes(needle)) {
      items.push({
        envelope: {
          event_id: `00000000-0000-4000-8000-${String(seq).padStart(12, "0")}`,
          stream_id: "00000000-0000-4000-8000-000000000001",
          sequence: seq,
          event_type: eventType,
          schema_version: 1,
          occurred_at: "2026-11-17T09:00:00Z",
          recorded_at: "2026-11-17T09:00:01Z",
          project_id: "00000000-0000-4000-8000-000000000100",
          aggregate_id: `00000000-0000-4000-8000-${String((index % 1000) + 1).padStart(12, "0")}`,
          aggregate_revision: (index % 15) + 1,
          actor_ref: "uid:501",
          correlation_id: `00000000-0000-4000-9000-${String(seq).padStart(12, "0")}`,
          causation: {
            kind: "command",
            id: `rcpt-${String(seq)}`,
          },
          command_id: `cmd-${String(seq)}`,
          payload_hash: `sha256:${String(seq).padStart(64, "0")}`,
          payload: {
            lot: `lot-${String((index % 12) + 1)}`,
            slot_budget: "3/5",
            detail: summary,
          },
          sensitivity: "project",
        },
        severity,
        summary,
      });
    }
    index += 1;
    scanned += 1;
  }

  return {
    items,
    offset: safeOffset,
    limit: safeLimit,
    totalCount,
    laggingCount: 0,
    cursor: {
      stream_id: "00000000-0000-4000-8000-000000000001",
      sequence: totalCount,
    },
  };
}

export function buildTenThousandLineDiff(lineCount = 10_000): DiffDocument {
  const originalLines: string[] = [];
  const modifiedLines: string[] = [];
  for (let i = 1; i <= lineCount; i += 1) {
    const lineId = String(i);
    originalLines.push(
      `line ${lineId.padStart(5, "0")}: let state_${lineId} = Transition::Pending;`,
    );
    if (i % 50 === 0) {
      modifiedLines.push(
        `line ${lineId.padStart(5, "0")}: let state_${lineId} = Transition::Verified { evidence: "EV-${lineId}" };`,
      );
    } else {
      modifiedLines.push(
        `line ${lineId.padStart(5, "0")}: let state_${lineId} = Transition::Pending;`,
      );
    }
  }
  return {
    title: "Diff intégration — lot core-scheduler",
    path: "crates/unleash-harness/src/reducer.rs",
    baseRevision: "rev-412",
    targetRevision: "rev-413",
    totalLines: lineCount,
    originalText: originalLines.join("\n"),
    modifiedText: modifiedLines.join("\n"),
  };
}

export const M0_SAMPLE_MARKDOWN_REPORT = `# Rapport d'audit M0 — Mission Multi-Équipes

Statut : **en cours** — trois équipes suivies, une intégration bloquée et un document périmé.

## Faits observés

- Équipe Core Platform : 3 slots utilisés sur 5 configurés (plafond dur 50).
- Intégration bloquée : \`blocked_target_checkout\` sur la branche cible \`dev\`.
- Document périmé : \`ADR-09 Worktrees\` nécessite une réobservation explicite.
- Coût : estimé \`1.40 USD\`, engagé \`0.82 USD\`, confirmé \`0.61 USD\` (usage fournisseur partiellement inconnu).

> Aucun effet \`OutcomeUnknown\` n'est converti en succès par les logs.

[Consulter la spécification locale](https://v2.tauri.app/security/csp/)
`;

export function buildInitialReportAst(): MarkdownDocumentAst {
  return {
    truncated: false,
    total_nodes: 8,
    blocked_raw_html_count: 0,
    blocked_url_count: 0,
    blocks: [
      {
        kind: "heading",
        level: 1,
        children: [{ kind: "text", text: "Rapport d'audit M0 — Mission Multi-Équipes" }],
      },
      {
        kind: "paragraph",
        children: [
          { kind: "text", text: "Statut : " },
          { kind: "strong", children: [{ kind: "text", text: "en cours" }] },
          {
            kind: "text",
            text: " — trois équipes suivies, une intégration bloquée et un document périmé.",
          },
        ],
      },
      {
        kind: "list",
        ordered: false,
        start: 1,
        items: [
          [
            {
              kind: "paragraph",
              children: [
                {
                  kind: "text",
                  text: "Équipe Core Platform : 3 slots utilisés sur 5 configurés (plafond dur 50).",
                },
              ],
            },
          ],
          [
            {
              kind: "paragraph",
              children: [
                {
                  kind: "text",
                  text: "Intégration bloquée : blocked_target_checkout sur la branche cible dev.",
                },
              ],
            },
          ],
          [
            {
              kind: "paragraph",
              children: [
                {
                  kind: "text",
                  text: "Coût : estimé 1.40 USD, engagé 0.82 USD, confirmé 0.61 USD (usage inconnu).",
                },
              ],
            },
          ],
        ],
      },
      {
        kind: "blockquote",
        children: [
          {
            kind: "paragraph",
            children: [
              {
                kind: "text",
                text: "Aucun effet OutcomeUnknown n'est converti en succès par les logs.",
              },
            ],
          },
        ],
      },
    ],
  };
}

export const M0_TERMINAL_INITIAL: TerminalProjection = {
  processRunUuid: "proc-run-9b4a2c10",
  worktree: ".claude/worktrees/g0001-m0-t07",
  leaseId: "lease-001",
  fence: 7,
  attached: false,
  initialLines: [
    "[unleash-pty] process_run_uuid=proc-run-9b4a2c10 lease=lease-001 fence=7",
    "[unleash-pty] OSC 8 links=disabled OSC 52 clipboard=disabled",
    "$ cargo test -p unleash-protocol --quiet",
    "test result: ok. 31 passed; 0 failed",
  ],
};
