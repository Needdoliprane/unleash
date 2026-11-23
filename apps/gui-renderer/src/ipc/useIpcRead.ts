import { useEffect, useState } from "react";
import type { GuiQueryInput, OperationName, TransportAdapter } from "./types";

export type Reading<T> =
    { state: "loading" } | { state: "error"; code?: string } | { state: "ready"; data: T };

export function useIpcRead<T>(
    transport: TransportAdapter,
    operation: OperationName,
    request: GuiQueryInput | null,
    read: (value: unknown) => T,
    attempt = 0,
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
            (error: unknown) => {
                if (
                    error &&
                    typeof error === "object" &&
                    "code" in error &&
                    typeof error.code === "string"
                ) {
                    finish({ state: "error", code: error.code });
                } else {
                    finish({ state: "error" });
                }
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
