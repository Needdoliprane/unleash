import type { GuiQueryInput } from "./types";

export function record(value: unknown): Record<string, unknown> {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
        throw new Error("Projection IPC illisible");
    }
    return value as Record<string, unknown>;
}

export function list(value: unknown): unknown[] {
    if (!Array.isArray(value)) {
        throw new Error("Liste IPC illisible");
    }
    return value;
}

export function text(value: unknown): string {
    if (typeof value !== "string" || value.length === 0) {
        throw new Error("Libellé ou référence IPC illisible");
    }
    return value;
}

export function number(value: unknown): number {
    if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
        throw new Error("Valeur numérique IPC illisible");
    }
    return value;
}

export const PROJECTS_REQUEST: GuiQueryInput = { project_id: null, payload: {} };

export function projectChoices(value: unknown) {
    return list(value).map((item) => {
        const fields = record(item);
        return { id: text(fields.project_id), name: text(fields.name) };
    });
}
