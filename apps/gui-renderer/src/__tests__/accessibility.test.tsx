import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../App";

describe("AC-F014-010 — Accessibilité clavier, ARIA, IME et thèmes", () => {
  it("permet navigation clavier, sélection d'agent, bascule clair/sombre et composition IME sans soumission prématurée", () => {
    const { container } = render(<App agentCount={12} eventTotalCount={50} diffLineCount={40} />);

    const shell = container.querySelector(".app-shell");
    expect(shell).toHaveAttribute("data-theme", "light");

    // Bascule thème clair -> sombre
    const themeToggle = screen.getByTestId("theme-toggle");
    fireEvent.click(themeToggle);
    expect(shell).toHaveAttribute("data-theme", "dark");

    // Navigation clavier sur les 10 espaces
    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(10);
    const activityTab = screen.getByRole("tab", { name: /Activity/ });
    fireEvent.click(activityTab);
    expect(activityTab).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText("Espace activity")).toBeInTheDocument();

    // Composition IME japonaise sur le filtre agents : ne filtre pas avant compositionend
    const filterInput = screen.getByTestId("agent-filter-input");
    fireEvent.compositionStart(filterInput);
    fireEvent.change(filterInput, { target: { value: "にほ" } });
    // Les lignes d'agents restent présentes pendant la composition IME
    expect(screen.getByTestId("agent-row-team-core")).toBeInTheDocument();

    fireEvent.compositionEnd(filterInput, { currentTarget: { value: "Leader-01" } });
    fireEvent.change(filterInput, { target: { value: "Leader-01" } });
    expect(screen.getByText(/Leader-01/)).toBeInTheDocument();

    // Navigation flèche bas / Entrée dans l'arbre agents
    fireEvent.change(filterInput, { target: { value: "" } });
    const firstNode = screen.getByTestId("agent-row-team-core");
    firstNode.focus();
    fireEvent.keyDown(firstNode, { key: "ArrowDown" });
    expect(screen.getByTestId("inspector-agent").textContent).toContain("team-connectors");
  });
});
