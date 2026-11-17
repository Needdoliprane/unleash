// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type CommandEnvelope = { 
/**
 * Version du protocole (doit être `unleash.ipc.v2`).
 */
protocol_version: ProtocolVersion, 
/**
 * Identifiant de la transmission réseau (peut changer au retry).
 */
request_id: string, 
/**
 * Identifiant d'idempotence durable de la commande.
 */
command_id: string, 
/**
 * Lignée serveur épinglée lors de la création durable.
 */
authority_generation: string, 
/**
 * Identifiant de l'acteur authentifié résolu serveur (null si non fourni).
 */
actor_ref: string | null, 
/**
 * Identifiant du projet concerné ou null.
 */
project_id: string | null, 
/**
 * Agrégat ciblé (UUID).
 */
aggregate_id: string, 
/**
 * Révision attendue de l'agrégat (0 pour une création).
 */
expected_revision: number, 
/**
 * Nom canonique de l'opération (ex: `PauseMission`).
 */
command_type: string, 
/**
 * Charge utile métier de la commande.
 */
payload: Record<string, unknown>, };
