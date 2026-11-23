import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Button } from "react-aria-components";
import {
    profileDetail,
    profilePage,
    projectChoices,
    transportProbeMandates,
    type InventoryModel,
    type ProviderProfileRow,
    type TransportProbePhase,
} from "../ipc/providerProfileViews";
import type { GuiCommandInput, GuiQueryInput, OperationName, TransportAdapter } from "../ipc/types";
import { newUuid } from "../ipc/uuid";
import { LocalProviderProfileForm } from "./LocalProviderProfileForm";

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
    if (phase === "connect" && status === "connected") return "Connexion observée";
    return status === "validated"
        ? "Configuration validée"
        : `${PHASE_LABELS.get(phase) ?? phase} · ${status}`;
}

const MODELS_PER_PAGE = 25;

function InventoryModels({ models }: { models: InventoryModel[] }) {
    const [page, setPage] = useState(0);
    if (models.length === 0) {
        return <p className="provider-inventory">Aucun modèle dans cet inventaire.</p>;
    }

    const start = page * MODELS_PER_PAGE;
    const pageCount = Math.ceil(models.length / MODELS_PER_PAGE);
    return (
        <div className="provider-inventory">
            {pageCount > 1 ? (
                <div className="provider-inventory-pages">
                    <Button
                        className="control-button"
                        isDisabled={page === 0}
                        onPress={() => {
                            setPage(page - 1);
                        }}
                    >
                        Modèles précédents
                    </Button>
                    <Button
                        className="control-button"
                        isDisabled={page + 1 === pageCount}
                        onPress={() => {
                            setPage(page + 1);
                        }}
                    >
                        Modèles suivants
                    </Button>
                    <span role="status">
                        Page {page + 1} sur {pageCount}
                    </span>
                </div>
            ) : null}
            <ul aria-label="Modèles observés">
                {models
                    .slice(start, start + MODELS_PER_PAGE)
                    .map(({ name, format, quantization }, index) => (
                        <li key={start + index}>
                            <span>{name.trim() === "" ? "Nom non renseigné" : name}</span>
                            {format !== null || quantization !== null ? (
                                <small className="provider-model-details">
                                    {format === null ? null : <span>Format {format}</span>}
                                    {quantization === null ? null : (
                                        <span>Quantification {quantization}</span>
                                    )}
                                </small>
                            ) : null}
                        </li>
                    ))}
            </ul>
        </div>
    );
}

type ProfileDetail = ReturnType<typeof profileDetail>;
type ProbePhase = "validate_config" | TransportProbePhase;
const OBSERVED_STATUS: Record<ProbePhase, string> = {
    validate_config: "validated",
    connect: "connected",
    inventory: "inventory_observed",
};
type ProbeState =
    | { state: "submitting" | "refused" }
    | { state: "retryable"; command: GuiCommandInput }
    | { state: "observing"; probeId: string; resultRef: string | null };

function useProfileProbe(
    transport: TransportAdapter,
    phase: ProbePhase,
    read: Reading<ProfileDetail>,
    refresh: () => void,
) {
    const [probe, setProbe] = useState<ProbeState | null>(null);
    const submitting = useRef(false);
    const observed =
        probe?.state === "observing" &&
        read.state === "ready" &&
        read.data.observations.some(
            (item) =>
                item.id === probe.probeId &&
                item.phase === phase &&
                item.status === OBSERVED_STATUS[phase] &&
                (probe.resultRef === null || item.resultRef === probe.resultRef),
        );
    const needsRead = probe?.state === "observing" && !observed;
    const checking = probe?.state === "submitting" || (needsRead && read.state === "loading");

    function observe(probeId: string, resultRef: string | null) {
        setProbe({ state: "observing", probeId, resultRef });
        refresh();
    }

    async function run(create: (profile: ProfileDetail) => GuiCommandInput | null) {
        if (checking || submitting.current) return;
        if (needsRead) {
            refresh();
            return;
        }
        if (read.state !== "ready") return;
        const command = probe?.state === "retryable" ? probe.command : create(read.data);
        if (command === null) return;

        submitting.current = true;
        setProbe({ state: "submitting" });
        try {
            const receipt = await transport.submitCommand("TestProviderProfile", command);
            const matches =
                receipt.command_id === command.command_id &&
                receipt.aggregate_id === command.aggregate_id;
            if (matches && (receipt.state === "blocked" || receipt.state === "rejected")) {
                setProbe({ state: "refused" });
            } else {
                observe(
                    command.aggregate_id,
                    matches && (phase !== "validate_config" || receipt.state === "completed")
                        ? (receipt.result_ref ?? null)
                        : null,
                );
            }
        } catch (error) {
            const retryClass =
                typeof error === "object" && error !== null && "retry_class" in error
                    ? error.retry_class
                    : null;
            if (retryClass === "after_backoff") {
                setProbe({ state: "retryable", command });
            } else if (retryClass === "never") {
                setProbe({ state: "refused" });
            } else {
                observe(command.aggregate_id, null);
            }
        } finally {
            submitting.current = false;
        }
    }
    return { state: probe?.state, needsRead, checking, observed, run };
}

