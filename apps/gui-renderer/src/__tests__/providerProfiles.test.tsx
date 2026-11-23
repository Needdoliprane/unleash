import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import listResponse from "../../../../.claude/examples/ipc-provider-profile-list-response.json";
import { App } from "../App";
import type { CommandReceipt, GuiCommandInput, GuiQueryInput, QueryResponse } from "../ipc/types";

const [initialProfile] = listResponse.result.profiles;
if (initialProfile === undefined) {
    throw new Error("La fixture de liste doit contenir un profil.");
}
const project = {
    project_id: "00000000-0000-4000-8000-000000000009",
    name: "Atlas",
};

const configurationReceiptId = "00000000-0000-4000-8000-000000000020";
const validationResultRef = `sha256:${"ab".repeat(32)}`;
const freshUuid = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

type Invoke = (
    command: string,
    args: { input: GuiQueryInput | GuiCommandInput },
) => Promise<QueryResponse | CommandReceipt>;

function response(result: unknown): QueryResponse {
    return { ...listResponse, result };
}

function observation(probeId: string) {
    return {
        phase: "validate_config",
        status: "validated",
        observed_at_ms: 1795348800000,
        probe_id: probeId,
        result_ref: validationResultRef,
    };
}

function detailResponse(
    observations = [observation("00000000-0000-4000-8000-000000000010")],
): QueryResponse {
    return response({
        ...initialProfile,
        configuration_receipt_id: configurationReceiptId,
        observations,
    });
}

function submittedInput(input: GuiQueryInput | GuiCommandInput): GuiCommandInput {
    if (!("command_id" in input)) {
        throw new Error("Une commande IPC était attendue.");
    }
    return input;
}

function completedReceipt(input: GuiCommandInput): CommandReceipt {
    return {
        receipt_id: "00000000-0000-4000-8000-000000000021",
        command_id: input.command_id,
        aggregate_id: input.aggregate_id,
        accepted_at: "2026-11-22T12:00:00Z",
        new_revision: 1,
        operation_id: null,
        state: "completed",
        event_cursor: listResponse.watermark,
        result_ref: validationResultRef,
    };
}

