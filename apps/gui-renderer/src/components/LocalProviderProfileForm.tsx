import { useEffect, useId, useRef, useState, type SubmitEvent } from "react";
import { Button } from "react-aria-components";
import { profileDetail } from "../ipc/providerProfileViews";
import type { CommandReceipt, GuiCommandInput, TransportAdapter } from "../ipc/types";
import { newUuid } from "../ipc/uuid";

const providers = [
    { kind: "ollama", name: "Ollama", dialect: "ollama_chat", endpoint: "http://127.0.0.1:11434" },
    {
        kind: "lmstudio",
        name: "LM Studio",
        dialect: "openai_chat_completions",
        endpoint: "http://127.0.0.1:1234/v1",
    },
    {
        kind: "llama_cpp",
        name: "llama.cpp",
        dialect: "openai_chat_completions",
        endpoint: "http://127.0.0.1:8080/v1",
    },
] as const;

type LocalProvider = (typeof providers)[number];
type Submission =
    | { state: "editing"; error?: string }
    | { state: "pending" }
    | { state: "unconfirmed"; request: GuiCommandInput; receiptId: string | null }
    | { state: "retryable"; request: GuiCommandInput }
    | { state: "stale" }
    | { state: "saved"; reference: string };

function isEndpointWithoutCredentials(value: string) {
    try {
        const url = new URL(value);
        return (
            (url.protocol === "http:" || url.protocol === "https:") &&
            url.username === "" &&
            url.password === "" &&
            !/^https?:\/\/[^/]*@/iu.test(value) &&
            !/[?#]/u.test(value)
        );
    } catch {
        return false;
    }
}

export function LocalProviderProfileForm({
    transport,
    scope,
    onSaved,
}: {
    transport: TransportAdapter;
    scope: string | null;
    onSaved: () => void;
}) {
    const id = useId();
    const [provider, setProvider] = useState<LocalProvider>(providers[0]);
    const [endpoint, setEndpoint] = useState<string>(providers[0].endpoint);
    const [model, setModel] = useState("");
    const [submission, setSubmission] = useState<Submission>({ state: "editing" });
    const mounted = useRef(false);
    const inFlight = useRef(false);
    const feedback = useRef<HTMLParagraphElement>(null);

    useEffect(() => {
        mounted.current = true;
        return () => {
            mounted.current = false;
        };
    }, []);
    useEffect(() => {
        feedback.current?.focus();
    }, [submission]);

    async function readVersion(request: GuiCommandInput, receiptId: string | null) {
        let reference: string | null = null;
        try {
            const { result } = await transport.executeQuery("GetProviderProfile", {
                project_id: request.project_id,
                payload: { profile_id: request.aggregate_id, profile_version: 1 },
            });
            const detail = profileDetail(result);
            if (
                detail.id === request.aggregate_id &&
                detail.version === 1 &&
                (receiptId === null || detail.configurationReceiptId === receiptId)
            ) {
                reference = detail.reference;
            }
        } catch {
            // The exact version remains unknown until the authority can be read.
        }
        if (!mounted.current) return;
        if (reference === null) {
            setSubmission({ state: "unconfirmed", request, receiptId });
        } else {
            setSubmission({ state: "saved", reference });
            onSaved();
        }
    }

    async function send(request: GuiCommandInput) {
        let receipt: CommandReceipt;
        try {
            receipt = await transport.submitCommand("SetProviderProfile", request);
        } catch (error) {
            if (!mounted.current) return;
            const failure = typeof error === "object" && error !== null ? error : {};
            const reason = "reason_code" in failure ? failure.reason_code : null;
            const code = "code" in failure ? failure.code : null;
            const retryClass = "retry_class" in failure ? failure.retry_class : null;
            if (code === "StaleFence" || reason === "STALE_AUTHORITY_GENERATION") {
                setSubmission({ state: "stale" });
            } else if (retryClass === "after_backoff") {
                setSubmission({ state: "retryable", request });
            } else if (
                retryClass === "never" &&
                (code === "InvalidArgument" ||
                    code === "ScopeDenied" ||
                    code === "CapabilityDenied")
            ) {
                setSubmission({ state: "editing", error: "Enregistrement refusé." });
            } else {
                await readVersion(request, null);
            }
            return;
        }
        if (!mounted.current) return;
        const matches =
            receipt.command_id === request.command_id &&
            receipt.aggregate_id === request.aggregate_id;
        if (matches && (receipt.state === "blocked" || receipt.state === "rejected")) {
            setSubmission({ state: "editing", error: "Enregistrement refusé." });
            return;
        }
        const accepted =
            matches &&
            receipt.new_revision === 1 &&
            (receipt.state === "accepted" || receipt.state === "completed");
        await readVersion(request, accepted ? receipt.receipt_id : null);
    }

    function perform(action: () => Promise<void>) {
        if (inFlight.current) {
            return;
        }
        inFlight.current = true;
        setSubmission({ state: "pending" });
        void action().finally(() => {
            inFlight.current = false;
        });
    }

    function submit(event: SubmitEvent<HTMLFormElement>) {
        event.preventDefault();
        if (submission.state !== "editing") {
            return;
        }
        if (!model.trim() || !isEndpointWithoutCredentials(endpoint.trim())) {
            setSubmission({
                state: "editing",
                error: "Renseignez un modèle et une adresse HTTP ou HTTPS sans identifiants ni paramètres.",
            });
            return;
        }
        const request: GuiCommandInput = {
            command_id: newUuid(),
            aggregate_id: newUuid(),
            project_id: scope,
            expected_revision: 0,
            payload: {
                provider_kind: provider.kind,
                dialect: { kind: provider.dialect, version: 1 },
                endpoint: endpoint.trim(),
                model: model.trim(),
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
        };
        perform(() => send(request));
    }

    return (
        <form className="provider-create" aria-labelledby={`${id}-title`} onSubmit={submit}>
            <h3 id={`${id}-title`}>Nouveau profil local</h3>
            <fieldset className="provider-create-fields" disabled={submission.state !== "editing"}>
                <label>
                    Fournisseur
                    <select
                        className="control-input"
                        value={provider.kind}
                        onChange={(event) => {
                            const next = providers.find(
                                ({ kind }) => kind === event.currentTarget.value,
                            );
                            if (next) {
                                setEndpoint((previous) =>
                                    previous === provider.endpoint ? next.endpoint : previous,
                                );
                                setProvider(next);
                            }
                        }}
                    >
                        {providers.map(({ kind, name }) => (
                            <option key={kind} value={kind}>
                                {name}
                            </option>
                        ))}
                    </select>
                </label>
                <label>
                    Adresse du serveur
                    <input
                        className="control-input"
                        type="url"
                        autoComplete="off"
                        aria-describedby={`${id}-hint`}
                        value={endpoint}
                        required
                        onChange={(event) => {
                            setEndpoint(event.currentTarget.value);
                        }}
                    />
                </label>
                <label>
                    Modèle
                    <input
                        className="control-input"
                        value={model}
                        required
                        onChange={(event) => {
                            setModel(event.currentTarget.value);
                        }}
                    />
                </label>
            </fieldset>
            <p className="provider-create-note" id={`${id}-hint`}>
                Serveur local sans authentification.
            </p>
            {submission.state === "editing" || submission.state === "pending" ? (
                <div className="provider-create-actions">
                    <Button
                        className="control-button control-button-primary"
                        type="submit"
                        isPending={submission.state === "pending"}
                    >
                        Enregistrer le profil
                    </Button>
                    {submission.state === "editing" && submission.error ? (
                        <p ref={feedback} role="alert" tabIndex={-1}>
                            {submission.error}
                        </p>
                    ) : null}
                    {submission.state === "pending" ? (
                        <p role="status">Enregistrement en cours…</p>
                    ) : null}
                </div>
            ) : null}
            {submission.state === "saved" ? (
                <div>
                    <p ref={feedback} role="status" tabIndex={-1}>
                        Profil enregistré.
                    </p>
                    <p className="mono">{submission.reference}</p>
                </div>
            ) : null}
            {submission.state === "stale" ? (
                <p ref={feedback} role="alert" tabIndex={-1}>
                    Le daemon a changé de lignée. Vérifiez la portée et les profils avant de
                    reprendre.
                </p>
            ) : null}
            {submission.state === "unconfirmed" ? (
                <div className="provider-create-actions">
                    <p ref={feedback} role="alert" tabIndex={-1}>
                        Résultat de l’enregistrement indisponible.
                    </p>
                    <Button
                        className="control-button"
                        onPress={() => {
                            perform(() => readVersion(submission.request, submission.receiptId));
                        }}
                    >
                        Relire le profil
                    </Button>
                </div>
            ) : null}
            {submission.state === "retryable" ? (
                <div className="provider-create-actions">
                    <p ref={feedback} role="alert" tabIndex={-1}>
                        Enregistrement temporairement indisponible.
                    </p>
                    <Button
                        className="control-button"
                        onPress={() => {
                            perform(() => send(submission.request));
                        }}
                    >
                        Reprendre l’enregistrement
                    </Button>
                </div>
            ) : null}
        </form>
    );
}
