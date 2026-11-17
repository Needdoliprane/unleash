// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type EventEnvelope = { 
/**
 * Identifiant unique de l'événement (UUID).
 */
event_id: string, 
/**
 * Identifiant du flux ordonné auquel appartient l'événement.
 */
stream_id: string, 
/**
 * Numéro de séquence strictement monotone dans le flux (>= 1).
 */
sequence: number, 
/**
 * Type de fait intervenu (ex: `InvocationAdmitted`).
 */
event_type: string, 
/**
 * Version de l'enveloppe (vaut 1).
 */
schema_version: number, 
/**
 * Date-heure UTC de survenance du fait.
 */
occurred_at: string, 
/**
 * Date-heure UTC d'enregistrement en base.
 */
recorded_at: string, 
/**
 * Projet associé ou null.
 */
project_id: string | null, 
/**
 * Agrégat modifié par cet événement.
 */
aggregate_id: string, 
/**
 * Révision résultante de l'agrégat (>= 1).
 */
aggregate_revision: number, 
/**
 * Acteur à l'origine du fait.
 */
actor_ref: string, 
/**
 * Identifiant de corrélation transverse de la requête.
 */
correlation_id: string, 
/**
 * Lien causal typé ayant produit cet événement.
 */
causation: EventCausation, 
/**
 * Identifiant de la commande liée (informatif, ou null).
 */
command_id: string | null, 
/**
 * Empreinte canonique SHA-256 du payload (`sha256:<hex64>`).
 */
payload_hash: string, 
/**
 * Charge utile métier de l'événement (objet JSON).
 */
payload: Record<string, unknown>, 
/**
 * Niveau de sensibilité de l'événement.
 */
sensitivity: Sensitivity, };
