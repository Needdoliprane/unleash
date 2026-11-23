import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { App } from "../App";
import type {
    CommandReceipt,
    GuiCommandInput,
    GuiErrorPayload,
    GuiQueryInput,
    QueryResponse,
} from "../ipc/types";

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
const prepared = {
    ...run,
    plan: {
        workflow_version_id: run.workflow_version_id,
        definition_hash: definition,
        scopes: { implementation: { scope_id: projectId, scope_revision: 1 } },
    },
};

function response(result: unknown): QueryResponse {
    return {
        request_id: "00000000-0000-4000-8000-000000000001",
        watermark: { stream_id: "00000000-0000-4000-8000-000000000002", sequence: 12 },
        result,
    };
}

async function consultRun(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole("tab", { name: /^Workflows/u }));
    await screen.findByRole("option", { name: /^Atlas · /u });
    await user.selectOptions(screen.getByRole("combobox", { name: "Projet" }), projectId);
    await user.type(screen.getByRole("textbox", { name: "Identifiant du run" }), runId);
    await user.keyboard("{Enter}");
}

afterEach(() => {
    vi.unstubAllGlobals();
});

it("T-F005-001 (renderer) — lit la version engagée et l’état du run demandé", async () => {
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
    expect(detail.queryByRole("button", { name: "Démarrer le run" })).not.toBeInTheDocument();
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
        "op_list_projects",
        "op_get_run",
    ]);
});

it("T-F005-001 (renderer) — ignore la réponse tardive après le changement de projet", async () => {
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

it("T-F005-004 (renderer) — démarre le run exact et relit son état engagé", async () => {
    let started = false;
    const invoke = vi.fn(
        (command: string, { input }: { input: GuiQueryInput | GuiCommandInput }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([{ project_id: projectId, name: "Atlas" }]));
                case "op_get_run":
                    expect(input).toEqual({ project_id: projectId, payload: { run_id: runId } });
                    return Promise.resolve(
                        response(
                            started ? { ...prepared, state: "running", revision: 2 } : prepared,
                        ),
                    );
                case "op_start_run": {
                    if (!("command_id" in input))
                        throw new Error("Commande de démarrage attendue.");
                    const { command_id, ...target } = input;
                    expect(command_id).toMatch(
                        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u,
                    );
                    expect(target).toEqual({
                        project_id: projectId,
                        aggregate_id: runId,
                        expected_revision: 1,
                        payload: {},
                    });
                    started = true;
                    return Promise.resolve({
                        receipt_id: "00000000-0000-4000-8000-000000000030",
                        command_id: input.command_id,
                        accepted_at: "2026-11-23T12:00:00Z",
                        aggregate_id: runId,
                        new_revision: 2,
                        operation_id: null,
                        state: "completed",
                        event_cursor: {
                            stream_id: "00000000-0000-4000-8000-000000000002",
                            sequence: 13,
                        },
                        result_ref: `workflow-run:${runId}`,
                    } satisfies CommandReceipt);
                }
                default:
                    throw new Error(`Opération inattendue : ${command}`);
            }
        },
    );
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    const user = userEvent.setup();
    render(<App agentCount={4} eventTotalCount={12} diffLineCount={20} />);

    await consultRun(user);
    await screen.findByText("Préparé");
    expect(invoke.mock.calls.some(([command]) => command === "op_start_run")).toBe(false);

    await user.tab();
    await user.tab();
    expect(screen.getByRole("button", { name: "Démarrer le run" })).toHaveFocus();
    await user.keyboard("{Enter}");

    const detail = within(await screen.findByRole("region", { name: "Détail du run" }));
    expect(await detail.findByText("En cours")).toBeVisible();
    expect(detail.getByText("workflow.feature@1")).toBeVisible();
    expect(detail.queryByRole("button", { name: "Démarrer le run" })).not.toBeInTheDocument();
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
        "op_list_projects",
        "op_get_run",
        "op_start_run",
        "op_get_run",
    ]);
});

it("T-F005-009 (renderer) — relit un démarrage incertain sans renvoyer de commande", async () => {
    let committed = false;
    const invoke = vi.fn(
        (command: string, { input }: { input: GuiQueryInput | GuiCommandInput }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([{ project_id: projectId, name: "Atlas" }]));
                case "op_get_run":
                    expect(input).toEqual({ project_id: projectId, payload: { run_id: runId } });
                    return Promise.resolve(
                        response(
                            committed ? { ...prepared, state: "running", revision: 2 } : prepared,
                        ),
                    );
                case "op_start_run":
                    return Promise.reject(
                        Object.assign(new Error("Reçu perdu"), {
                            code: "OutcomeUnknown",
                            retry_class: "after_reconciliation",
                        }),
                    );
                default:
                    throw new Error(`Opération inattendue : ${command}`);
            }
        },
    );
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    const user = userEvent.setup();
    render(<App agentCount={4} eventTotalCount={12} diffLineCount={20} />);

    await consultRun(user);
    await user.click(await screen.findByRole("button", { name: "Démarrer le run" }));

    expect(await screen.findByText("Démarrage non confirmé.")).toBeVisible();
    expect(screen.queryByText("En cours")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Démarrer le run" })).not.toBeInTheDocument();
    committed = true;
    await user.click(screen.getByRole("button", { name: "Relire le résultat du démarrage" }));

    expect(await screen.findByText("En cours")).toBeVisible();
    expect(screen.queryByText("Démarrage non confirmé.")).not.toBeInTheDocument();
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
        "op_list_projects",
        "op_get_run",
        "op_start_run",
        "op_get_run",
        "op_get_run",
    ]);
});

