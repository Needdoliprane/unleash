// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type QueryResponse = { 
/**
 * Identifiant de corrélation correspondant à la requête.
 */
request_id: string, 
/**
 * Watermark transactionnel de la vue projetée si applicable.
 */
watermark?: EventCursor | null, 
/**
 * Résultat de la requête.
 */
result: any, };
