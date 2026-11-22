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

export function profileDetail(value: unknown) {
    const fields = record(value);
    return {
        ...profileIdentity(fields),
        qualification: text(fields.qualification),
        observations: list(fields.observations).map((item) => {
            const observation = record(item);
            return {
                id: text(observation.probe_id),
                phase: text(observation.phase),
                status: text(observation.status),
                observedAt: new Date(number(observation.observed_at_ms)).toISOString(),
            };
        }),
    };
}
