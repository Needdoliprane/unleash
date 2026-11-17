import { useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Button, Input, Label, SearchField } from "react-aria-components";
import { buildEventPage } from "../data/m0Fixtures";
import type { TimelineEventItem } from "../ipc/types";

type TimelinePanelProps = {
  totalEvents: number;
  pageSize?: number;
  laggingCount: number;
  selectedEventId: string | null;
  onSelectEvent: (item: TimelineEventItem) => void;
};

export function TimelinePanel({
  totalEvents,
  pageSize = 200,
  laggingCount,
  selectedEventId,
  onSelectEvent,
}: TimelinePanelProps) {
  const [pageOffset, setPageOffset] = useState(0);
  const [filterInput, setFilterInput] = useState("");
  const [committedFilter, setCommittedFilter] = useState("");
  const [isComposing, setIsComposing] = useState(false);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);

  const page = useMemo(
    () => buildEventPage(pageOffset, pageSize, committedFilter, totalEvents),
    [pageOffset, pageSize, committedFilter, totalEvents],
  );

  const virtualizer = useVirtualizer({
    count: page.items.length,
    getScrollElement: () => scrollContainerRef.current,
    estimateSize: () => 38,
    overscan: 10,
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

  const canPrev = pageOffset > 0;
  const canNext = pageOffset + pageSize < totalEvents;

  return (
    <section className="panel-card" aria-labelledby="timeline-panel-title">
      <div className="panel-header">
        <div>
          <h2 id="timeline-panel-title" className="panel-title">
            Timeline des événements ({totalEvents.toLocaleString("fr-FR")} total, page de{" "}
            {page.items.length})
          </h2>
          <div className="header-meta" style={{ marginTop: "6px" }}>
            <span className="badge mono">Curseur : seq {page.cursor.sequence}</span>
            <span className="badge">
              Fenêtre : {pageOffset + 1}..{Math.min(totalEvents, pageOffset + pageSize)}
            </span>
            {laggingCount > 0 ? (
              <span className="badge badge-attention" role="status">
                Flux retardé : {laggingCount} en attente
              </span>
            ) : (
              <span className="badge badge-success">Flux à jour (backpressure 0)</span>
            )}
          </div>
        </div>

        <SearchField
          aria-label="Filtrer les événements de la timeline"
          value={filterInput}
          onChange={(nextValue) => {
            setFilterInput(nextValue);
            if (!isComposing) {
              setCommittedFilter(nextValue);
              setPageOffset(0);
            }
          }}
        >
          <Label className="mono" style={{ marginRight: "8px" }}>
            Filtre events
          </Label>
          <Input
            className="control-input"
            placeholder="Type, lot, séquence..."
            data-testid="timeline-filter-input"
            onCompositionStart={() => {
              setIsComposing(true);
            }}
            onCompositionEnd={(event) => {
              setIsComposing(false);
              setCommittedFilter(event.currentTarget.value);
              setPageOffset(0);
            }}
          />
        </SearchField>
      </div>

      <div
        ref={scrollContainerRef}
        className="virtual-scroll"
        role="listbox"
        aria-label="Journal d'événements paginé"
        data-testid="timeline-scroll"
      >
        <div
          style={{
            height: `${String(virtualizer.getTotalSize())}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const item = page.items[virtualRow.index];
            if (!item) {
              return null;
            }
            const isSelected = item.envelope.event_id === selectedEventId;
            return (
              <button
                key={item.envelope.event_id}
                type="button"
                role="option"
                aria-selected={isSelected}
                data-selected={isSelected}
                data-testid={`event-row-${String(item.envelope.sequence)}`}
                className="virtual-row"
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  height: `${String(virtualRow.size)}px`,
                  transform: `translateY(${String(virtualRow.start)}px)`,
                }}
                onClick={() => {
                  onSelectEvent(item);
                }}
              >
                <span
                  style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
                >
                  {item.summary}
                </span>
                <span
                  className={`badge ${
                    item.severity === "danger"
                      ? "badge-danger"
                      : item.severity === "attention"
                        ? "badge-attention"
                        : ""
                  }`}
                >
                  {item.envelope.sensitivity}
                </span>
              </button>
            );
          })}
        </div>
      </div>

      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          marginTop: "10px",
          gap: "8px",
        }}
      >
        <div style={{ display: "flex", gap: "8px" }}>
          <Button
            className="control-button"
            isDisabled={!canPrev}
            onPress={() => {
              setPageOffset((prev) => Math.max(0, prev - pageSize));
            }}
          >
            Page précédente
          </Button>
          <Button
            className="control-button"
            isDisabled={!canNext}
            onPress={() => {
              setPageOffset((prev) =>
                Math.min(Math.max(0, totalEvents - pageSize), prev + pageSize),
              );
            }}
          >
            Page suivante
          </Button>
          <Button
            className="control-button"
            onPress={() => {
              setPageOffset(Math.max(0, totalEvents - pageSize));
            }}
          >
            Fin (100 000)
          </Button>
        </div>
        <span className="mono">
          Cache borné : {page.items.length} / {pageSize} max par page
        </span>
      </div>
    </section>
  );
}
