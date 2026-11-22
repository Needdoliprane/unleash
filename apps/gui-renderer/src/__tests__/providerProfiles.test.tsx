import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import listResponse from "../../../../.claude/examples/ipc-provider-profile-list-response.json";
import { App } from "../App";
import type { GuiQueryInput, QueryResponse } from "../ipc/types";

const [initialProfile] = listResponse.result.profiles;
if (initialProfile === undefined) {
    throw new Error("La fixture de liste doit contenir un profil.");
}
const project = {
    project_id: "00000000-0000-4000-8000-000000000009",
    name: "Atlas",
};

type Invoke = (command: string, args: { input: GuiQueryInput }) => Promise<QueryResponse>;

function response(result: unknown): QueryResponse {
    return { ...listResponse, result };
}

function detailResponse(): QueryResponse {
    return response({
        ...initialProfile,
        observations: [
            {
                phase: "validate_config",
                status: "validated",
                observed_at_ms: 1795348800000,
                probe_id: "00000000-0000-4000-8000-000000000010",
            },
        ],
    });
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