function profileProbeCommand(
    scope: string | null,
    profile: ProfileDetail,
    options: {
        phase: ProbePhase;
        mandateRef: string;
        timeoutMs: number;
        locality: "local_only" | "approved_remote";
    },
): GuiCommandInput {
    return {
        project_id: scope,
        aggregate_id: newUuid(),
        command_id: newUuid(),
        expected_revision: 0,
        payload: {
            profile_id: profile.id,
            profile_version: profile.version,
            phase: options.phase,
            capability_set: [],
            limits: {
                timeout_ms: options.timeoutMs,
                max_calls: 0,
                max_output_tokens: 0,
                max_cost_usd_micros: 0,
            },
            locality: options.locality,
            mandate_ref: options.mandateRef,
        },
    };
}

const TRANSPORT_PROBE_LABELS = {
    connect: {
        mandate: "de connexion",
        start: "Tester la connexion",
        resume: "Reprendre le test de connexion",
        readResult: "Relire le résultat du test de connexion",
        pending: "Test de connexion en cours…",
        unconfirmed: "Demande de connexion non confirmée.",
        refused: "Connexion refusée.",
        missing: "Résultat de ce test de connexion indisponible.",
        observed: "Connexion enregistrée.",
    },
    inventory: {
        mandate: "d’inventaire",
        start: "Lancer l’inventaire",
        resume: "Reprendre l’inventaire",
        readResult: "Relire le résultat de l’inventaire",
        pending: "Demande d’inventaire en cours…",
        unconfirmed: "Demande d’inventaire non confirmée.",
        refused: "Inventaire refusé.",
        missing: "Résultat de cet inventaire indisponible.",
        observed: "Inventaire enregistré.",
    },
};

