import { useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Button, Input, Label, SearchField } from "react-aria-components";
import type { AgentNode, PoolStatus } from "../ipc/types";

type AgentTreePanelProps = {
  agents: readonly AgentNode[];
  pool: PoolStatus;
  selectedAgentId: string | null;
  onSelectAgent: (agent: AgentNode) => void;
};

export function AgentTreePanel({
  agents,
  pool,
  selectedAgentId,
  onSelectAgent,
}: AgentTreePanelProps) {
  const [filterInput, setFilterInput] = useState("");
  const [committedFilter, setCommittedFilter] = useState("");
  const [isComposing, setIsComposing] = useState(false);
  const [collapsedIds, setCollapsedIds] = useState<ReadonlySet<string>>(() => new Set());
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);

  const visibleAgents = useMemo(() => {
    const query = committedFilter.trim().toLowerCase();
    if (query.length > 0) {
      return agents.filter(
        (agent) =>
          agent.name.toLowerCase().includes(query) ||
          agent.role.toLowerCase().includes(query) ||
          agent.logicalState.toLowerCase().includes(query) ||
          agent.model.toLowerCase().includes(query),
      );
    }

    const byId = new Map<string, AgentNode>();
    for (const agent of agents) {
      byId.set(agent.id, agent);
    }

    const isAncestorCollapsed = (node: AgentNode): boolean => {
      let currentParent = node.parentId;
      while (currentParent !== null) {
        if (collapsedIds.has(currentParent)) {
          return true;
        }
        currentParent = byId.get(currentParent)?.parentId ?? null;
      }
      return false;
    };

    return agents.filter((agent) => !isAncestorCollapsed(agent));
  }, [agents, collapsedIds, committedFilter]);

  const virtualizer = useVirtualizer({
    count: visibleAgents.length,
    getScrollElement: () => scrollContainerRef.current,
    estimateSize: () => 36,
    overscan: 8,
    initialRect: { width: 640, height: 300 },
    observeElementRect: (instance, cb) => {
      const el = instance.scrollElement;
      if (!el) {
        cb({ width: 640, height: 300 });
        return () => {};
      }
      const update = (): void => {
        const rect = el.getBoundingClientRect();
        cb(
          rect.height > 0
            ? { width: rect.width, height: rect.height }
            : { width: 640, height: 300 },
        );
      };
      update();
      const observer = new ResizeObserver(update);
      observer.observe(el);
      return () => {
        observer.disconnect();
      };
    },
  });

  const toggleCollapse = (agentId: string): void => {
    setCollapsedIds((prev) => {
      const next = new Set(prev);
      if (next.has(agentId)) {
        next.delete(agentId);
      } else {
        next.add(agentId);
      }
      return next;
    });
  };

  const dormantCount = useMemo(
    () => agents.filter((a) => a.logicalState === "dormant").length,
    [agents],
  );
  const activeCount = useMemo(
    () => agents.filter((a) => a.logicalState === "active").length,
    [agents],
  );
  const blockedCount = useMemo(
    () => agents.filter((a) => a.logicalState === "blocked").length,
    [agents],
  );

  return (
    <section className="panel-card" aria-labelledby="agents-panel-title">
      <div className="panel-header">
        <div>
          <h2 id="agents-panel-title" className="panel-title">
            Arbre hiérarchique des agents ({agents.length} logiques)
          </h2>
          <div className="header-meta" style={{ marginTop: "6px" }}>
            <span className="badge badge-success">Actifs : {activeCount}</span>
            <span className="badge">Dormants : {dormantCount}</span>
            <span className="badge badge-danger">Bloqués : {blockedCount}</span>
            <span className="badge badge-attention">
              Pool global : {pool.activeSlots}/{pool.configuredLimit} (max {pool.hardMaxLimit},
              réserve {pool.controlReserve})
            </span>
          </div>
        </div>

        <SearchField
          aria-label="Filtrer les agents logiques"
          value={filterInput}
          onChange={(nextValue) => {
            setFilterInput(nextValue);
            if (!isComposing) {
              setCommittedFilter(nextValue);
            }
          }}
        >
          <Label className="mono" style={{ marginRight: "8px" }}>
            Filtre agents
          </Label>
          <Input
            className="control-input"
            placeholder="Nom, rôle, état, modèle..."
            data-testid="agent-filter-input"
            onCompositionStart={() => {
              setIsComposing(true);
            }}
            onCompositionEnd={(event) => {
              setIsComposing(false);
              setCommittedFilter(event.currentTarget.value);
            }}
          />
          {filterInput.length > 0 ? (
            <Button
              className="control-button"
              style={{ marginLeft: "6px" }}
              onPress={() => {
                setFilterInput("");
                setCommittedFilter("");
              }}
            >
              Effacer
            </Button>
          ) : null}
        </SearchField>
      </div>

      <div
        ref={scrollContainerRef}
        className="virtual-scroll"
        role="tree"
        aria-label="Hiérarchie des agents logiques"
        data-testid="agent-tree-scroll"
      >
        <div
          style={{
            height: `${String(virtualizer.getTotalSize())}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const agent = visibleAgents[virtualRow.index];
            if (!agent) {
              return null;
            }
            const isSelected = agent.id === selectedAgentId;
            const isCollapsed = collapsedIds.has(agent.id);
            const indentPx = agent.depth * 18 + 12;

            return (
              <div
                key={agent.id}
                role="treeitem"
                aria-level={agent.depth + 1}
                aria-selected={isSelected}
                aria-expanded={agent.hasChildren ? !isCollapsed : undefined}
                tabIndex={0}
                data-selected={isSelected}
                data-testid={`agent-row-${agent.id}`}
                className="virtual-row"
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  height: `${String(virtualRow.size)}px`,
                  transform: `translateY(${String(virtualRow.start)}px)`,
                  paddingLeft: `${String(indentPx)}px`,
                }}
                onClick={() => {
                  onSelectAgent(agent);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    onSelectAgent(agent);
                  } else if (event.key === "ArrowRight" && agent.hasChildren && isCollapsed) {
                    event.preventDefault();
                    toggleCollapse(agent.id);
                  } else if (event.key === "ArrowLeft" && agent.hasChildren && !isCollapsed) {
                    event.preventDefault();
                    toggleCollapse(agent.id);
                  } else if (event.key === "ArrowDown") {
                    const next = visibleAgents[virtualRow.index + 1];
                    if (next) {
                      event.preventDefault();
                      onSelectAgent(next);
                      virtualizer.scrollToIndex(virtualRow.index + 1);
                    }
                  } else if (event.key === "ArrowUp") {
                    const prev = visibleAgents[virtualRow.index - 1];
                    if (prev) {
                      event.preventDefault();
                      onSelectAgent(prev);
                      virtualizer.scrollToIndex(virtualRow.index - 1);
                    }
                  }
                }}
              >
                <span style={{ display: "inline-flex", alignItems: "center", gap: "8px" }}>
                  {agent.hasChildren ? (
                    <button
                      type="button"
                      className="control-button"
                      style={{ padding: "1px 6px", fontSize: "11px" }}
                      aria-label={isCollapsed ? `Déplier ${agent.name}` : `Replier ${agent.name}`}
                      onClick={(event) => {
                        event.stopPropagation();
                        toggleCollapse(agent.id);
                      }}
                    >
                      {isCollapsed ? "+" : "−"}
                    </button>
                  ) : (
                    <span aria-hidden="true" className="mono">
                      ·
                    </span>
                  )}
                  <span className="badge">{agent.level}</span>
                  <span>{agent.name}</span>
                </span>
                <span style={{ display: "inline-flex", alignItems: "center", gap: "8px" }}>
                  <span className="mono">{agent.model}</span>
                  <span
                    className={`badge ${
                      agent.logicalState === "active"
                        ? "badge-success"
                        : agent.logicalState === "blocked" || agent.logicalState === "error"
                          ? "badge-danger"
                          : agent.logicalState === "waiting_tool"
                            ? "badge-attention"
                            : ""
                    }`}
                  >
                    logique:{agent.logicalState} / inv:{agent.invocationState}
                  </span>
                </span>
              </div>
            );
          })}
        </div>
      </div>

      <p className="mono" style={{ marginTop: "8px", marginBottom: 0 }}>
        Lignes visibles : {visibleAgents.length} / {agents.length} (virtualisées)
      </p>
    </section>
  );
}