function fixture(handler: Invoke) {
    const invoke = vi.fn(handler);
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    const user = userEvent.setup();
    render(<App agentCount={4} eventTotalCount={12} diffLineCount={20} />);
    return { invoke, user };
}

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("T-F012-018 — création d’un profil local", () => {
    it.each(["accepté", "perdu", "expiré"])(
        "retrouve le profil Ollama dans la portée projet avec un reçu %s",
        async (receiptStatus) => {
            let created: GuiCommandInput | null = null;
            const model = "exemple-local:8b";
            const { invoke, user } = fixture((operation, { input }) => {
                switch (operation) {
                    case "op_list_projects":
                        return Promise.resolve(response([project]));
                    case "op_list_provider_profiles":
                        return Promise.resolve(
                            response({
                                profiles:
                                    created && input.project_id === project.project_id
                                        ? [
                                              {
                                                  ...initialProfile,
                                                  model,
                                                  profile_id: created.aggregate_id,
                                                  profile_version: 1,
                                                  provider_kind: "ollama",
                                                  qualification: "required",
                                              },
                                          ]
                                        : [],
                                next_after_profile_id: null,
                            }),
                        );
                    case "op_set_provider_profile":
                        created = submittedInput(input);
                        if (receiptStatus === "perdu") {
                            return Promise.reject(
                                new Error("Réponse perdue après l’enregistrement."),
                            );
                        }
                        if (receiptStatus === "expiré") {
                            return Promise.reject(
                                Object.assign(new Error("La fenêtre d’idempotence est expirée."), {
                                    code: "ContractViolation",
                                    reason_code: "IDEMPOTENCY_WINDOW_EXPIRED",
                                    retry_class: "never",
                                }),
                            );
                        }
                        return Promise.resolve({
                            ...completedReceipt(created),
                            receipt_id: configurationReceiptId,
                            state: "accepted",
                            result_ref: null,
                        });
                    case "op_get_provider_profile":
                        if (created === null) {
                            throw new Error("Le profil doit être créé avant sa lecture.");
                        }
                        return Promise.resolve(
                            response({
                                ...initialProfile,
                                profile_id: created.aggregate_id,
                                profile_version: 1,
                                configuration_receipt_id: configurationReceiptId,
                                qualification: "required",
                                observations: [],
                            }),
                        );
                    default:
                        throw new Error(`Appel inattendu à ${operation}`);
                }
            });

            await user.click(screen.getByRole("tab", { name: /Settings/ }));
            await screen.findByRole("option", { name: /Atlas/ });
            await user.selectOptions(
                screen.getByRole("combobox", { name: "Portée" }),
                project.project_id,
            );
            await user.click(screen.getByRole("button", { name: "Ajouter un profil local" }));
            await user.type(screen.getByLabelText("Modèle"), model);
            await user.click(screen.getByRole("button", { name: "Enregistrer le profil" }));

            expect(await screen.findByText("Profil enregistré.")).toBeVisible();
            expect(
                await screen.findByRole("button", { name: new RegExp(model, "u") }),
            ).toBeVisible();

            const writes = invoke.mock.calls.filter(([op]) => op === "op_set_provider_profile");
            expect(writes).toHaveLength(1);
            const call = writes[0];
            if (call === undefined) {
                throw new Error("La création doit envoyer une commande.");
            }
            const { command_id, aggregate_id, ...write } = submittedInput(call[1].input);
            expect(command_id).toMatch(freshUuid);
            expect(aggregate_id).toMatch(freshUuid);
            expect(write).toEqual({
                expected_revision: 0,
                project_id: project.project_id,
                payload: {
                    provider_kind: "ollama",
                    dialect: { kind: "ollama_chat", version: 1 },
                    endpoint: "http://127.0.0.1:11434",
                    model,
                    locality: "local_only",
                    server_management: "user_managed",
                    auth: { kind: "none" },
                    limits: {
                        max_inflight: 1,
                        connect_timeout_ms: 2000,
                        queue_timeout_ms: 1000,
                        load_timeout_ms: 180000,
                        first_token_timeout_ms: 60000,
                        generation_timeout_ms: 300000,
                    },
                    declared_capabilities: [],
                    pricing: { kind: "none_local" },
                },
            });
            const reads = invoke.mock.calls.filter(([op]) => op === "op_get_provider_profile");
            expect(reads).toHaveLength(1);
            expect(reads[0]?.[1].input).toEqual({
                project_id: project.project_id,
                payload: { profile_id: aggregate_id, profile_version: 1 },
            });
            expect(invoke.mock.calls.some(([op]) => op === "op_test_provider_profile")).toBe(false);
        },
    );

    it("reprend une soumission LM Studio après attente avec les mêmes identifiants", async () => {
        let lastWrite: GuiCommandInput | null = null;
        const { invoke, user } = fixture((operation, { input }) => {
            switch (operation) {
                case "op_list_projects":
                    return Promise.resolve(response([]));
                case "op_list_provider_profiles":
                    return Promise.resolve(response({ profiles: [], next_after_profile_id: null }));
                case "op_set_provider_profile": {
                    const wasSubmitted = lastWrite !== null;
                    lastWrite = submittedInput(input);
                    if (!wasSubmitted) {
                        return Promise.reject(
                            Object.assign(new Error("Soumission temporairement indisponible"), {
                                code: "ResourceBusy",
                                retry_class: "after_backoff",
                            }),
                        );
                    }
                    return Promise.resolve({
                        ...completedReceipt(lastWrite),
                        receipt_id: configurationReceiptId,
                    });
                }
                case "op_get_provider_profile":
                    return Promise.resolve(
                        response({
                            ...initialProfile,
                            profile_id: lastWrite?.aggregate_id,
                            profile_version: 1,
                            configuration_receipt_id: configurationReceiptId,
                            qualification: "required",
                            observations: [],
                        }),
                    );
                default:
                    throw new Error(`Appel inattendu à ${operation}`);
            }
        });
        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await user.click(screen.getByRole("button", { name: "Ajouter un profil local" }));
        await user.selectOptions(screen.getByLabelText("Fournisseur"), "lmstudio");
        await user.type(screen.getByLabelText("Modèle"), "exemple-local");
        await user.click(screen.getByRole("button", { name: "Enregistrer le profil" }));
        expect(await screen.findByRole("alert")).toHaveTextContent(
            "Enregistrement temporairement indisponible.",
        );
        const writes = () =>
            invoke.mock.calls.filter(([operation]) => operation === "op_set_provider_profile");
        expect(writes()).toHaveLength(1);
        expect(writes()[0]?.[1].input).toMatchObject({
            project_id: null,
            payload: {
                provider_kind: "lmstudio",
                dialect: { kind: "openai_chat_completions", version: 1 },
                endpoint: "http://127.0.0.1:1234/v1",
            },
        });
        await user.click(screen.getByRole("button", { name: "Reprendre l’enregistrement" }));
        expect(await screen.findByText("Profil enregistré.")).toBeVisible();
        expect(writes()).toHaveLength(2);
        expect(writes()[1]).toEqual(writes()[0]);
    });

    it("ignore un reçu tardif après le passage de la portée utilisateur à Atlas", async () => {
        let resolveCreation!: (receipt: CommandReceipt) => void;
        let pendingWrite: GuiCommandInput | null = null;
        const receipt = new Promise<CommandReceipt>((resolve) => {
            resolveCreation = resolve;
        });
        const { invoke, user } = fixture(async (operation, { input }) => {
            switch (operation) {
                case "op_list_projects":
                    return response([project]);
                case "op_list_provider_profiles":
                    return input.project_id === project.project_id
                        ? response({
                              profiles: [{ ...initialProfile, model: "atlas-model" }],
                              next_after_profile_id: null,
                          })
                        : response({ profiles: [], next_after_profile_id: null });
                case "op_set_provider_profile":
                    pendingWrite = submittedInput(input);
                    return receipt;
                default:
                    throw new Error(`Appel inattendu à ${operation}`);
            }
        });
        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await screen.findByRole("option", { name: /Atlas/ });
        await user.click(screen.getByRole("button", { name: "Ajouter un profil local" }));
        await user.selectOptions(screen.getByLabelText("Fournisseur"), "llama_cpp");
        await user.type(screen.getByLabelText("Modèle"), "exemple-local");
        await user.click(screen.getByRole("button", { name: "Enregistrer le profil" }));
        expect(await screen.findByText("Enregistrement en cours…")).toBeVisible();
        await user.selectOptions(screen.getByLabelText("Portée"), project.project_id);
        expect(await screen.findByRole("button", { name: /atlas-model/ })).toBeVisible();
        await act(async () => {
            if (pendingWrite === null) {
                throw new Error("Le profil utilisateur doit être en attente.");
            }
            expect(pendingWrite).toMatchObject({
                project_id: null,
                payload: {
                    provider_kind: "llama_cpp",
                    dialect: { kind: "openai_chat_completions", version: 1 },
                    endpoint: "http://127.0.0.1:8080/v1",
                },
            });
            resolveCreation(completedReceipt(pendingWrite));
            await receipt;
        });
        expect(screen.queryByText("Profil enregistré.")).not.toBeInTheDocument();
        expect(
            invoke.mock.calls.some(([operation]) => operation === "op_get_provider_profile"),
        ).toBe(false);
    });
});

