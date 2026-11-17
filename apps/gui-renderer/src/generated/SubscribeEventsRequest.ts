// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type SubscribeEventsRequest = { 
/**
 * Flux d'événements ciblé.
 */
stream_id: string, 
/**
 * Numéro de séquence à partir duquel reprendre l'écoute (> `after_sequence`).
 */
after_sequence: number, 
/**
 * Filtre projet optionnel.
 */
project_id?: string | null, 
/**
 * Filtre agrégat optionnel.
 */
aggregate_id?: string | null, 
/**
 * Nombre maximal d'événements par trame de lot.
 */
batch_limit: number, };
