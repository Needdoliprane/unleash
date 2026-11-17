// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

import type { ProtocolVersion } from "./ProtocolVersion";

export type HelloResponse = { 
/**
 * Version protocolaire active du daemon (`unleash.ipc.v2`).
 */
protocol_version: ProtocolVersion, 
/**
 * Lignée d'autorité serveur active.
 */
authority_generation: string, 
/**
 * Époque d'incarnation du daemon au sein de la lignée.
 */
daemon_epoch: number, 
/**
 * Identifiant d'instance unique de ce daemon en cours d'exécution.
 */
daemon_instance_id: string, 
/**
 * Identifiant du flux principal d'événements du daemon.
 */
stream_id: string, 
/**
 * Taille maximale admise d'une trame en octets (8 MiB).
 */
max_frame_bytes: number, 
/**
 * Taille suggérée des segments pour les uploads découpés (ex: 1 MiB).
 */
suggested_segment_bytes: number, 
/**
 * Durée de conservation garantie des réponses d'idempotence en secondes (ex: 7 jours).
 */
idempotency_retention_seconds: number, 
/**
 * Capacités exposées par le daemon.
 */
capabilities: Array<string>, };