describe("T-F012-018 — vérification explicite de la configuration", () => {
    it("soumet le contrôle local de la version projet et relit son observation", async () => {
        let submitted: GuiCommandInput | null = null;
        const { invoke, user } = fixture((command, { input }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([project]));
                case "op_list_provider_profiles":
                    return Promise.resolve(
                        response({ ...listResponse.result, next_after_profile_id: null }),
                    );
                case "op_get_provider_profile":
                    return Promise.resolve(
                        detailResponse(
                            submitted === null ? [] : [observation(submitted.aggregate_id)],
                        ),
                    );
                case "op_test_provider_profile":
                    submitted = submittedInput(input);
                    return Promise.resolve(completedReceipt(submitted));
                default:
                    throw new Error(`Appel inattendu à ${command}`);
            }
        });

        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await user.selectOptions(
            screen.getByRole("combobox", { name: "Portée" }),
            project.project_id,
        );
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        expect(
            await screen.findByText("Aucune observation enregistrée pour cette version."),
        ).toBeVisible();
        expect(invoke.mock.calls.some(([command]) => command === "op_test_provider_profile")).toBe(
            false,
        );

        const check = screen.getByRole("button", { name: "Vérifier la configuration" });
        await user.tab();
        expect(check).toHaveFocus();
        await user.keyboard("{Enter}");
        expect(await screen.findByText("Configuration validée")).toBeVisible();
        expect(screen.getAllByText("Qualification requise")).toHaveLength(2);
        expect(check).toHaveFocus();

        const requests = invoke.mock.calls.filter(
            ([command]) => command === "op_test_provider_profile",
        );
        expect(requests).toHaveLength(1);
        const [call] = requests;
        if (call === undefined) throw new Error("Le contrôle n’a pas été transmis.");
        const issued = submittedInput(call[1].input);
        const { aggregate_id: probeId, command_id: commandId, ...selection } = issued;
        expect(probeId).toMatch(freshUuid);
        expect(commandId).toMatch(freshUuid);
        expect(commandId).not.toBe(probeId);
        expect(selection).toEqual({
            project_id: project.project_id,
            expected_revision: 0,
            payload: {
                profile_id: initialProfile.profile_id,
                profile_version: initialProfile.profile_version,
                phase: "validate_config",
                capability_set: [],
                limits: {
                    timeout_ms: 1000,
                    max_calls: 0,
                    max_output_tokens: 0,
                    max_cost_usd_micros: 0,
                },
                locality: "local_only",
                mandate_ref: configurationReceiptId,
            },
        });
        const detail = {
            project_id: project.project_id,
            payload: {
                profile_id: initialProfile.profile_id,
                profile_version: initialProfile.profile_version,
            },
        };
        expect(invoke.mock.calls.slice(-3)).toEqual([
            ["op_get_provider_profile", { input: detail }],
            ["op_test_provider_profile", { input: issued }],
            ["op_get_provider_profile", { input: detail }],
        ]);
    });

    it("conserve les identifiants du contrôle pour une reprise autorisée", async () => {
        let submissions = 0;
        let committed: GuiCommandInput | null = null;
        const { invoke, user } = fixture((command, { input }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([]));
                case "op_list_provider_profiles":
                    return Promise.resolve(listResponse);
                case "op_get_provider_profile":
                    return Promise.resolve(
                        detailResponse(
                            committed === null ? [] : [observation(committed.aggregate_id)],
                        ),
                    );
                case "op_test_provider_profile":
                    submissions += 1;
                    if (submissions === 1) {
                        return Promise.reject(
                            Object.assign(new Error("Réessai après attente"), {
                                code: "ResourceBusy",
                                reason_code: null,
                                retry_class: "after_backoff",
                            }),
                        );
                    }
                    committed = submittedInput(input);
                    return Promise.resolve(completedReceipt(committed));
                default:
                    throw new Error(`Appel inattendu à ${command}`);
            }
        });

        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        await screen.findByText("Aucune observation enregistrée pour cette version.");
        await user.click(screen.getByRole("button", { name: "Vérifier la configuration" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("Vérification non confirmée.");
        expect(screen.queryByText("Configuration validée")).not.toBeInTheDocument();
        expect(submissions).toBe(1);

        await user.click(screen.getByRole("button", { name: "Reprendre la vérification" }));
        expect(await screen.findByText("Configuration validée")).toBeVisible();
        const requests = invoke.mock.calls.filter(
            ([command]) => command === "op_test_provider_profile",
        );
        expect(requests).toHaveLength(2);
        expect(requests[0]).toEqual(requests[1]);
    });

    it("retrouve le contrôle enregistré quand son reçu n’atteint pas le renderer", async () => {
        let committed: GuiCommandInput | null = null;
        const { invoke, user } = fixture((command, { input }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([]));
                case "op_list_provider_profiles":
                    return Promise.resolve(listResponse);
                case "op_get_provider_profile":
                    return Promise.resolve(
                        detailResponse(
                            committed === null ? [] : [observation(committed.aggregate_id)],
                        ),
                    );
                case "op_test_provider_profile":
                    committed = submittedInput(input);
                    return Promise.reject(new Error("Réponse de la commande perdue"));
                default:
                    throw new Error(`Appel inattendu à ${command}`);
            }
        });

        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        await screen.findByText("Aucune observation enregistrée pour cette version.");
        await user.click(screen.getByRole("button", { name: "Vérifier la configuration" }));
        expect(await screen.findByText("Vérification enregistrée.")).toBeVisible();
        expect(screen.getByText("Configuration validée")).toBeVisible();
        expect(invoke.mock.calls.slice(-3).map(([command]) => command)).toEqual([
            "op_get_provider_profile",
            "op_test_provider_profile",
            "op_get_provider_profile",
        ]);
    });

    it("relit un résultat absent sans envoyer de nouveau contrôle", async () => {
        let submitted: GuiCommandInput | null = null;
        let detailsAfterValidation = 0;
        const { invoke, user } = fixture((command, { input }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([]));
                case "op_list_provider_profiles":
                    return Promise.resolve(listResponse);
                case "op_get_provider_profile":
                    if (submitted === null) return Promise.resolve(detailResponse([]));
                    detailsAfterValidation += 1;
                    return Promise.resolve(
                        detailsAfterValidation === 1
                            ? detailResponse()
                            : detailResponse([observation(submitted.aggregate_id)]),
                    );
                case "op_test_provider_profile":
                    submitted = submittedInput(input);
                    return Promise.resolve(completedReceipt(submitted));
                default:
                    throw new Error(`Appel inattendu à ${command}`);
            }
        });

        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        await screen.findByText("Aucune observation enregistrée pour cette version.");
        await user.click(screen.getByRole("button", { name: "Vérifier la configuration" }));
        expect(await screen.findByRole("alert")).toHaveTextContent(
            "Résultat de cette vérification indisponible.",
        );
        await user.click(screen.getByRole("button", { name: "Relire le résultat" }));
        await screen.findByRole("button", { name: "Vérifier la configuration" });
        expect(screen.queryByRole("alert")).not.toBeInTheDocument();
        expect(detailsAfterValidation).toBe(2);
        expect(
            invoke.mock.calls.filter(([command]) => command === "op_test_provider_profile"),
        ).toHaveLength(1);
    });
});

