import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { App } from "../App";
import type { GuiQueryInput, QueryResponse } from "../ipc/types";

const projectId = "00000000-0000-4000-8000-000000000009";
const runId = "00000000-0000-4000-8000-000000000010";
const definition = `sha256:${"ab".repeat(32)}`;
const bundle = `sha256:${"cd".repeat(32)}`;

const run = {
    project_id: projectId,
    run_id: runId,
    mission_id: "00000000-0000-4000-8000-000000000120",
    workflow_version_id: "workflow.feature@1",
    definition_hash: definition,
    effective_bundle_hash: bundle,
    state: "prepared",
    revision: 1,
    purpose: "operational",
    plan: null,
};

function response(result: unknown): QueryResponse {
    return {
        request_id: "00000000-0000-4000-8000-000000000001",
        watermark: { stream_id: "00000000-0000-4000-8000-000000000002", sequence: 12 },
        result,
    };
}

afterEach(() => {
    vi.unstubAllGlobals();
});

it("lit la version engagée et l’état durable du run demandé dans Workflows", async () => {
    const invoke = vi.fn((command: string, { input }: { input: GuiQueryInput }) => {
        if (command === "op_list_projects") {
            return Promise.resolve(response([{ project_id: projectId, name: "Atlas" }]));
        }
        if (command === "op_get_run") {
            expect(input).toEqual({ project_id: projectId, payload: { run_id: runId } });
            return Promise.resolve(response(run));
        }
        throw new Error(`Opération inattendue : ${command}`);
    });
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    const user = userEvent.setup();
    render(<App agentCount={4} eventTotalCount={12} diffLineCount={20} />);

    await user.click(screen.getByRole("tab", { name: /^Workflows/u }));
    expect(invoke.mock.calls.some(([command]) => command === "op_get_run")).toBe(false);
    await screen.findByRole("option", { name: /^Atlas · /u });
    await user.selectOptions(screen.getByRole("combobox", { name: "Projet" }), projectId);
    await user.type(screen.getByRole("textbox", { name: "Identifiant du run" }), runId);
    await user.keyboard("{Enter}");

    const detail = within(await screen.findByRole("region", { name: "Détail du run" }));
    expect(detail.getByText(runId)).toBeVisible();
    expect(detail.getByText("Préparé")).toBeVisible();
    expect(detail.getByText("workflow.feature@1")).toBeVisible();
    expect(detail.getByText(definition)).toBeVisible();
    expect(detail.getByText(bundle)).toBeVisible();
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
        "op_list_projects",
        "op_get_run",
    ]);
});

it("ignore la réponse tardive d’un autre projet après la lecture du nouveau run", async () => {
    const otherProject = "00000000-0000-4000-8000-000000000020";
    const otherRun = "00000000-0000-4000-8000-000000000021";
    let resolveFirst: ((result: QueryResponse) => void) | undefined;
    const delayed = new Promise<QueryResponse>((resolve) => {
        resolveFirst = resolve;
    });
    const invoke = vi.fn((command: string, { input }: { input: GuiQueryInput }) => {
        if (command === "op_list_projects") {
            return Promise.resolve(
                response([
                    { project_id: projectId, name: "Atlas" },
                    { project_id: otherProject, name: "Nova" },
                ]),
            );
        }
        if (command !== "op_get_run") {
            throw new Error(`Opération inattendue : ${command}`);
        }
        return input.project_id === projectId
            ? delayed
            : Promise.resolve(
                  response({
                      ...run,
                      project_id: otherProject,
                      run_id: otherRun,
                      state: "running",
                      revision: 3,
                      workflow_version_id: "workflow.feature@2",
                  }),
              );
    });
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    const user = userEvent.setup();
    render(<App agentCount={4} eventTotalCount={12} diffLineCount={20} />);

    await user.click(screen.getByRole("tab", { name: /^Workflows/u }));
    const project = await screen.findByRole("combobox", { name: "Projet" });
    const runInput = screen.getByRole("textbox", { name: "Identifiant du run" });
    await user.selectOptions(project, projectId);
    await user.type(runInput, runId);
    await user.keyboard("{Enter}");
    expect(await screen.findByText("Lecture du run…")).toBeVisible();

    await user.selectOptions(project, otherProject);
    expect(runInput).toHaveValue("");
    await user.type(runInput, otherRun);
    await user.keyboard("{Enter}");
    const detail = within(await screen.findByRole("region", { name: "Détail du run" }));
    expect(detail.getByText("workflow.feature@2")).toBeVisible();

    await act(async () => {
        if (resolveFirst === undefined) throw new Error("Réponse différée indisponible");
        resolveFirst(response(run));
        await delayed;
    });
    expect(detail.getByText(otherRun)).toBeVisible();
    expect(detail.getByText("En cours")).toBeVisible();
    expect(detail.queryByText("workflow.feature@1")).not.toBeInTheDocument();
    expect(invoke.mock.calls.filter(([command]) => command === "op_get_run")).toEqual([
        ["op_get_run", { input: { project_id: projectId, payload: { run_id: runId } } }],
        ["op_get_run", { input: { project_id: otherProject, payload: { run_id: otherRun } } }],
    ]);
});
