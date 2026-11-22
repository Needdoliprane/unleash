function record(value: unknown): Record<string, unknown> {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
        throw new Error("Projection IPC illisible");
    }
    return value as Record<string, unknown>;
}

function list(value: unknown): unknown[] {
    if (!Array.isArray(value)) {
        throw new Error("Liste IPC illisible");
    }
    return value;
}

function text(value: unknown): string {
    if (typeof value !== "string" || value.length === 0) {
        throw new Error("Libellé ou référence IPC illisible");
    }
    return value;
}

function number(value: unknown): number {
    if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
        throw new Error("Valeur numérique IPC illisible");
    }
    return value;
}

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

export function projectChoices(value: unknown) {
    return list(value).map((item) => {
        const fields = record(item);
        return { id: text(fields.project_id), name: text(fields.name) };
    });
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

function inventoryModelNames(value: unknown): string[] {
    return list(record(value).models).map((model) => {
        const { name } = record(model);
        if (typeof name !== "string") {
            throw new Error("Nom de modèle IPC illisible");
        }
        return name;
    });
}

export function profileDetail(value: unknown) {
    const fields = record(value);
    return {
        ...profileIdentity(fields),
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
                modelNames:
                    phase === "inventory" && status === "inventory_observed"
                        ? inventoryModelNames(observation.inventory)
                        : null,
            };
        }),
    };
}
