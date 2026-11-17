// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

import type { CommandReceipt } from "./CommandReceipt";
import type { LineageStatus } from "./LineageStatus";

export type CommandReceiptInspection = { 
/**
 * Reçu de commande original.
 */
receipt: CommandReceipt, 
/**
 * Lignée d'autorité sous laquelle le reçu a été engagé.
 */
authority_generation: string, 
/**
 * Indication si la lignée est courante ou historique.
 */
lineage_status: LineageStatus, 
/**
 * Référence de provenance pour audit.
 */
provenance_ref: string | null, };