describe("T-F012-018 — lecture des profils dans Settings", () => {
    it("consulte la version affichée et poursuit la liste avec le curseur du daemon", async () => {
        const { invoke, user } = fixture((command, { input }) => {
            switch (command) {
                case "op_list_projects":
                    return Promise.resolve(response([project]));
                case "op_list_provider_profiles":
                    return Promise.resolve(
                        input.payload.after_profile_id === null
                            ? listResponse
                            : response({ profiles: [], next_after_profile_id: null }),
                    );
                case "op_get_provider_profile":
                    return Promise.resolve(detailResponse());
                default:
                    throw new Error(`Appel inattendu à ${command}`);
            }
        });

        expect(invoke).not.toHaveBeenCalled();
        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        const profile = await screen.findByRole("button", { name: /first-v2/ });
        expect(invoke).toHaveBeenCalledWith("op_list_provider_profiles", {
            input: { project_id: null, payload: { limit: 50, after_profile_id: null } },
        });
        await user.click(profile);
        expect(await screen.findByText("Configuration validée")).toBeVisible();
        expect(screen.getByText(initialProfile.profile_version_ref)).toBeVisible();
        expect(invoke).toHaveBeenCalledWith("op_get_provider_profile", {
            input: {
                project_id: null,
                payload: {
                    profile_id: initialProfile.profile_id,
                    profile_version: initialProfile.profile_version,
                },
            },
        });

        await user.click(screen.getByRole("button", { name: "Profils suivants" }));
        expect(await screen.findByText("Aucun profil dans cette portée.")).toBeVisible();
        expect(screen.queryByText("Configuration validée")).not.toBeInTheDocument();
        expect(invoke).toHaveBeenCalledWith("op_list_provider_profiles", {
            input: {
                project_id: null,
                payload: { limit: 50, after_profile_id: listResponse.result.next_after_profile_id },
            },
        });
        expect(invoke.mock.calls.map(([command]) => command)).toEqual([
            "op_list_projects",
            "op_list_provider_profiles",
            "op_get_provider_profile",
            "op_list_provider_profiles",
        ]);
    });

    it("conserve la portée projet malgré un détail tardif et un échec de relecture", async () => {
        let projectReads = 0;
        let resolveDetail: (value: QueryResponse) => void = () => {
            throw new Error("La lecture du détail doit être en attente.");
        };
        const pendingDetail = new Promise<QueryResponse>((resolve) => {
            resolveDetail = resolve;
        });
        const { invoke, user } = fixture(async (command, { input }) => {
            if (command === "op_list_projects") {
                projectReads += 1;
                if (projectReads > 1) {
                    throw new Error("Le catalogue de projets est indisponible.");
                }
                return response([project]);
            }
            if (command === "op_get_provider_profile") {
                return pendingDetail;
            }
            if (command === "op_list_provider_profiles") {
                return input.project_id === project.project_id
                    ? response({
                          profiles: [{ ...initialProfile, model: "atlas-model" }],
                          next_after_profile_id: null,
                      })
                    : listResponse;
            }
            throw new Error(`Appel inattendu à ${command}`);
        });
        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        expect(await screen.findByText("Lecture du détail…")).toBeVisible();
        await user.selectOptions(
            screen.getByRole("combobox", { name: "Portée" }),
            project.project_id,
        );
        expect(await screen.findByRole("button", { name: /atlas-model/ })).toBeVisible();
        expect(invoke).toHaveBeenCalledWith("op_list_provider_profiles", {
            input: {
                project_id: project.project_id,
                payload: { limit: 50, after_profile_id: null },
            },
        });
        await act(async () => {
            resolveDetail(detailResponse());
            await pendingDetail;
        });
        expect(screen.queryByText("Configuration validée")).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: /first-v2/ })).not.toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Actualiser les profils" }));
        expect(await screen.findByRole("alert")).toHaveTextContent(
            "Impossible de lire les projets.",
        );
        expect(await screen.findByRole("button", { name: /atlas-model/ })).toBeVisible();
        expect(screen.getByRole("combobox", { name: "Portée" })).toHaveValue(project.project_id);
    });

    it("signale un résultat illisible et permet de relire une liste vide authentique", async () => {
        let listRequests = 0;
        const { user } = fixture((command) => {
            if (command === "op_list_projects") {
                return Promise.resolve(response([]));
            }
            if (command === "op_list_provider_profiles") {
                listRequests += 1;
                return Promise.resolve(
                    response(
                        listRequests === 1
                            ? { fixture: true }
                            : { profiles: [], next_after_profile_id: null },
                    ),
                );
            }
            throw new Error(`Appel inattendu à ${command}`);
        });
        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        expect(await screen.findByRole("alert")).toHaveTextContent(
            "Impossible de lire les profils.",
        );
        expect(screen.queryByText("Aucun profil dans cette portée.")).not.toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Actualiser les profils" }));
        expect(await screen.findByText("Aucun profil dans cette portée.")).toBeVisible();
        expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
});

