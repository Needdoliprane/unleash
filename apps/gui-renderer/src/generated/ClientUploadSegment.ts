// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type ClientUploadSegment = { 
/**
 * Commande liée à cet import de données.
 */
command_id: string, 
/**
 * Index du segment (0-based).
 */
segment_index: number, 
/**
 * Nombre total de segments attendus.
 */
total_segments: number, 
/**
 * Position de départ en octets dans le fichier source.
 */
byte_offset: number, 
/**
 * Longueur de ce segment en octets.
 */
byte_length: number, 
/**
 * Données brutes du segment encodées en base64.
 */
data_base64: string, 
/**
 * Empreinte canonique SHA-256 du segment.
 */
sha256: string, };
