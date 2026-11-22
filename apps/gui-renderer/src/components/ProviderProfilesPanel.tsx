import { useEffect, useMemo, useState } from "react";
import { Button } from "react-aria-components";
import {
    profileDetail,
    profilePage,
    projectChoices,
    type ProviderProfileRow,
} from "../ipc/providerProfileViews";
import type { GuiQueryInput, OperationName, TransportAdapter } from "../ipc/types";

type Reading<T> = { state: "loading" } | { state: "error" } | { state: "ready"; data: T };

function useSettingsRead<T>(
    transport: TransportAdapter,
    operation: OperationName,
    request: GuiQueryInput | null,
    read: (value: unknown) => T,
    attempt: number,
): Reading<T> {
    const [settled, setSettled] = useState<{
        request: GuiQueryInput;
        transport: TransportAdapter;
        attempt: number;
        result: Reading<T>;
    } | null>(null);

    useEffect(() => {
        if (request === null) {
            return;
        }
        const requested = request;
        let current = true;
        function finish(result: Reading<T>) {
            if (current) {
                setSettled({ request: requested, transport, attempt, result });
            }
        }
        void transport.executeQuery(operation, request).then(
            ({ result }) => {
                try {
                    finish({ state: "ready", data: read(result) });
                } catch {
                    finish({ state: "error" });
                }
            },
            () => {
                finish({ state: "error" });
            },
        );
        return () => {
            current = false;
        };
    }, [attempt, operation, read, request, transport]);

    if (
        settled === null ||
        settled.request !== request ||
        settled.transport !== transport ||
        settled.attempt !== attempt
    ) {
        return { state: "loading" };
    }
    return settled.result;
}

const PROJECTS_REQUEST: GuiQueryInput = { project_id: null, payload: {} };
const PROVIDER_LABELS = new Map([
    ["openai", "OpenAI"],
    ["anthropic", "Anthropic"],
    ["lmstudio", "LM Studio"],
    ["ollama", "Ollama"],
    ["llama_cpp", "llama.cpp"],
    ["openai_compatible", "Compatible OpenAI"],
]);
const PHASE_LABELS = new Map([
    ["validate_config", "Configuration"],
    ["connect", "Connexion"],
    ["inventory", "Inventaire"],
    ["generation_probe", "Génération"],
]);
const DATE_FORMAT = new Intl.DateTimeFormat("fr-FR", { dateStyle: "medium", timeStyle: "short" });

function qualificationLabel(value: string) {
    return value === "required" ? "Qualification requise" : value;
}

function observationLabel(phase: string, status: string) {
    return status === "validated"
        ? "Configuration validée"
        : `${PHASE_LABELS.get(phase) ?? phase} · ${status}`;
}

function SelectedProfile({
    profile,
    read,
}: {
    profile: ProviderProfileRow;
    read: Reading<ReturnType<typeof profileDetail>>;
}) {
    return (
        <section className="provider-detail" aria-labelledby="provider-detail-title">
            <h3 id="provider-detail-title">{profile.model}</h3>
            {read.state === "loading" ? <p role="status">Lecture du détail…</p> : null}
            {read.state === "error" ? <p role="alert">Impossible de lire cette version.</p> : null}
            {read.state === "ready" ? (
                <>
                    <dl className="provider-detail-fields">
                        <dt>Version configurée</dt>
                        <dd>{read.data.version}</dd>
                        <dt>Référence de version</dt>
                        <dd className="mono">{read.data.reference}</dd>
                        <dt>Qualification</dt>
                        <dd>{qualificationLabel(read.data.qualification)}</dd>
                    </dl>
                    <h4>Observations enregistrées</h4>
                    {read.data.observations.length === 0 ? (
                        <p>Aucune observation enregistrée pour cette version.</p>
                    ) : (
                        <ul className="provider-observations">
                            {read.data.observations.map(({ id, phase, status, observedAt }) => (
                                <li key={id}>
                                    <span>{observationLabel(phase, status)}</span>
                                    <time dateTime={observedAt}>
                                        {DATE_FORMAT.format(new Date(observedAt))}
                                    </time>
                                </li>
                            ))}
                        </ul>
                    )}
                </>
            ) : null}
        </section>
    );
}

