// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type IpcError = { 
/**
 * Classe commune d'erreur parmi les 17 canoniques.
 */
code: ErrorCode, 
/**
 * Raison typée ou null. Clé toujours présente.
 */
reason_code: ReasonCode | null, 
/**
 * Message d'erreur explicatif (borné à 4096 caractères).
 */
message: string, 
/**
 * Stratégie de reprise autorisée pour le client.
 */
retry_class: RetryClass, 
/**
 * Référence opaque vers les détails sensibles protégés ou null. Clé toujours présente.
 */
details_ref: string | null, 
/**
 * Identifiant de commande corrélé ou null. Clé toujours présente.
 */
command_id: string | null, 
/**
 * Révision courante de l'agrégat (présente uniquement lors de conflits de révision).
 */
current_revision?: number, };