function TransportProbeControls({
    phase,
    scope,
    transport,
    read,
    refresh,
}: {
    phase: TransportProbePhase;
    scope: string | null;
    transport: TransportAdapter;
    read: Reading<ProfileDetail>;
    refresh: () => void;
}) {
    const [attempt, setAttempt] = useState(0);
    const [reference, setReference] = useState("");
    const fieldId = useId();
    const labels = TRANSPORT_PROBE_LABELS[phase];
    const opened = attempt > 0;
    const request = useMemo<GuiQueryInput>(() => ({ project_id: scope, payload: {} }), [scope]);
    const decodeMandates = useCallback(
        (value: unknown) => transportProbeMandates(value, phase),
        [phase],
    );
    const mandates = useSettingsRead(
        transport,
        "ReadPolicySummary",
        opened ? request : null,
        decodeMandates,
        attempt,
    );
    const probe = useProfileProbe(transport, phase, read, refresh);
    const profile = read.state === "ready" ? read.data : null;
    const available =
        mandates.state === "ready" && profile !== null
            ? mandates.data.filter(
                  (mandate) =>
                      mandate.scopeKind === (scope === null ? "global" : "project") &&
                      mandate.scopeId === (scope ?? "global") &&
                      mandate.profileId === profile.id &&
                      mandate.profileVersion === profile.version &&
                      mandate.configurationDigest === profile.configurationDigest,
              )
            : [];
    const selected = available.find((mandate) => mandate.reference === reference);
    const canRetry = probe.state === "retryable";

    return (
        <div className="provider-validation">
            <Button
                className="control-button"
                isDisabled={
                    profile?.configurationDigest == null ||
                    probe.checking ||
                    probe.needsRead ||
                    canRetry
                }
                isPending={opened && mandates.state === "loading"}
                onPress={() => {
                    setReference("");
                    setAttempt((value) => value + 1);
                }}
            >
                {opened ? "Actualiser les mandats" : "Choisir un mandat"} {labels.mandate}
            </Button>
            {opened && mandates.state === "loading" ? (
                <p role="status">Lecture des mandats…</p>
            ) : null}
            {opened && mandates.state === "error" ? (
                <p role="alert">Impossible de lire les mandats {labels.mandate}.</p>
            ) : null}
            {mandates.state === "ready" && profile && available.length === 0 ? (
                <p>Aucun mandat {labels.mandate} disponible pour cette version.</p>
            ) : null}
            {opened && mandates.state === "ready" ? (
                <div className="provider-mandate-choice">
                    <label htmlFor={fieldId}>Mandat {labels.mandate}</label>
                    <select
                        id={fieldId}
                        className="control-input"
                        value={selected?.reference ?? ""}
                        disabled={
                            available.length === 0 || probe.checking || probe.needsRead || canRetry
                        }
                        onChange={(event) => {
                            setReference(event.currentTarget.value);
                        }}
                    >
                        <option value="">Sélectionner un mandat</option>
                        {available.map((mandate) => (
                            <option key={mandate.reference} value={mandate.reference}>
                                {mandate.principal} · {mandate.reference}
                            </option>
                        ))}
                    </select>
                </div>
            ) : null}
            {opened ? (
                <Button
                    className="control-button"
                    isDisabled={
                        !probe.needsRead && !canRetry && selected === undefined && !probe.checking
                    }
                    isPending={probe.checking}
                    onPress={() => {
                        void probe.run((detail) =>
                            selected === undefined
                                ? null
                                : profileProbeCommand(scope, detail, {
                                      phase,
                                      mandateRef: selected.reference,
                                      timeoutMs: Math.min(selected.timeoutMs, 60_000),
                                      locality: selected.locality,
                                  }),
                        );
                    }}
                >
                    {probe.needsRead ? labels.readResult : canRetry ? labels.resume : labels.start}
                </Button>
            ) : null}
            {probe.state === "submitting" ? <p role="status">{labels.pending}</p> : null}
            {canRetry ? <p role="alert">{labels.unconfirmed}</p> : null}
            {probe.state === "refused" ? <p role="alert">{labels.refused}</p> : null}
            {probe.needsRead && !probe.checking ? <p role="alert">{labels.missing}</p> : null}
            {probe.observed ? <p role="status">{labels.observed}</p> : null}
        </div>
    );
}