describe("T-F012-018 — lecture de l’inventaire observé", () => {
    const ollamaProfile = { ...initialProfile, provider_kind: "ollama" };

    function inventoryModel(name: string, details: Record<string, string | null> | null = null) {
        return { name, model: null, modified_at: null, size: null, digest: null, details };
    }

    function settingsInventory(
        models: ReturnType<typeof inventoryModel>[] | null,
        version = ollamaProfile.profile_version,
    ) {
        const probeId = "00000000-0000-4000-8000-000000000041";
        return fixture((operation) => {
            switch (operation) {
                case "op_list_projects":
                    return Promise.resolve(response([project]));
                case "op_list_provider_profiles":
                    return Promise.resolve(
                        response({ profiles: [ollamaProfile], next_after_profile_id: null }),
                    );
                case "op_get_provider_profile":
                    return Promise.resolve(
                        response({
                            ...ollamaProfile,
                            profile_version: version,
                            configuration_receipt_id: configurationReceiptId,
                            observations: [
                                {
                                    phase: "inventory",
                                    status: "inventory_observed",
                                    observed_at_ms: 1795348800000,
                                    probe_id: probeId,
                                    result_ref: `provider-probe:${probeId}`,
                                    inventory: {
                                        fact_ref: {
                                            evidence_id: "00000000-0000-4000-8000-000000000042",
                                            object_hash: "cd".repeat(32),
                                        },
                                        inventory_digest: "ef".repeat(32),
                                        models,
                                    },
                                },
                            ],
                        }),
                    );
                default:
                    throw new Error(`Appel inattendu à ${operation}`);
            }
        });
    }

    it("affiche les modèles de la version projet et les parcourt sans nouvel appel IPC", async () => {
        const first = "<modele-observe:1>";
        const last = "dernier-modèle:26";
        const { invoke, user } = settingsInventory(
            [
                first,
                ...Array.from({ length: 24 }, (_, index) => `modèle:${String(index + 2)}`),
                last,
            ].map((name) => inventoryModel(name)),
        );
        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await screen.findByRole("option", { name: /Atlas/ });
        await user.selectOptions(
            screen.getByRole("combobox", { name: "Portée" }),
            project.project_id,
        );
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        expect(await screen.findByText("Inventaire observé")).toBeVisible();
        expect(screen.getByText(first)).toBeVisible();
        expect(screen.queryByText(last)).not.toBeInTheDocument();
        expect(screen.getAllByText("Qualification requise")).toHaveLength(2);
        expect(invoke).toHaveBeenLastCalledWith("op_get_provider_profile", {
            input: {
                project_id: project.project_id,
                payload: {
                    profile_id: ollamaProfile.profile_id,
                    profile_version: ollamaProfile.profile_version,
                },
            },
        });
        expect(invoke.mock.calls.every(([operation]) => !operation.startsWith("op_test"))).toBe(
            true,
        );
        const reads = invoke.mock.calls.length;
        await user.click(screen.getByRole("button", { name: "Modèles suivants" }));
        expect(screen.getByText(last)).toBeVisible();
        expect(screen.queryByText(first)).not.toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Modèles précédents" }));
        expect(screen.getByText(first)).toBeVisible();
        expect(invoke).toHaveBeenCalledTimes(reads);
    });

    it("présente le format et la quantification uniquement pour le modèle qui les déclare", async () => {
        const { invoke, user } = settingsInventory([
            inventoryModel("qwen3:8b", {
                format: "gguf",
                family: null,
                parameter_size: null,
                quantization_level: "Q4_K_M",
            }),
            inventoryModel("modèle-sans-métadonnées"),
        ]);
        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        const models = await screen.findByRole("list", { name: "Modèles observés" });
        const [withDetails, withoutDetails] = within(models).getAllByRole("listitem");
        expect(withDetails).toBeDefined();
        expect(withoutDetails).toBeDefined();
        expect(withDetails).toHaveTextContent("qwen3:8b");
        expect(withDetails).toHaveTextContent("Format gguf");
        expect(withDetails).toHaveTextContent("Quantification Q4_K_M");
        expect(withoutDetails).toHaveTextContent(/^modèle-sans-métadonnées$/u);
        expect(screen.getAllByText("Qualification requise")).toHaveLength(2);
        expect(invoke.mock.calls.map(([operation]) => operation)).toEqual([
            "op_list_projects",
            "op_list_provider_profiles",
            "op_get_provider_profile",
        ]);
    });

    it("affiche une liste vide constatée par le daemon", async () => {
        const { user } = settingsInventory([]);
        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        expect(await screen.findByText("Aucun modèle dans cet inventaire.")).toBeVisible();
    });

    it.each(["accepté", "perdu"])("retrouve l’inventaire avec un reçu %s", async (mode) => {
        const mandateRef = "00000000-0000-4000-8000-000000000071";
        const configurationDigest = "ab".repeat(32);
        const mandate = {
            mandate_ref: mandateRef,
            scope_kind: "project",
            scope_id: project.project_id,
            authority_generation: "00000000-0000-4000-8000-000000000072",
            principal_id: "fixture-local-admin",
            profile_id: ollamaProfile.profile_id,
            profile_version: ollamaProfile.profile_version,
            configuration_digest: configurationDigest,
            phase: "inventory",
            locality: "local_only",
            capability_set: ["text"],
            limits: {
                timeout_ms: 1750,
                max_calls: 0,
                max_output_tokens: 0,
                max_cost_usd_micros: 0,
            },
            expires_at_ms: 1800000000000,
            mandate_digest: "bc".repeat(32),
            status: "active",
            revocation_receipt_id: null,
        };
        let showResult = mode === "accepté";
        let observationForProbe: Record<string, unknown> | null = null;
        const { invoke, user } = fixture((operation, { input }) => {
            switch (operation) {
                case "op_list_projects":
                    return Promise.resolve(response([project]));
                case "op_list_provider_profiles":
                    return Promise.resolve(
                        response({ profiles: [ollamaProfile], next_after_profile_id: null }),
                    );
                case "op_get_provider_profile":
                    return Promise.resolve(
                        response({
                            ...ollamaProfile,
                            configuration_receipt_id: configurationReceiptId,
                            configuration_digest: configurationDigest,
                            observations:
                                observationForProbe === null || !showResult
                                    ? []
                                    : [observationForProbe],
                        }),
                    );
                case "op_read_policy_summary":
                    return Promise.resolve(
                        response({
                            provider_probe_mandates: [
                                {
                                    ...mandate,
                                    mandate_ref: configurationReceiptId,
                                    phase: "connect",
                                },
                                mandate,
                            ],
                        }),
                    );
                case "op_test_provider_profile": {
                    const command = submittedInput(input);
                    const resultRef = `provider-probe:${command.aggregate_id}`;
                    observationForProbe = {
                        phase: "inventory",
                        status: "inventory_observed",
                        observed_at_ms: 1795348800000,
                        probe_id: command.aggregate_id,
                        result_ref: resultRef,
                        inventory: { models: [inventoryModel("modèle-du-probe:8b")] },
                    };
                    return mode === "perdu"
                        ? Promise.reject(
                              Object.assign(new Error("Reçu indisponible"), {
                                  retry_class: "after_reconciliation",
                              }),
                          )
                        : Promise.resolve({
                              ...completedReceipt(command),
                              state: "accepted",
                              result_ref: resultRef,
                          });
                }
                default:
                    throw new Error(`Appel inattendu à ${operation}`);
            }
        });

        await user.click(screen.getByRole("tab", { name: /Settings/ }));
        await screen.findByRole("option", { name: /Atlas/ });
        await user.selectOptions(
            screen.getByRole("combobox", { name: "Portée" }),
            project.project_id,
        );
        await user.click(await screen.findByRole("button", { name: /first-v2/ }));
        await screen.findByText("Référence de version");
        expect(invoke.mock.calls.map(([operation]) => operation)).not.toContain(
            "op_read_policy_summary",
        );
        await user.click(screen.getByRole("button", { name: "Choisir un mandat d’inventaire" }));
        const choice = await screen.findByRole("combobox", { name: "Mandat d’inventaire" });
        expect(within(choice).getAllByRole("option")).toHaveLength(2);
        expect(invoke).toHaveBeenLastCalledWith("op_read_policy_summary", {
            input: { project_id: project.project_id, payload: {} },
        });
        expect(screen.getByRole("button", { name: "Lancer l’inventaire" })).toBeDisabled();
        await user.selectOptions(choice, mandateRef);
        await user.click(screen.getByRole("button", { name: "Lancer l’inventaire" }));

        if (mode === "perdu") {
            expect(
                await screen.findByText("Résultat de cet inventaire indisponible."),
            ).toBeVisible();
            expect(screen.queryByText("modèle-du-probe:8b")).not.toBeInTheDocument();
            showResult = true;
            await user.click(
                screen.getByRole("button", { name: "Relire le résultat de l’inventaire" }),
            );
        }

        expect(await screen.findByText("modèle-du-probe:8b")).toBeVisible();
        expect(screen.getByText("Inventaire enregistré.")).toBeVisible();
        expect(screen.getAllByText("Qualification requise")).toHaveLength(2);
        const commands = invoke.mock.calls.filter(([operation]) => operation.startsWith("op_test"));
        expect(commands).toHaveLength(1);
        expect(commands[0]).toEqual([
            "op_test_provider_profile",
            {
                input: {
                    project_id: project.project_id,
                    aggregate_id: expect.stringMatching(freshUuid) as unknown as string,
                    command_id: expect.stringMatching(freshUuid) as unknown as string,
                    expected_revision: 0,
                    payload: {
                        profile_id: ollamaProfile.profile_id,
                        profile_version: 2,
                        phase: "inventory",
                        capability_set: [],
                        locality: "local_only",
                        limits: mandate.limits,
                        mandate_ref: mandateRef,
                    },
                },
            },
        ]);
    });

    it.each(["illisible", "d’une autre version"])(
        "refuse un inventaire %s sans le présenter comme vide",
        async (variant) => {
            const model = "autre-version:latest";
            const { user } =
                variant === "illisible"
                    ? settingsInventory(null)
                    : settingsInventory([inventoryModel(model)], ollamaProfile.profile_version + 1);
            await user.click(screen.getByRole("tab", { name: /Settings/ }));
            await user.click(await screen.findByRole("button", { name: /first-v2/ }));
            expect(await screen.findByRole("alert")).toHaveTextContent(
                "Impossible de lire cette version.",
            );
            expect(screen.queryByText("Aucun modèle dans cet inventaire.")).not.toBeInTheDocument();
            expect(screen.queryByText(model)).not.toBeInTheDocument();
        },
    );
});
