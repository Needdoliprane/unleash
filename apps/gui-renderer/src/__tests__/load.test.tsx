import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../App";
import { buildAgentTree, buildEventPage, buildTenThousandLineDiff } from "../data/m0Fixtures";

function percentile(sorted: readonly number[], ratio: number): number {
  if (sorted.length === 0) {
    return 0;
  }
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * ratio));
  return sorted[idx] ?? 0;
}

describe("AC-F014-011 — Charge GUI (1 000 agents, 100 000 événements, diff 10 000 lignes)", () => {
  it("rend et filtre la charge M0 en p95 < 100 ms sans matérialiser toutes les lignes", () => {
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

    // Résoudre les sélecteurs DOM avant les boucles chronométrées d'interaction utilisateur
    const agentFilter = screen.getByTestId("agent-filter-input");
    const timelineFilter = screen.getByTestId("timeline-filter-input");
    const prevPageBtn = screen.getByRole("button", { name: "Page précédente" });
    const jumpEndBtn = screen.getByRole("button", { name: "Fin (100 000)" });

    const agentSamples: number[] = [];
    for (let i = 0; i < 20; i += 1) {
      const value = i % 2 === 0 ? "Leader-01" : "Worker-00";
      const t0 = performance.now();
      fireEvent.change(agentFilter, { target: { value } });
      agentSamples.push(performance.now() - t0);
    }
    expect(screen.getAllByText(/Worker-00/).length).toBeGreaterThan(0);
    fireEvent.change(agentFilter, { target: { value: "" } });

    const timelineSamples: number[] = [];
    for (let i = 0; i < 20; i += 1) {
      const value = i % 2 === 0 ? "IntegrationBlocked" : "";
      const t0 = performance.now();
      fireEvent.change(timelineFilter, { target: { value } });
      timelineSamples.push(performance.now() - t0);
    }
    expect(screen.getByText(/Cache borné :/)).toBeInTheDocument();

    const pageJumpSamples: number[] = [];
    for (let i = 0; i < 20; i += 1) {
      const btn = i % 2 === 0 ? jumpEndBtn : prevPageBtn;
      const t0 = performance.now();
      fireEvent.click(btn);
      pageJumpSamples.push(performance.now() - t0);
    }
    expect(screen.getByText(/Fenêtre : 99/)).toBeInTheDocument();

    agentSamples.sort((a, b) => a - b);
    timelineSamples.sort((a, b) => a - b);
    pageJumpSamples.sort((a, b) => a - b);

    const agentP50 = percentile(agentSamples, 0.5);
    const agentP95 = percentile(agentSamples, 0.95);
    const timelineP50 = percentile(timelineSamples, 0.5);
    const timelineP95 = percentile(timelineSamples, 0.95);
    const pageJumpP50 = percentile(pageJumpSamples, 0.5);
    const pageJumpP95 = percentile(pageJumpSamples, 0.95);

    // Mesures consignées dans la sortie de test pour qualification M0
    console.info(
      `[GUI-LOAD] initialRenderMs=${initialRenderMs.toFixed(2)} agentFilter(p50=${agentP50.toFixed(2)}ms p95=${agentP95.toFixed(2)}ms) timelineFilter(p50=${timelineP50.toFixed(2)}ms p95=${timelineP95.toFixed(2)}ms) pageJump(p50=${pageJumpP50.toFixed(2)}ms p95=${pageJumpP95.toFixed(2)}ms) domAgentRows=${String(renderedAgentRows.length)}`,
    );

    expect(agentP95).toBeLessThan(100);
    expect(timelineP95).toBeLessThan(100);
    expect(pageJumpP95).toBeLessThan(100);
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
