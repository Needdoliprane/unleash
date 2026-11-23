import { type FormEvent, useCallback, useId, useMemo, useState } from "react";
import { Button } from "react-aria-components";
import { number, PROJECTS_REQUEST, projectChoices, record, text } from "../ipc/projections";
import type { GuiQueryInput, TransportAdapter } from "../ipc/types";
import { useIpcRead } from "../ipc/useIpcRead";

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
    };
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

    function submit(event: FormEvent<HTMLFormElement>) {
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
                        onPress={() => setProjectsAttempt((attempt) => attempt + 1)}
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
                {target !== null && read.state === "ready" ? (
                    <section className="workflow-run-detail" aria-labelledby={`${formId}-detail`}>
                        <h3 id={`${formId}-detail`}>Détail du run</h3>
                        <dl>
                            <dt>Identifiant</dt>
                            <dd>
                                <code>{target.runId}</code>
                            </dd>
                            <dt>État enregistré</dt>
                            <dd>{read.data.state}</dd>
                            <dt>Version engagée</dt>
                            <dd>
                                <code>{read.data.workflowVersion}</code>
                            </dd>
                            <dt>Révision du run</dt>
                            <dd>{read.data.revision}</dd>
                            <dt>Usage</dt>
                            <dd>{read.data.purpose}</dd>
                            <dt>Mission</dt>
                            <dd>
                                <code>{read.data.missionId}</code>
                            </dd>
                            <dt>Empreinte de la définition</dt>
                            <dd>
                                <code>{read.data.definitionHash}</code>
                            </dd>
                            <dt>Empreinte du plan effectif</dt>
                            <dd>
                                <code>{read.data.bundleHash}</code>
                            </dd>
                        </dl>
                    </section>
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