function SelectedProfile({
    profile,
    scope,
    transport,
}: {
    profile: ProviderProfileRow;
    scope: string | null;
    transport: TransportAdapter;
}) {
    const [attempt, setAttempt] = useState(0);
    const request = useMemo<GuiQueryInput>(
        () => ({
            project_id: scope,
            payload: { profile_id: profile.id, profile_version: profile.version },
        }),
        [profile.id, profile.version, scope],
    );
    const detail = useSettingsRead(
        transport,
        "GetProviderProfile",
        request,
        profileDetail,
        attempt,
    );
    const read: Reading<ProfileDetail> =
        detail.state === "ready" &&
        (detail.data.id !== profile.id ||
            detail.data.version !== profile.version ||
            detail.data.reference !== profile.reference)
            ? { state: "error" }
            : detail;
    const refresh = () => {
        setAttempt((value) => value + 1);
    };
    const verification = useProfileProbe(transport, "validate_config", read, refresh);
    const { needsRead, checking, observed } = verification;

    const actionLabel = needsRead
        ? "Relire le résultat"
        : verification.state === "retryable"
          ? "Reprendre la vérification"
          : "Vérifier la configuration";

    return (
        <section className="provider-detail" aria-labelledby="provider-detail-title">
            <h3 id="provider-detail-title">{profile.model}</h3>
            {read.state === "loading" ? <p role="status">Lecture du détail…</p> : null}
            {read.state === "error" && !needsRead ? (
                <p role="alert">Impossible de lire cette version.</p>
            ) : null}
            {read.state === "ready" ? (
                <dl className="provider-detail-fields">
                    <dt>Version configurée</dt>
                    <dd>{read.data.version}</dd>
                    <dt>Référence de version</dt>
                    <dd className="mono">{read.data.reference}</dd>
                    <dt>Qualification</dt>
                    <dd>{qualificationLabel(read.data.qualification)}</dd>
                </dl>
            ) : null}
            <div className="provider-validation">
                <Button
                    className="control-button"
                    isDisabled={!needsRead && read.state !== "ready"}
                    isPending={checking}
                    onPress={() => {
                        void verification.run((detail) =>
                            profileProbeCommand(scope, detail, {
                                phase: "validate_config",
                                mandateRef: detail.configurationReceiptId,
                                timeoutMs: 1000,
                                locality: "local_only",
                            }),
                        );
                    }}
                >
                    {actionLabel}
                </Button>
                {verification.state === "submitting" ? (
                    <p role="status">Vérification en cours…</p>
                ) : null}
                {verification.state === "retryable" ? (
                    <p role="alert">Vérification non confirmée.</p>
                ) : null}
                {verification.state === "refused" ? (
                    <p role="alert">Vérification refusée.</p>
                ) : null}
                {needsRead && !checking ? (
                    <p role="alert">Résultat de cette vérification indisponible.</p>
                ) : null}
                {observed ? <p role="status">Vérification enregistrée.</p> : null}
            </div>
            <TransportProbeControls
                phase="connect"
                scope={scope}
                transport={transport}
                read={read}
                refresh={refresh}
            />
            {profile.provider === "ollama" ? (
                <TransportProbeControls
                    phase="inventory"
                    scope={scope}
                    transport={transport}
                    read={read}
                    refresh={refresh}
                />
            ) : null}
            {read.state === "ready" ? (
                <>
                    <h4>Observations enregistrées</h4>
                    {read.data.observations.length === 0 ? (
                        <p>Aucune observation enregistrée pour cette version.</p>
                    ) : (
                        <ul className="provider-observations">
                            {read.data.observations.map(
                                ({ id, phase, status, observedAt, models }) => (
                                    <li key={id}>
                                        <span>
                                            {models === null
                                                ? observationLabel(phase, status)
                                                : "Inventaire observé"}
                                        </span>
                                        <time dateTime={observedAt}>
                                            {DATE_FORMAT.format(new Date(observedAt))}
                                        </time>
                                        {models === null ? null : (
                                            <InventoryModels models={models} />
                                        )}
                                    </li>
                                ),
                            )}
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
    const [creating, setCreating] = useState(false);
    const listRequest = useMemo(
        () => ({ project_id: scope, payload: { limit: 50, after_profile_id: cursor } }),
        [scope, cursor],
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

    function moveToPage(after: string | null) {
        setCursor(after);
        setSelected(null);
    }

    function reload() {
        moveToPage(null);
        setAttempt((value) => value + 1);
    }

    const showCurrentScope =
        scope !== null &&
        (projects.state !== "ready" || !projects.data.some(({ id }) => id === scope));

    return (
        <section className="provider-settings" aria-labelledby="provider-settings-title">
            <div className="provider-settings-header">
                <h2 id="provider-settings-title">Profils de modèles</h2>
                <div className="provider-settings-actions">
                    <Button
                        className="control-button"
                        aria-expanded={creating}
                        onPress={() => {
                            setCreating((open) => !open);
                        }}
                    >
                        {creating ? "Fermer l’ajout" : "Ajouter un profil local"}
                    </Button>
                    <Button className="control-button" onPress={reload}>
                        Actualiser les profils
                    </Button>
                </div>
            </div>
            <div className="provider-scope">
                <label htmlFor="provider-scope">Portée</label>
                <select
                    id="provider-scope"
                    value={scope ?? ""}
                    className="control-input"
                    onChange={(event) => {
                        setScope(event.currentTarget.value || null);
                        setCreating(false);
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

            {creating ? (
                <LocalProviderProfileForm transport={transport} scope={scope} onSaved={reload} />
            ) : null}

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
                        key={selected.reference}
                        profile={selected}
                        scope={scope}
                        transport={transport}
                    />
                ) : null}
            </div>
        </section>
    );
}
