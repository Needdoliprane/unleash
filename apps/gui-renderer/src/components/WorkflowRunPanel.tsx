import { type SubmitEvent, useCallback, useId, useMemo, useRef, useState } from "react";
import { Button } from "react-aria-components";
import { number, PROJECTS_REQUEST, projectChoices, record, text } from "../ipc/projections";
import type { GuiCommandInput, GuiQueryInput, TransportAdapter } from "../ipc/types";
import { type Reading, useIpcRead } from "../ipc/useIpcRead";
import { newUuid } from "../ipc/uuid";

const UUID_PATTERN = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
const RUN_STATES = new Map([
    ["prepared", "Préparé"],
    ["running", "En cours"],
    ["paused", "En pause"],
    ["blocked", "Bloqué"],
    ["cancelling", "Annulation en cours"],
    ["cancelled", "Annulé"],
    ["succeeded", "Réussi"],
    ["failed", "Échec"],
]);
const RUN_PURPOSES = new Map([
    ["operational", "Opérationnel"],
    ["evaluation", "Évaluation"],
]);

type RunTarget = { projectId: string; runId: string };

function runDetails(value: unknown, target: RunTarget | null) {
    const fields = record(value);
    if (
        target === null ||
        fields.project_id !== target.projectId ||
        fields.run_id !== target.runId
    ) {
        throw new Error("Projection du run hors de la cible demandée");
    }
    const state = text(fields.state);
    const purpose = text(fields.purpose);
    return {
        state: RUN_STATES.get(state) ?? state,
        purpose: RUN_PURPOSES.get(purpose) ?? purpose,
        revision: number(fields.revision),
        missionId: text(fields.mission_id),
        workflowVersion: text(fields.workflow_version_id),
        definitionHash: text(fields.definition_hash),
        bundleHash: text(fields.effective_bundle_hash),
        offersStart:
            state === "prepared" &&
            purpose === "operational" &&
            typeof fields.plan === "object" &&
            fields.plan !== null &&
            !Array.isArray(fields.plan),
    };
}

type StartFeedback =
    | { kind: "submitting" }
    | { kind: "observing"; previousRevision: number }
    | { kind: "retryable"; command: GuiCommandInput }
    | { kind: "conflict" | "refreshing" }
    | { kind: "refused"; explanation: string | null };

function RunDetail({
    headingId,
    read,
    target,
    refresh,
    transport,
}: {
    headingId: string;
    read: Reading<ReturnType<typeof runDetails>>;
    target: RunTarget;
    refresh: () => void;
    transport: TransportAdapter;
}) {
    const [feedback, setFeedback] = useState<StartFeedback | null>(null);
    const sending = useRef(false);
    const detail = read.state === "ready" ? read.data : null;
    const waiting =
        feedback?.kind === "observing" &&
        (detail === null || detail.revision <= feedback.previousRevision);
    const canStart = detail?.offersStart === true && !waiting && feedback?.kind !== "refused";
    const retry =
        feedback?.kind === "retryable" && feedback.command.expected_revision === detail?.revision
            ? feedback.command
            : null;
    const submitting = feedback?.kind === "submitting";

    async function start() {
        if (!canStart || sending.current) return;
        const command = retry ?? {
            command_id: newUuid(),
            project_id: target.projectId,
            aggregate_id: target.runId,
            expected_revision: detail.revision,
            payload: {},
        };
        sending.current = true;
        setFeedback({ kind: "submitting" });
        try {
            const receipt = await transport.submitCommand("StartRun", command);
            if (
                receipt.command_id === command.command_id &&
                receipt.aggregate_id === command.aggregate_id &&
                (receipt.state === "blocked" || receipt.state === "rejected")
            ) {
                setFeedback({ kind: "refused", explanation: null });
                return;
            }
        } catch (error) {
            if (typeof error === "object" && error !== null) {
                const code = "code" in error ? error.code : null;
                const retryClass = "retry_class" in error ? error.retry_class : null;
                if (code === "RevisionConflict") {
                    setFeedback({ kind: "conflict" });
                    refresh();
                    return;
                }
                if (retryClass === "after_backoff") {
                    setFeedback({ kind: "retryable", command });
                    return;
                }
                if (retryClass === "never" || retryClass === "after_human_decision") {
                    const explanation =
                        code === "BudgetExceeded"
                            ? "Budget d’activation épuisé."
                            : retryClass === "after_human_decision"
                              ? "Une décision est requise."
                              : null;
                    setFeedback({ kind: "refused", explanation });
                    return;
                }
            }
        } finally {
            sending.current = false;
        }
        setFeedback({ kind: "observing", previousRevision: detail.revision });
        refresh();
    }

    if (detail === null && feedback === null) return null;
    const startingAction = canStart || submitting;
    const label = waiting
        ? "Relire le résultat du démarrage"
        : startingAction
          ? retry === null
              ? "Démarrer le run"
              : "Reprendre le démarrage"
          : "Actualiser le run";

    return (
        <section className="workflow-run-detail" aria-labelledby={headingId}>
            <h3 id={headingId}>Détail du run</h3>
            {detail === null ? null : (
                <dl>
                    <dt>Identifiant</dt>
                    <dd>
                        <code>{target.runId}</code>
                    </dd>
                    <dt>État enregistré</dt>
                    <dd>{detail.state}</dd>
                    <dt>Version engagée</dt>
                    <dd>
                        <code>{detail.workflowVersion}</code>
                    </dd>
                    <dt>Révision du run</dt>
                    <dd>{detail.revision}</dd>
                    <dt>Usage</dt>
                    <dd>{detail.purpose}</dd>
                    <dt>Mission</dt>
                    <dd>
                        <code>{detail.missionId}</code>
                    </dd>
                    <dt>Empreinte de la définition</dt>
                    <dd>
                        <code>{detail.definitionHash}</code>
                    </dd>
                    <dt>Empreinte du plan effectif</dt>
                    <dd>
                        <code>{detail.bundleHash}</code>
                    </dd>
                </dl>
            )}
            {canStart || feedback !== null ? (
                <Button
                    className={
                        startingAction ? "control-button control-button-primary" : "control-button"
                    }
                    isPending={submitting || read.state === "loading"}
                    onPress={() => {
                        if (canStart) {
                            void start();
                        } else {
                            if (feedback?.kind === "refused") setFeedback({ kind: "refreshing" });
                            refresh();
                        }
                    }}
                >
                    {label}
                </Button>
            ) : null}
            {submitting ? <p role="status">Demande de démarrage en cours…</p> : null}
            {waiting && read.state !== "loading" ? (
                <p role="alert">Démarrage non confirmé.</p>
            ) : null}
            {feedback?.kind === "conflict" ? (
                <p role="alert">Les données du run ont changé. Vérifiez la version relue.</p>
            ) : null}
            {feedback?.kind === "refused" ? (
                <p role="alert">Démarrage refusé. {feedback.explanation}</p>
            ) : null}
            {retry !== null ? <p role="alert">Démarrage temporairement indisponible.</p> : null}
        </section>
    );
}

