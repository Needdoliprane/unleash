// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).
// Ne pas modifier manuellement.

import type { ErrorCode } from "../generated/ErrorCode";
import type { ReasonCode } from "../generated/ReasonCode";
import type { RetryClass } from "../generated/RetryClass";

export type GuiErrorPayload = { 
/**
 * Classe d'erreur canonique alignée sur le protocole Unleash.
 */
code: ErrorCode, 
/**
 * Raison typée si disponible.
 */
reason_code: ReasonCode | null, 
/**
 * Stratégie de reprise client.
 */
retry_class: RetryClass, 
/**
 * Message borné sans secret ni chemin système sensible.
 */
message: string, };
