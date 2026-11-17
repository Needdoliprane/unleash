import { describe, expect, it } from "vitest";
import type { OperationName } from "../generated";
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
});
