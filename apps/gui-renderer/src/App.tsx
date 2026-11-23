import { useMemo, useState } from "react";
import { Button } from "react-aria-components";
import { AgentTreePanel } from "./components/AgentTreePanel";
import { DiffAndReportPanel } from "./components/DiffAndReportPanel";
import { InspectorPanel } from "./components/InspectorPanel";
import { ProviderProfilesPanel } from "./components/ProviderProfilesPanel";
import { TerminalPanel } from "./components/TerminalPanel";
import { TimelinePanel } from "./components/TimelinePanel";
import { WorkflowRunPanel } from "./components/WorkflowRunPanel";
import {
    buildAgentTree,
    buildInitialReportAst,
    buildTenThousandLineDiff,
    M0_MISSION_SUMMARY,
    M0_SAMPLE_MARKDOWN_REPORT,
    M0_TERMINAL_INITIAL,
} from "./data/m0Fixtures";
import { createTauriTransportAdapter } from "./ipc/tauriBridge";
import type {
    AgentNode,
    ConnectionState,
    MarkdownDocumentAst,
    SpaceId,
    ThemeMode,
    TimelineEventItem,
    TransportAdapter,
} from "./ipc/types";

const SPACES: ReadonlyArray<{ id: SpaceId; label: string; groupCount: string }> = [
    { id: "overview", label: "Overview", groupCount: "6 PAR" },
    { id: "agents", label: "Agents", groupCount: "5 PAR" },
    { id: "conversations", label: "Conversations", groupCount: "5 PAR" },
    { id: "workspaces", label: "Workspaces", groupCount: "5 PAR" },
    { id: "workflows", label: "Workflows", groupCount: "9 PAR" },
    { id: "skills", label: "Skills Studio", groupCount: "5 PAR" },
    { id: "knowledge", label: "Knowledge", groupCount: "5 PAR" },
    { id: "activity", label: "Activity", groupCount: "6 PAR" },
    { id: "reports", label: "Reports", groupCount: "6 PAR" },
    { id: "settings", label: "Settings", groupCount: "6 PAR" },
];

export type AppProps = {
    transport?: TransportAdapter;
    initialTheme?: ThemeMode;
    agentCount?: number;
    eventTotalCount?: number;
    diffLineCount?: number;
    initialReportAst?: MarkdownDocumentAst;
};

