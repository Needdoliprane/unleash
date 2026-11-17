// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type QueryEnvelope = { 
/**
 * Version du protocole (doit être `unleash.ipc.v2`).
 */
protocol_version: ProtocolVersion, 
/**
 * Identifiant de corrélation de la requête.
 */
request_id: string, 
/**
 * Nom canonique de la requête (ex: `GetAgent`).
 */
query_type: string, 
/**
 * Périmètre projet optionnel.
 */
project_id: string | null, 
/**
 * Paramètres typés de la lecture.
 */
payload: Record<string, unknown>, };
