import { describe, expect, it } from "vitest";
import type { GuiStreamNotification, OperationName } from "../ipc/types";
import {
  FORBIDDEN_GUI_OPERATIONS,
  createTauriTransportAdapter,
  isExposedGuiOperation,
  operationToTauriCommand,
} from "../ipc/tauriBridge";

describe("Contrat IPC généré et exclusion des opérations internes worker/handshake", () => {
  it("refuse Hello, RecordClaim, RecordObservation et RecordJudgment côté renderer", async () => {
    const adapter = createTauriTransportAdapter();
    const forbidden: OperationName[] = [
      "Hello",
      "RecordClaim",
      "RecordObservation",
      "RecordJudgment",
    ];

    expect(FORBIDDEN_GUI_OPERATIONS.size).toBe(4);
    for (const op of forbidden) {
      expect(isExposedGuiOperation(op)).toBe(false);
      await expect(
        adapter.submitCommand(op, {
          project_id: null,
          aggregate_id: "00000000-0000-0000-0000-000000000001",
          expected_revision: 0,
          command_id: "cmd-forbidden",
          payload: {},
        }),
      ).rejects.toThrow(/Opération interdite/);
    }

    expect(isExposedGuiOperation("GetOverview")).toBe(true);
    expect(operationToTauriCommand("GetOverview")).toBe("op_get_overview");
    expect(operationToTauriCommand("SendTerminalInput")).toBe("op_send_terminal_input");
  });

  it("expose subscribeEvents sur Channel Tauri ordonné sans dupliquer les DTOs", async () => {
    const adapter = createTauriTransportAdapter();
    const fallbackEvents: GuiStreamNotification[] = [];
    const fallbackSub = await adapter.subscribeEvents(
      { project_id: null, aggregate_id: null, batch_limit: 50 },
      (n) => fallbackEvents.push(n),
    );
    expect(fallbackEvents[0]?.kind).toBe("snapshot_loaded");
    fallbackSub.close();

    // Simule un hôte Tauri injectant __TAURI_INTERNALS__ et émettant des trames Channel désordonnées
    const callbacks = new Map<number, (payload: unknown) => void>();
    let unregisteredId: number | null = null;
    let capturedCmd = "";
    let capturedSerializedChannel = "";

    const win = window as unknown as { __TAURI_INTERNALS__?: Record<string, unknown> };
    win.__TAURI_INTERNALS__ = {
      transformCallback: (cb: (payload: unknown) => void): number => {
        callbacks.set(42, cb);
        return 42;
      },
      unregisterCallback: (id: number): void => {
        unregisteredId = id;
        callbacks.delete(id);
      },
      invoke: (cmd: string, args?: { channel?: unknown }) => {
        capturedCmd = cmd;
        capturedSerializedChannel = JSON.stringify(args?.channel);
        const cb = callbacks.get(42);
        // Émet index 1 avant index 0 : le pont doit réordonner par index
        cb?.({
          index: 1,
          message: {
            kind: "events",
            events: [],
            cursor: { stream_id: "00000000-0000-0000-0000-000000000001", sequence: 2 },
          },
        });
        cb?.({
          index: 0,
          message: {
            kind: "snapshot_loaded",
            watermark: { stream_id: "00000000-0000-0000-0000-000000000001", sequence: 1 },
            snapshot: { ok: true },
            resync_reason: null,
          },
        });
        return Promise.resolve({ stream_id: "00000000-0000-0000-0000-000000000001", sequence: 1 });
      },
    };

    try {
      const received: GuiStreamNotification[] = [];
      const sub = await adapter.subscribeEvents(
        { project_id: null, aggregate_id: null, batch_limit: 50 },
        (n) => received.push(n),
      );
      expect(capturedCmd).toBe("op_subscribe_events");
      expect(capturedSerializedChannel).toBe('"__CHANNEL__:42"');
      expect(sub.cursor.sequence).toBe(1);
      expect(received.map((n) => n.kind)).toEqual(["snapshot_loaded", "events"]);
      sub.close();
      expect(unregisteredId).toBe(42);
    } finally {
      delete win.__TAURI_INTERNALS__;
    }
  });
});