export function App({
    transport: providedTransport,
    initialTheme = "light",
    agentCount = 1000,
    eventTotalCount = 100_000,
    diffLineCount = 10_000,
    initialReportAst,
}: AppProps) {
    const transport = useMemo(
        () => providedTransport ?? createTauriTransportAdapter(),
        [providedTransport],
    );
    const [theme, setTheme] = useState<ThemeMode>(initialTheme);
    const [activeSpace, setActiveSpace] = useState<SpaceId>("overview");
    const isSettings = activeSpace === "settings";
    const isWorkflows = activeSpace === "workflows";
    const showsMissionFixture = !isSettings && !isWorkflows;
    const [connectionState] = useState<ConnectionState>("connected");
    const [reportAst, setReportAst] = useState<MarkdownDocumentAst>(
        () => initialReportAst ?? buildInitialReportAst(),
    );
    const [liveStatus, setLiveStatus] = useState("Tranche verticale M0 prête");

    const agents = useMemo(() => buildAgentTree(agentCount), [agentCount]);
    const diff = useMemo(() => buildTenThousandLineDiff(diffLineCount), [diffLineCount]);

    const [selectedAgent, setSelectedAgent] = useState<AgentNode | null>(() => agents[0] ?? null);
    const [selectedEvent, setSelectedEvent] = useState<TimelineEventItem | null>(null);

    const handleRefreshAstFromHost = async (): Promise<void> => {
        const parsed = await transport.renderMarkdownAst(M0_SAMPLE_MARKDOWN_REPORT);
        setReportAst(parsed);
        setLiveStatus(`AST Markdown recalculé côté hôte (${String(parsed.total_nodes)} nœuds)`);
    };

    return (
        <div className="app-shell" data-theme={theme}>
            <header className="top-header">
                <div className="brand-group">
                    <h1 className="brand-title">OVPT Unleash</h1>
                    {showsMissionFixture ? (
                        <>
                            <span className="badge badge-success">
                                IPC unleash.ipc.v2 · {connectionState}
                            </span>
                            <span className="badge">Mode : {M0_MISSION_SUMMARY.mode}</span>
                        </>
                    ) : null}
                </div>

                <div className="header-meta">
                    {showsMissionFixture ? (
                        <>
                            <span className="mono" role="status" aria-live="polite">
                                {liveStatus}
                            </span>
                            <Button
                                className="control-button"
                                onPress={() => {
                                    void handleRefreshAstFromHost();
                                }}
                            >
                                Rafraîchir AST Rust
                            </Button>
                        </>
                    ) : null}
                    <Button
                        className="control-button"
                        data-testid="theme-toggle"
                        onPress={() => {
                            setTheme((prev) => (prev === "light" ? "dark" : "light"));
                        }}
                    >
                        Thème : {theme === "light" ? "Clair" : "Sombre"}
                    </Button>
                </div>
            </header>

            <div className="workspace-grid" data-space={activeSpace}>
                <nav className="sidebar-nav" aria-label="Espaces principaux OVPT Unleash">
                    <div className="nav-list" role="tablist" aria-orientation="vertical">
                        {SPACES.map((space) => {
                            const isSelected = space.id === activeSpace;
                            return (
                                <button
                                    key={space.id}
                                    type="button"
                                    role="tab"
                                    aria-selected={isSelected}
                                    data-selected={isSelected}
                                    className="nav-button"
                                    onClick={() => {
                                        setActiveSpace(space.id);
                                        setLiveStatus(`Espace actif : ${space.label}`);
                                    }}
                                >
                                    <span>{space.label}</span>
                                    {showsMissionFixture ? (
                                        <span className="mono">{space.groupCount}</span>
                                    ) : null}
                                </button>
                            );
                        })}
                    </div>
                </nav>

                {isSettings ? (
                    <main className="main-column" aria-label="Paramètres des modèles">
                        <ProviderProfilesPanel transport={transport} />
                    </main>
                ) : isWorkflows ? (
                    <main className="main-column" aria-label="Workflows">
                        <WorkflowRunPanel transport={transport} />
                    </main>
                ) : (
                    <main className="main-column" aria-label="Zone de travail Mission et Overview">
                        <section className="mission-header-card" aria-labelledby="mission-heading">
                            <h2 id="mission-heading" className="mission-title">
                                {M0_MISSION_SUMMARY.title}
                            </h2>
                            <p className="mission-objective">{M0_MISSION_SUMMARY.objective}</p>
                            <div className="status-strip">
                                <span className="badge">
                                    Responsable : {M0_MISSION_SUMMARY.ownerLabel}
                                </span>
                                <span className="badge badge-danger">
                                    {M0_MISSION_SUMMARY.blockedIntegrationNote}
                                </span>
                                <span className="badge badge-attention">
                                    {M0_MISSION_SUMMARY.staleDocumentNote}
                                </span>
                                <span className="badge badge-unknown">
                                    Coût : est. {M0_MISSION_SUMMARY.cost.estimatedUsd} / eng.{" "}
                                    {M0_MISSION_SUMMARY.cost.committedUsd} / conf.{" "}
                                    {M0_MISSION_SUMMARY.cost.confirmedUsd}
                                    {M0_MISSION_SUMMARY.cost.hasUnknownUsage
                                        ? " (+ usage inconnu)"
                                        : ""}
                                </span>
                            </div>
                        </section>

                        {activeSpace !== "overview" ? (
                            <section
                                className="panel-card"
                                aria-label={`Vue synthétique ${activeSpace}`}
                            >
                                <h2 className="panel-title">Espace {activeSpace}</h2>
                                <p style={{ marginBottom: 0 }}>
                                    Les opérations du catalogue sont raccordées au même backend
                                    `unleash-client`. La tranche verticale M0 reste inspectable
                                    ci-dessous sur le même flux mission.
                                </p>
                            </section>
                        ) : null}

                        <AgentTreePanel
                            agents={agents}
                            pool={M0_MISSION_SUMMARY.pool}
                            selectedAgentId={selectedAgent?.id ?? null}
                            onSelectAgent={(agent) => {
                                setSelectedAgent(agent);
                                setLiveStatus(`Agent sélectionné : ${agent.name}`);
                            }}
                        />

                        <TimelinePanel
                            totalEvents={eventTotalCount}
                            laggingCount={0}
                            selectedEventId={selectedEvent?.envelope.event_id ?? null}
                            onSelectEvent={(item) => {
                                setSelectedEvent(item);
                                setLiveStatus(
                                    `Événement sélectionné : #${String(item.envelope.sequence)}`,
                                );
                            }}
                        />

                        <DiffAndReportPanel diff={diff} reportAst={reportAst} />

                        <TerminalPanel terminal={M0_TERMINAL_INITIAL} transport={transport} />
                    </main>
                )}

                {showsMissionFixture ? (
                    <InspectorPanel selectedAgent={selectedAgent} selectedEvent={selectedEvent} />
                ) : null}
            </div>

            {showsMissionFixture ? (
                <footer className="footer-bar">
                    <span>
                        OVPT Unleash — Tranche verticale M0 (Tauri 2 + React 19 + TS strict)
                    </span>
                    <span className="mono">
                        58 groupes PAR · 0 règle métier dans le renderer · WCAG 2.2 AA cible
                    </span>
                </footer>
            ) : null}
        </div>
    );
}