it("T-F005-001 (renderer) — relit la révision après un conflit de démarrage", async () => {
    let revision = 1;
    const invoke = vi.fn(
        (command: string, { input }: { input: GuiQueryInput | GuiCommandInput }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([{ project_id: projectId, name: "Atlas" }]));
                case "op_get_run":
                    expect(input).toEqual({ project_id: projectId, payload: { run_id: runId } });
                    return Promise.resolve(response({ ...prepared, revision }));
                case "op_start_run":
                    revision = 2;
                    return Promise.reject(
                        Object.assign(new Error("Révision périmée"), {
                            code: "RevisionConflict",
                            retry_class: "after_refresh",
                        }),
                    );
                default:
                    throw new Error(`Opération inattendue : ${command}`);
            }
        },
    );
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    const user = userEvent.setup();
    render(<App agentCount={4} eventTotalCount={12} diffLineCount={20} />);

    await consultRun(user);
    await user.click(await screen.findByRole("button", { name: "Démarrer le run" }));

    const detail = within(await screen.findByRole("region", { name: "Détail du run" }));
    expect(await detail.findByText("2")).toBeVisible();
    expect(detail.getByText("Préparé")).toBeVisible();
    expect(detail.getByRole("alert")).toHaveTextContent("Les données du run ont changé.");
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
        "op_list_projects",
        "op_get_run",
        "op_start_run",
        "op_get_run",
    ]);
});

it("T-F005-009 (renderer) — conserve l’identité d’une reprise autorisée et affiche le refus", async () => {
    const starts: GuiCommandInput[] = [];
    const invoke = vi.fn(
        (command: string, { input }: { input: GuiQueryInput | GuiCommandInput }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([{ project_id: projectId, name: "Atlas" }]));
                case "op_get_run":
                    return Promise.resolve(response(prepared));
                case "op_start_run":
                    if (!("command_id" in input))
                        throw new Error("Commande de démarrage attendue.");
                    starts.push(input);
                    return Promise.reject(
                        Object.assign(
                            new Error("Démarrage indisponible."),
                            starts.length === 1
                                ? { code: "ResourceBusy", retry_class: "after_backoff" }
                                : { code: "CapabilityDenied", retry_class: "never" },
                        ),
                    );
                default:
                    throw new Error(`Opération inattendue : ${command}`);
            }
        },
    );
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    const user = userEvent.setup();
    render(<App agentCount={4} eventTotalCount={12} diffLineCount={20} />);

    await consultRun(user);
    await user.click(await screen.findByRole("button", { name: "Démarrer le run" }));
    const retry = await screen.findByRole("button", { name: "Reprendre le démarrage" });
    expect(starts).toHaveLength(1);
    await user.click(retry);

    expect(await screen.findByRole("alert")).toHaveTextContent("Démarrage refusé.");
    expect(screen.getByText("Préparé")).toBeVisible();
    expect(screen.queryByText("En cours")).not.toBeInTheDocument();
    expect(starts).toHaveLength(2);
    expect(starts[1]).toEqual(starts[0]);
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
        "op_list_projects",
        "op_get_run",
        "op_start_run",
        "op_start_run",
    ]);
});

it("T-F005-004 (renderer) — affiche le budget épuisé comme un refus de démarrage", async () => {
    const failure: GuiErrorPayload = {
        code: "BudgetExceeded",
        reason_code: "BUDGET_EXHAUSTED",
        retry_class: "after_human_decision",
        message: "Budget réservé au run insuffisant pour l’activation initiale.",
    };
    const invoke = vi.fn((command: string) => {
        switch (command) {
            case "op_list_projects":
                return Promise.resolve(response([{ project_id: projectId, name: "Atlas" }]));
            case "op_get_run":
                return Promise.resolve(response(prepared));
            case "op_start_run":
                return Promise.reject(Object.assign(new Error(failure.message), failure));
            default:
                throw new Error(`Opération inattendue : ${command}`);
        }
    });
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    const user = userEvent.setup();
    render(<App agentCount={4} eventTotalCount={12} diffLineCount={20} />);

    await consultRun(user);
    await user.click(await screen.findByRole("button", { name: "Démarrer le run" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Budget d’activation épuisé.");
    expect(screen.getByText("Préparé")).toBeVisible();
    expect(screen.queryByText("Démarrage non confirmé.")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Démarrer le run" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Actualiser le run" }));
    expect(await screen.findByText("Préparé")).toBeVisible();
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
        "op_list_projects",
        "op_get_run",
        "op_start_run",
        "op_get_run",
    ]);
});
