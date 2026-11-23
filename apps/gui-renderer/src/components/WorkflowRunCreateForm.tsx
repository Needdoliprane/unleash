import { type SubmitEvent, useCallback, useId, useMemo, useRef, useState } from "react";
import { Button } from "react-aria-components";
import { record, text } from "../ipc/projections";
import type { GuiQueryInput, TransportAdapter } from "../ipc/types";
import { useIpcRead } from "../ipc/useIpcRead";
import { newUuid, UUID_PATTERN } from "../ipc/uuid";

const DIGEST_PATTERN = "sha256:[0-9a-f]{64}";
const MAX_VERSION = 4_294_967_295;

type RunReference = { runId: string; missionId: string; definitionHash: string };
type Submission =
    | { state: "refused"; message: string }
    | { state: "sending" | "observing"; target: RunReference };

const referenceFields = [
    { name: "workflow", label: "Identifiant du workflow" },
    { name: "version", label: "Numéro de version" },
    { name: "definition", label: "Empreinte de la définition", pattern: DIGEST_PATTERN },
    { name: "mission", label: "Identifiant de la mission", pattern: UUID_PATTERN },
    { name: "scope", label: "Empreinte du manifeste de portée", pattern: DIGEST_PATTERN },
    {
        name: "predecessor",
        label: "Run prédécesseur (facultatif)",
        pattern: UUID_PATTERN,
        optional: true,
    },
    { name: "reason", label: "Motif de sélection" },
];

function refusalMessage(error: unknown) {
    if (typeof error !== "object" || error === null) return null;
    const code = "code" in error ? error.code : null;
    const reason = "reason_code" in error ? error.reason_code : null;
    if (code === "ResourceBusy" && reason === "PREDECESSOR_NOT_TERMINAL") {
        return "Le run prédécesseur doit être terminé.";
    }
    if (code === "InvalidArgument" && reason === "SUCCESSOR_EXISTS") {
        return "Ce run prédécesseur a déjà un successeur.";
    }
    if (
        "retry_class" in error &&
        error.retry_class === "never" &&
        (code === "InvalidArgument" ||
            code === "CapabilityDenied" ||
            code === "ScopeDenied" ||
            code === "NotFoundOrHidden")
    ) {
        return "Vérifiez la mission, la portée et la version publiée.";
    }
    return null;
}

