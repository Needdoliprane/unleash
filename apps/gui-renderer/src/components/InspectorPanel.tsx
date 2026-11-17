import type { AgentNode, TimelineEventItem } from "../ipc/types";

type InspectorPanelProps = {
  selectedAgent: AgentNode | null;
  selectedEvent: TimelineEventItem | null;
};

export function InspectorPanel({ selectedAgent, selectedEvent }: InspectorPanelProps) {
  return (
    <aside className="inspector-card" aria-labelledby="inspector-title">
      <h2 id="inspector-title" className="panel-title">
        Inspecteur de détail
      </h2>
      <p style={{ color: "var(--color-text-secondary)", fontSize: "13px" }}>
        Ouvre la provenance et l&apos;état exact sans quitter la mission.
      </p>

      {selectedAgent ? (
        <div
          style={{
            padding: "12px",
            borderRadius: "8px",
            backgroundColor: "var(--color-surface-subtle)",
            marginBottom: "12px",
          }}
          data-testid="inspector-agent"
        >
          <h3 style={{ margin: "0 0 6px 0", fontSize: "14px" }}>Agent sélectionné</h3>
          <dl className="mono" style={{ margin: 0, display: "grid", gap: "4px" }}>
            <div>ID : {selectedAgent.id}</div>
            <div>Nom : {selectedAgent.name}</div>
            <div>Niveau : {selectedAgent.level}</div>
            <div>Rôle : {selectedAgent.role}</div>
            <div>Modèle effectif : {selectedAgent.model}</div>
            <div>État logique : {selectedAgent.logicalState}</div>
            <div>Invocation : {selectedAgent.invocationState}</div>
            {selectedAgent.blockedReason ? (
              <div className="badge badge-danger">Blocage : {selectedAgent.blockedReason}</div>
            ) : null}
          </dl>
        </div>
      ) : null}

      {selectedEvent ? (
        <div
          style={{
            padding: "12px",
            borderRadius: "8px",
            backgroundColor: "var(--color-surface-subtle)",
          }}
          data-testid="inspector-event"
        >
          <h3 style={{ margin: "0 0 6px 0", fontSize: "14px" }}>Événement sélectionné</h3>
          <dl className="mono" style={{ margin: 0, display: "grid", gap: "4px" }}>
            <div>Type : {selectedEvent.envelope.event_type}</div>
            <div>Séquence : {selectedEvent.envelope.sequence}</div>
            <div>Stream : {selectedEvent.envelope.stream_id}</div>
            <div>
              Causalité : {selectedEvent.envelope.causation.kind} (
              {selectedEvent.envelope.causation.id})
            </div>
            <div>Command ID : {selectedEvent.envelope.command_id ?? "aucun"}</div>
            <div>Sensibilité : {selectedEvent.envelope.sensitivity}</div>
            <div style={{ wordBreak: "break-all" }}>
              Hash : {selectedEvent.envelope.payload_hash}
            </div>
            <div>Détail : {selectedEvent.summary}</div>
          </dl>
        </div>
      ) : (
        <p className="mono">
          Sélectionnez un événement de la timeline pour inspecter sa causalité.
        </p>
      )}
    </aside>
  );
}
