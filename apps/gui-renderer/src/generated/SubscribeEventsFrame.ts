// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

import type { EventCursor } from "./EventCursor";
import type { EventEnvelope } from "./EventEnvelope";

export type SubscribeEventsFrame = { "kind": "events", 
/**
 * Liste des événements du lot.
 */
events: Array<EventEnvelope>, 
/**
 * Curseur de fin de lot.
 */
cursor: EventCursor, } | { "kind": "resync_required", 
/**
 * Nouveau flux d'événements actif.
 */
stream_id: string, 
/**
 * Watermark transactionnel actuel à charger via snapshot.
 */
current_watermark: EventCursor, 
/**
 * Motif de la demande de resynchronisation.
 */
reason: string, };
