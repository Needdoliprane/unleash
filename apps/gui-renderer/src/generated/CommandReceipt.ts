// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type CommandReceipt = { 
/**
 * Identifiant global unique serveur du reçu.
 */
receipt_id: string, 
/**
 * Identifiant de la commande originale.
 */
command_id: string, 
/**
 * Horodatage UTC RFC 3339 d'acceptation de la commande.
 */
accepted_at: string, 
/**
 * Agrégat ciblé.
 */
aggregate_id: string, 
/**
 * Nouvelle révision de l'agrégat suite à l'engagement.
 */
new_revision: number, 
/**
 * Identifiant de l'opération longue initiée ou null. Clé toujours présente.
 */
operation_id: string | null, 
/**
 * État d'avancement du traitement.
 */
state: ReceiptState, 
/**
 * Curseur d'événement correspondant dans le journal.
 */
event_cursor: EventCursor, 
/**
 * Référence opaque vers le résultat produit ou null si absent.
 */
result_ref?: string | null, };
