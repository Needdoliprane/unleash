// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).
// Ne pas modifier manuellement.

import type { EventCursor } from "../generated/EventCursor";
import type { EventEnvelope } from "../generated/EventEnvelope";

export type GuiStreamNotification = { "kind": "snapshot_loaded", 
/**
 * Watermark transactionnel du snapshot.
 */
watermark: EventCursor, 
/**
 * Projection JSON retournée par le daemon.
 */
snapshot: unknown, 
/**
 * Motif optionnel de resynchronisation.
 */
resync_reason: string | null, } | { "kind": "events", 
/**
 * Événements du lot.
 */
events: Array<EventEnvelope>, 
/**
 * Curseur atteint.
 */
cursor: EventCursor, } | { "kind": "disconnected", 
/**
 * Motif borné de déconnexion du flux.
 */
reason: string, };
