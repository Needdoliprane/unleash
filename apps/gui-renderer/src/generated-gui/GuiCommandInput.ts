// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).
// Ne pas modifier manuellement.

export type GuiCommandInput = { 
/**
 * Identifiant optionnel du projet ciblé.
 */
project_id: string | null, 
/**
 * Identifiant UUID de l'agrégat ciblé.
 */
aggregate_id: string, 
/**
 * Révision attendue par contrôle de concurrence optimiste.
 */
expected_revision: number, 
/**
 * Clé d'idempotence durable conservée sur retry/reconnexion.
 */
command_id: string, 
/**
 * Charge utile métier JSON (`actor_ref` interdit).
 */
payload: Record<string, unknown>, };