export function ProviderProfilesPanel({ transport }: { transport: TransportAdapter }) {
    const [scope, setScope] = useState<string | null>(null);
    const [cursor, setCursor] = useState<string | null>(null);
    const [selected, setSelected] = useState<ProviderProfileRow | null>(null);
    const [attempt, setAttempt] = useState(0);
    const listRequest = useMemo(
        () => ({ project_id: scope, payload: { limit: 50, after_profile_id: cursor } }),
        [scope, cursor],
    );
    const detailRequest = useMemo(
        () =>
            selected === null
                ? null
                : {
                      project_id: scope,
                      payload: { profile_id: selected.id, profile_version: selected.version },
                  },
        [scope, selected],
    );
    const projects = useSettingsRead(
        transport,
        "ListProjects",
        PROJECTS_REQUEST,
        projectChoices,
        attempt,
    );
    const profiles = useSettingsRead(
        transport,
        "ListProviderProfiles",
        listRequest,
        profilePage,
        attempt,
    );
    const detail = useSettingsRead(
        transport,
        "GetProviderProfile",
        detailRequest,
        profileDetail,
        attempt,
    );

    function moveToPage(after: string | null) {
        setCursor(after);
        setSelected(null);
    }

    function reload() {
        moveToPage(null);
        setAttempt((value) => value + 1);
    }

    const detailMatches =
        detail.state !== "ready" ||
        (selected !== null &&
            detail.data.id === selected.id &&
            detail.data.version === selected.version &&
            detail.data.reference === selected.reference);
    const showCurrentScope =
        scope !== null &&
        (projects.state !== "ready" || !projects.data.some(({ id }) => id === scope));

    return (
        <section className="provider-settings" aria-labelledby="provider-settings-title">
            <div className="provider-settings-header">
                <h2 id="provider-settings-title">Profils de modèles</h2>
                <Button className="control-button" onPress={reload}>
                    Actualiser les profils
                </Button>
            </div>
            <div className="provider-scope">
                <label htmlFor="provider-scope">Portée</label>
                <select
                    id="provider-scope"
                    value={scope ?? ""}
                    className="control-input"
                    onChange={(event) => {
                        setScope(event.currentTarget.value || null);
                        moveToPage(null);
                    }}
                >
                    <option value="">Utilisateur</option>
                    {showCurrentScope ? <option value={scope}>Projet {scope}</option> : null}
                    {projects.state === "ready"
                        ? projects.data.map(({ id, name }) => (
                              <option key={id} value={id}>
                                  {name} · {id}
                              </option>
                          ))
                        : null}
                </select>
                {projects.state === "loading" ? (
                    <span role="status">Lecture des projets…</span>
                ) : null}
                {projects.state === "error" ? (
                    <span role="alert">Impossible de lire les projets.</span>
                ) : null}
            </div>

            <div className="provider-layout">
                <div className="provider-list-region">
                    {profiles.state === "loading" ? (
                        <p role="status">Lecture des profils…</p>
                    ) : null}
                    {profiles.state === "error" ? (
                        <p role="alert">Impossible de lire les profils.</p>
                    ) : null}
                    {profiles.state === "ready" ? (
                        profiles.data.profiles.length === 0 ? (
                            <p>Aucun profil dans cette portée.</p>
                        ) : (
                            <ul className="provider-list" aria-label="Profils configurés">
                                {profiles.data.profiles.map((profile) => (
                                    <li key={profile.id}>
                                        <Button
                                            className="provider-row"
                                            aria-pressed={selected?.reference === profile.reference}
                                            onPress={() => {
                                                setSelected(profile);
                                            }}
                                        >
                                            <strong>{profile.model}</strong>
                                            <span>
                                                {PROVIDER_LABELS.get(profile.provider) ??
                                                    profile.provider}
                                            </span>
                                            <span className="provider-version">
                                                Version {profile.version}
                                            </span>
                                            <span className="provider-qualification">
                                                {qualificationLabel(profile.qualification)}
                                            </span>
                                        </Button>
                                    </li>
                                ))}
                            </ul>
                        )
                    ) : null}
                    <div className="provider-pagination">
                        {cursor !== null ? (
                            <Button
                                className="control-button"
                                onPress={() => {
                                    moveToPage(null);
                                }}
                            >
                                Première page
                            </Button>
                        ) : null}
                        {profiles.state === "ready" && profiles.data.next !== null ? (
                            <Button
                                className="control-button"
                                onPress={() => {
                                    moveToPage(profiles.data.next);
                                }}
                            >
                                Profils suivants
                            </Button>
                        ) : null}
                    </div>
                </div>
                {selected ? (
                    <SelectedProfile
                        profile={selected}
                        read={detailMatches ? detail : { state: "error" }}
                    />
                ) : null}
            </div>
        </section>
    );
}