export function WorkflowRunCreateForm({
    projectId,
    transport,
    onConsult,
}: {
    projectId: string;
    transport: TransportAdapter;
    onConsult: (runId: string) => void;
}) {
    const id = useId();
    const form = useRef<HTMLFormElement>(null);
    const summary = useRef<HTMLElement>(null);
    const feedback = useRef<HTMLDivElement>(null);
    const [submissions, setSubmissions] = useState<ReadonlyMap<string, Submission>>(
        () => new Map(),
    );
    const sending = useRef(new Set<string>());
    const [attempt, setAttempt] = useState(0);
    const submission = submissions.get(projectId);
    const canSubmit = submission === undefined || submission.state === "refused";
    const target = submission?.state === "observing" ? submission.target : null;
    const request = useMemo<GuiQueryInput | null>(
        () =>
            target === null ? null : { project_id: projectId, payload: { run_id: target.runId } },
        [projectId, target],
    );
    const decode = useCallback(
        (value: unknown) => {
            const fields = record(value);
            if (
                target === null ||
                fields.project_id !== projectId ||
                fields.run_id !== target.runId ||
                fields.mission_id !== target.missionId ||
                fields.definition_hash !== target.definitionHash ||
                fields.purpose !== "operational"
            ) {
                throw new Error("Le run relu ne correspond pas à la préparation.");
            }
            return text(fields.workflow_version_id);
        },
        [projectId, target],
    );
    const read = useIpcRead(transport, "GetRun", request, decode, attempt);

    function recordSubmission(next: Submission | null) {
        setSubmissions((previous) => {
            const updated = new Map(previous);
            if (next === null) updated.delete(projectId);
            else updated.set(projectId, next);
            return updated;
        });
    }

    async function prepare(event: SubmitEvent<HTMLFormElement>) {
        event.preventDefault();
        if (!projectId || !canSubmit || sending.current.has(projectId)) return;
        const values = new FormData(event.currentTarget);
        const field = (name: string) => {
            const value = values.get(name);
            return typeof value === "string" ? value.trim() : "";
        };
        const workflow = field("workflow");
        const reason = field("reason");
        if (!workflow || !reason) {
            recordSubmission({
                state: "refused",
                message: "Renseignez l’identifiant du workflow et le motif.",
            });
            return;
        }
        const proposed: RunReference = {
            runId: newUuid(),
            missionId: field("mission").toLowerCase(),
            definitionHash: field("definition"),
        };
        const commandId = newUuid();
        const predecessor = field("predecessor").toLowerCase();
        sending.current.add(projectId);
        feedback.current?.focus();
        recordSubmission({ state: "sending", target: proposed });
        let refusal: string | null = null;
        try {
            const receipt = await transport.submitCommand("CreateRun", {
                command_id: commandId,
                project_id: projectId,
                aggregate_id: proposed.runId,
                expected_revision: 0,
                payload: {
                    mission_id: proposed.missionId,
                    source: {
                        kind: "workflow_version",
                        id: workflow,
                        version: Number(field("version")),
                        digest: proposed.definitionHash,
                    },
                    purpose: "operational",
                    selection_reason: reason,
                    scope_ref: field("scope"),
                    ...(predecessor ? { predecessor_run_id: predecessor } : {}),
                },
            });
            if (
                receipt.command_id === commandId &&
                receipt.aggregate_id === proposed.runId &&
                receipt.state === "rejected"
            ) {
                refusal = "Vérifiez les références saisies.";
            }
        } catch (error) {
            refusal = refusalMessage(error);
        } finally {
            sending.current.delete(projectId);
        }
        recordSubmission(
            refusal === null
                ? { state: "observing", target: proposed }
                : { state: "refused", message: refusal },
        );
    }

    if (!projectId) return null;
    return (
        <details className="workflow-run-create">
            <summary ref={summary}>Préparer un run</summary>
            <div
                ref={feedback}
                tabIndex={-1}
                role="group"
                aria-label="Suivi de la préparation"
                aria-live="polite"
            >
                {submission?.state === "refused" ? (
                    <p role="alert">Préparation refusée. {submission.message}</p>
                ) : null}
                {submission?.state === "sending" ||
                (target !== null && read.state === "loading") ? (
                    <p>Vérification de la préparation…</p>
                ) : null}
                {target !== null && read.state === "error" ? (
                    <div>
                        <p role="alert">
                            Préparation non confirmée. Identifiant du run{" "}
                            <code>{target.runId}</code>
                        </p>
                        <Button
                            className="control-button"
                            onPress={() => {
                                feedback.current?.focus();
                                setAttempt((value) => value + 1);
                            }}
                        >
                            Relire le run demandé
                        </Button>
                    </div>
                ) : null}
                {target !== null && read.state === "ready" ? (
                    <section className="workflow-run-detail" aria-labelledby={id}>
                        <h3 id={id}>Run créé</h3>
                        <dl>
                            <dt>Identifiant</dt>
                            <dd>
                                <code>{target.runId}</code>
                            </dd>
                            <dt>Version engagée</dt>
                            <dd>
                                <code>{read.data}</code>
                            </dd>
                        </dl>
                        <div className="workflow-run-create-actions">
                            <Button
                                className="control-button"
                                onPress={() => {
                                    onConsult(target.runId);
                                }}
                            >
                                Consulter le run créé
                            </Button>
                            <Button
                                className="control-button"
                                onPress={() => {
                                    form.current?.reset();
                                    summary.current?.focus();
                                    recordSubmission(null);
                                }}
                            >
                                Préparer un autre run
                            </Button>
                        </div>
                    </section>
                ) : null}
            </div>
            <form
                ref={form}
                hidden={!canSubmit}
                onSubmit={(event) => {
                    void prepare(event);
                }}
            >
                <p>Renseignez une version publiée et le manifeste retenu pour la mission.</p>
                <fieldset key={projectId} className="workflow-run-form workflow-run-create-fields">
                    <legend>Références du run</legend>
                    {referenceFields.map(({ name, label, pattern, optional }) => (
                        <label className="workflow-run-field" key={name}>
                            {label}
                            <input
                                className="control-input"
                                name={name}
                                {...(name === "version"
                                    ? { type: "number", min: 1, max: MAX_VERSION, step: 1 }
                                    : {
                                          pattern,
                                          autoComplete: "off",
                                          autoCapitalize: "none",
                                          spellCheck: false,
                                      })}
                                required={!optional}
                            />
                        </label>
                    ))}
                    <Button className="control-button control-button-primary" type="submit">
                        Préparer le run
                    </Button>
                </fieldset>
            </form>
        </details>
    );
}