export function WorkflowRunPanel({ transport }: { transport: TransportAdapter }) {
    const formId = useId();
    const [projectId, setProjectId] = useState("");
    const [runId, setRunId] = useState("");
    const [target, setTarget] = useState<RunTarget | null>(null);
    const [projectsAttempt, setProjectsAttempt] = useState(0);
    const projects = useIpcRead(
        transport,
        "ListProjects",
        PROJECTS_REQUEST,
        projectChoices,
        projectsAttempt,
    );
    const request = useMemo<GuiQueryInput | null>(
        () =>
            target === null
                ? null
                : { project_id: target.projectId, payload: { run_id: target.runId } },
        [target],
    );
    const decode = useCallback((value: unknown) => runDetails(value, target), [target]);
    const read = useIpcRead(transport, "GetRun", request, decode);
    const readingRun = target !== null && read.state === "loading";
    const hasProjects = projects.state === "ready" && projects.data.length > 0;

    function submit(event: SubmitEvent<HTMLFormElement>) {
        event.preventDefault();
        if (projectId && runId && !readingRun) {
            setTarget({ projectId, runId: runId.toLowerCase() });
        }
    }

    return (
        <section className="workflow-run-panel" aria-labelledby={`${formId}-title`}>
            <h2 id={`${formId}-title`}>Exécutions de workflow</h2>
            {projects.state === "loading" ? <p role="status">Lecture des projets…</p> : null}
            {projects.state === "error" ? (
                <div>
                    <p role="alert">Impossible de lire les projets.</p>
                    <Button
                        className="control-button"
                        onPress={() => {
                            setProjectsAttempt((attempt) => attempt + 1);
                        }}
                    >
                        Relire les projets
                    </Button>
                </div>
            ) : null}
            {projects.state === "ready" && !hasProjects ? <p>Aucun projet accessible.</p> : null}
            {hasProjects ? (
                <form className="workflow-run-form" onSubmit={submit}>
                    <label className="workflow-run-field" htmlFor={`${formId}-project`}>
                        Projet
                        <select
                            id={`${formId}-project`}
                            className="control-input"
                            value={projectId}
                            required
                            onChange={(event) => {
                                setProjectId(event.currentTarget.value);
                                setRunId("");
                                setTarget(null);
                            }}
                        >
                            <option value="">Choisir un projet</option>
                            {projects.data.map(({ id, name }) => (
                                <option key={id} value={id}>
                                    {name} · {id}
                                </option>
                            ))}
                        </select>
                    </label>
                    <label className="workflow-run-field" htmlFor={`${formId}-run`}>
                        Identifiant du run
                        <input
                            id={`${formId}-run`}
                            className="control-input"
                            autoComplete="off"
                            autoCapitalize="none"
                            spellCheck={false}
                            pattern={UUID_PATTERN}
                            title="UUID au format 8-4-4-4-12"
                            required
                            value={runId}
                            onChange={(event) => {
                                setRunId(event.currentTarget.value.trim());
                                setTarget(null);
                            }}
                        />
                    </label>
                    <Button
                        className="control-button control-button-primary"
                        type="submit"
                        isPending={readingRun}
                    >
                        Consulter le run
                    </Button>
                </form>
            ) : null}
            <div aria-live="polite">
                {readingRun ? <p>Lecture du run…</p> : null}
                {target !== null ? (
                    <RunDetail
                        key={`${target.projectId}/${target.runId}`}
                        headingId={`${formId}-detail`}
                        read={read}
                        target={target}
                        transport={transport}
                        refresh={() => {
                            setTarget((current) =>
                                current?.projectId === target.projectId &&
                                current.runId === target.runId
                                    ? { ...current }
                                    : current,
                            );
                        }}
                    />
                ) : null}
            </div>
            {target !== null && read.state === "error" ? (
                <p role="alert">
                    {read.code === "NotFoundOrHidden" || read.code === "ScopeDenied"
                        ? "Run introuvable ou inaccessible dans ce projet."
                        : "Impossible de lire ce run."}
                </p>
            ) : null}
        </section>
    );
}
