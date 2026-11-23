import { list, number, record, text } from "./projections";

function profileIdentity(fields: Record<string, unknown>) {
    const version = number(fields.profile_version);
    if (version === 0) {
        throw new Error("Version de profil IPC illisible");
    }
    return {
        id: text(fields.profile_id),
        version,
        reference: text(fields.profile_version_ref),
    };
}

export function profilePage(value: unknown) {
    const fields = record(value);
    return {
        profiles: list(fields.profiles).map((item) => {
            const profile = record(item);
            return {
                ...profileIdentity(profile),
                model: text(profile.model),
                provider: text(profile.provider_kind),
                qualification: text(profile.qualification),
            };
        }),
        next: fields.next_after_profile_id === null ? null : text(fields.next_after_profile_id),
    };
}

export type ProviderProfileRow = ReturnType<typeof profilePage>["profiles"][number];

export type TransportProbePhase = "connect" | "inventory";

export function transportProbeMandates(value: unknown, phase: TransportProbePhase) {
    return list(record(value).provider_probe_mandates).flatMap((item) => {
        const fields = record(item);
        if (fields.phase !== phase || fields.status !== "active") return [];

        const locality = fields.locality;
        const timeoutMs = number(record(fields.limits).timeout_ms);
        if ((locality !== "local_only" && locality !== "approved_remote") || timeoutMs === 0) {
            throw new Error("Mandat de transport IPC illisible");
        }

        return [
            {
                reference: text(fields.mandate_ref),
                principal: text(fields.principal_id),
                scopeKind: text(fields.scope_kind),
                scopeId: text(fields.scope_id),
                profileId: text(fields.profile_id),
                profileVersion: number(fields.profile_version),
                configurationDigest: text(fields.configuration_digest),
                locality,
                timeoutMs,
            } as const,
        ];
    });
}

function observedText(value: unknown): string | null {
    if (value === null || value === undefined) return null;
    if (typeof value !== "string") {
        throw new Error("Métadonnée de modèle IPC illisible");
    }
    return value.trim() === "" ? null : value;
}

function inventoryModels(value: unknown) {
    return list(record(value).models).map((model) => {
        const { name, details } = record(model);
        if (typeof name !== "string") {
            throw new Error("Nom de modèle IPC illisible");
        }
        const metadata = details === null || details === undefined ? null : record(details);
        return {
            name,
            format: observedText(metadata?.format),
            quantization: observedText(metadata?.quantization_level),
        };
    });
}

export type InventoryModel = ReturnType<typeof inventoryModels>[number];

export function profileDetail(value: unknown) {
    const fields = record(value);
    return {
        ...profileIdentity(fields),
        configurationDigest: observedText(fields.configuration_digest),
        configurationReceiptId: text(fields.configuration_receipt_id),
        qualification: text(fields.qualification),
        observations: list(fields.observations).map((item) => {
            const observation = record(item);
            const phase = text(observation.phase);
            const status = text(observation.status);
            return {
                id: text(observation.probe_id),
                phase,
                status,
                resultRef: text(observation.result_ref),
                observedAt: new Date(number(observation.observed_at_ms)).toISOString(),
                models:
                    phase === "inventory" && status === "inventory_observed"
                        ? inventoryModels(observation.inventory)
                        : null,
            };
        }),
    };
}
