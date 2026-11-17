// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

import type { EventCausationKind } from "./EventCausationKind";

export type EventCausation = { 
/**
 * Famille causale.
 */
kind: EventCausationKind, 
/**
 * Identifiant de la cause (ex: `receipt_id` si `kind=command`).
 */
id: string, };
