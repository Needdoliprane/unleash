import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../App";
import { buildAgentTree, buildEventPage, buildTenThousandLineDiff } from "../data/m0Fixtures";

describe("AC-F014-011 — Charge GUI (1 000 agents, 100 000 événements, diff 10 000 lignes)", () => {
  it("rend et filtre la charge M0 sans matérialiser toutes les lignes ni dépasser le budget de latence", () => {
    const startRender = performance.now();
    render(<App />);
    const initialRenderMs = performance.now() - startRender;

    expect(screen.getByText(/Arbre hiérarchique des agents \(1000 logiques\)/)).toBeInTheDocument();
    expect(screen.getByText(/100[\s\u202f]000 total, page de 200/)).toBeInTheDocument();
    expect(screen.getByText(/10[\s\u202f]000 lignes/)).toBeInTheDocument();

    // Virtualisation effective : seul un sous-ensemble borné des 1 000 agents est monté dans le DOM
    const renderedAgentRows = screen.getAllByRole("treeitem");
    expect(renderedAgentRows.length).toBeGreaterThan(0);
    expect(renderedAgentRows.length).toBeLessThan(80);

    // Filtre agents temps réel
    const agentFilter = screen.getByTestId("agent-filter-input");
    const startAgentFilter = performance.now();
    fireEvent.change(agentFilter, { target: { value: "Leader-01" } });
    const agentFilterMs = performance.now() - startAgentFilter;
    expect(screen.getByText(/Leader-01/)).toBeInTheDocument();

    // Pagination et filtre sur 100 000 événements
    const timelineFilter = screen.getByTestId("timeline-filter-input");
    const startTimelineFilter = performance.now();
    fireEvent.change(timelineFilter, { target: { value: "IntegrationBlocked" } });
    const timelineFilterMs = performance.now() - startTimelineFilter;
    expect(screen.getByText(/Cache borné :/)).toBeInTheDocument();

    // Saut en fin de journal (100 000)
    const startJump = performance.now();
    fireEvent.click(screen.getByRole("button", { name: "Fin (100 000)" }));
    const pageJumpMs = performance.now() - startJump;

    // Mesures consignées dans la sortie de test pour qualification M0
    console.info(
      `[GUI-LOAD] initialRenderMs=${initialRenderMs.toFixed(2)} agentFilterMs=${agentFilterMs.toFixed(2)} timelineFilterMs=${timelineFilterMs.toFixed(2)} pageJumpMs=${pageJumpMs.toFixed(2)} domAgentRows=${String(renderedAgentRows.length)}`,
    );

    expect(agentFilterMs).toBeLessThan(150);
    expect(timelineFilterMs).toBeLessThan(150);
    expect(pageJumpMs).toBeLessThan(150);
  });

  it("génère des projections bornées pour 1 000 agents, pages de 200/100 000 événements et diff 10 000 lignes", () => {
    const agents = buildAgentTree(1000);
    expect(agents).toHaveLength(1000);
    expect(agents.filter((a) => a.logicalState === "dormant").length).toBeGreaterThan(200);

    const page = buildEventPage(99_800, 200, "", 100_000);
    expect(page.items).toHaveLength(200);
    expect(page.totalCount).toBe(100_000);
    expect(page.items[199]?.envelope.sequence).toBe(100_000);

    const diff = buildTenThousandLineDiff(10_000);
    expect(diff.totalLines).toBe(10_000);
    expect(diff.originalText.split("\n")).toHaveLength(10_000);
    expect(diff.modifiedText.split("\n")).toHaveLength(10_000);
  });
});
