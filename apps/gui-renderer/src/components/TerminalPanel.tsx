import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { Button, Input, Label, TextField } from "react-aria-components";
import type { CommandReceipt, TerminalProjection, TransportAdapter } from "../ipc/types";

type TerminalPanelProps = {
  terminal: TerminalProjection;
  transport: TransportAdapter;
};

export function TerminalPanel({ terminal, transport }: TerminalPanelProps) {
  const xtermHostRef = useRef<HTMLDivElement | null>(null);
  const terminalInstanceRef = useRef<Terminal | null>(null);
  const [commandInput, setCommandInput] = useState("");
  const [lines, setLines] = useState<readonly string[]>(() => terminal.initialLines);
  const [lastReceipt, setLastReceipt] = useState<CommandReceipt | null>(null);
  const [statusMessage, setStatusMessage] = useState<string>("Lecture bornée active");

  useEffect(() => {
    const host = xtermHostRef.current;
    if (!host) {
      return;
    }

    let instance: Terminal | null = null;
    try {
      instance = new Terminal({
        convertEol: true,
        disableStdin: true,
        scrollback: 2000,
        allowProposedApi: false,
        // Neutralise explicitement les liens OSC 8 (ADR-03).
        linkHandler: {
          activate: () => {},
        },
        rows: 10,
        cols: 80,
      });
      instance.open(host);
      for (const line of terminal.initialLines) {
        instance.writeln(line);
      }
      terminalInstanceRef.current = instance;
    } catch {
      // Environnement headless sans Canvas 2D (ex: jsdom) : le miroir texte accessible prend le relais.
      terminalInstanceRef.current = null;
    }

    return () => {
      terminalInstanceRef.current = null;
      instance?.dispose();
    };
  }, [terminal.initialLines]);

  const handleSendCommand = async (): Promise<void> => {
    const trimmed = commandInput.trim();
    if (trimmed.length === 0) {
      return;
    }
    const receipt = await transport.submitCommand("SendTerminalInput", {
      project_id: "00000000-0000-4000-8000-000000000100",
      aggregate_id: "00000000-0000-4000-8000-000000000777",
      expected_revision: terminal.fence,
      command_id: `cmd-pty-${String(Date.now())}`,
      payload: {
        process_run_uuid: terminal.processRunUuid,
        lease_id: terminal.leaseId,
        fence: terminal.fence,
        input: `${trimmed}\n`,
      },
    });
    setLastReceipt(receipt);
    const echoLine = `> [SendTerminalInput lease=${terminal.leaseId ?? "aucun"} fence=${String(terminal.fence)}] ${trimmed}`;
    terminalInstanceRef.current?.writeln(echoLine);
    setLines((prev) => [...prev.slice(-1999), echoLine]);
    setStatusMessage(`Reçu ${receipt.state} (${receipt.receipt_id})`);
    setCommandInput("");
  };

  return (
    <section className="panel-card" aria-labelledby="terminal-panel-title">
      <div className="panel-header">
        <div>
          <h2 id="terminal-panel-title" className="panel-title">
            Inspecteur Terminal PTY (xterm.js borné)
          </h2>
          <p className="mono" style={{ margin: "4px 0 0 0" }}>
            process_run_uuid={terminal.processRunUuid} · worktree={terminal.worktree} · lease=
            {terminal.leaseId ?? "non acquis"} (fence {terminal.fence})
          </p>
        </div>
        <div className="header-meta">
          <span className="badge badge-success">OSC 8 / OSC 52 désactivés</span>
          <span className="badge" role="status">
            {statusMessage}
          </span>
        </div>
      </div>

      <div
        ref={xtermHostRef}
        className="xterm-host"
        data-testid="xterm-container"
        aria-label="Vue émulateur terminal xterm.js"
      />

      <pre
        className="mono"
        aria-label="Journal textuel borné du terminal"
        style={{
          maxHeight: "110px",
          overflowY: "auto",
          margin: "8px 0",
          padding: "8px",
          borderRadius: "8px",
          backgroundColor: "var(--color-surface-subtle)",
        }}
      >
        {lines.join("\n")}
      </pre>

      <form
        style={{ display: "flex", gap: "8px", alignItems: "end", flexWrap: "wrap" }}
        onSubmit={(event) => {
          event.preventDefault();
          void handleSendCommand();
        }}
      >
        <TextField
          style={{ flex: "1 1 260px", display: "flex", flexDirection: "column", gap: "4px" }}
          value={commandInput}
          onChange={setCommandInput}
        >
          <Label className="mono">Commande typée (SendTerminalInput)</Label>
          <Input
            className="control-input mono"
            placeholder="Saisir une commande sous lease actif..."
            data-testid="terminal-command-input"
          />
        </TextField>
        <Button type="submit" className="control-button control-button-primary">
          Envoyer au PTY
        </Button>
      </form>

      {lastReceipt ? (
        <p className="mono" style={{ marginTop: "8px", marginBottom: 0 }}>
          Dernier reçu : {lastReceipt.receipt_id} · état={lastReceipt.state} · rev=
          {lastReceipt.new_revision}
        </p>
      ) : null}
    </section>
  );
}
