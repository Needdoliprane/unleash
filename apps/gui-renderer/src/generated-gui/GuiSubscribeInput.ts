// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).
// Ne pas modifier manuellement.

export type GuiSubscribeInput = { 
/**
 * Projet optionnel à filtrer.
 */
project_id: string | null, 
/**
 * Agrégat optionnel à filtrer.
 */
aggregate_id: string | null, 
/**
 * Nombre maximal d'événements par lot (`1..=200`).
 */
batch_limit: number | null, };
