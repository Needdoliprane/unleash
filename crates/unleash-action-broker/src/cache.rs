// SPDX-License-Identifier: Apache-2.0
//! Cache borné des décisions `Allow` (`<= 300 s` et `<= expiry`, clé stricte incluant
//! `payload_digest`, `policy_revision` et `scope_revision`), avec invalidation instantanée
//! sur révocation, suspension ou changement de politique (`REQ-SEC-007`, `ai-guard.md`).

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use unleash_domain::{AuthorizationEvaluationId, ProjectId, Revision, Sha256Digest, TimestampMs};

use crate::effects::EffectsClass;
use crate::guard_port::{GuardDecisionRef, InternalObligation};

/// Plafond absolu de durée de réemploi d'une décision en cache (300 secondes, `ai-guard.md`).
pub const MAX_DECISION_CACHE_TTL: Duration = Duration::from_secs(300);

/// Clé composite exacte d'une entrée dans le cache de décisions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecisionCacheKey {
    /// Projet associé.
    pub project_id: Option<ProjectId>,
    /// Identité canonique du sujet.
    pub subject_key: String,
    /// Verbe exact.
    pub verb: String,
    /// Classe d'effet fermée.
    pub effects_class: EffectsClass,
    /// Empreinte canonique du payload.
    pub payload_digest: Sha256Digest,
    /// Révision exacte de la politique de sécurité.
    pub policy_revision: Revision,
    /// Révision exacte du scope.
    pub scope_revision: Revision,
    /// Profil de mapping actif au moment de l'évaluation.
    pub mapping_profile_id: String,
}

/// Entrée stockée dans le cache pour une décision `Allow` munie d'une `expiry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedDecision {
    /// Evaluation that obtained the original decision, distinct from a retry predecessor.
    pub source_evaluation_id: AuthorizationEvaluationId,
    /// Référence de décision originale.
    pub decision_ref: GuardDecisionRef,
    /// Obligations internes validées devant être réimposées à la réutilisation.
    pub obligations: Vec<InternalObligation>,
    /// Date d'expiration UTC fournie par le garde.
    pub expires_at_utc: TimestampMs,
    /// Instant monotone d'insertion en cache.
    pub cached_at_monotonic: Duration,
    /// Durée monotone maximale admise (`<= min(300s, ttl_configure, expiry - cached_at)`).
    pub effective_monotonic_ttl: Duration,
    /// Clés de sujet associées (pour invalidation instantanée sur suspension/révocation).
    pub subject_keys: Vec<String>,
    /// Projet concerné (pour invalidation instantanée sur changement de politique).
    pub project_id: Option<ProjectId>,
}

/// Cache borné en mémoire pour les autorisations `Allow` réutilisables.
#[derive(Debug)]
pub struct DecisionCache {
    max_entries: usize,
    configured_max_ttl: Duration,
    entries: BTreeMap<DecisionCacheKey, CachedDecision>,
    insertion_order: VecDeque<DecisionCacheKey>,
}

impl DecisionCache {
    /// Crée un cache borné à `max_entries` entrées et plafonné à `min(configured_max_ttl, 300s)`.
    #[must_use]
    pub fn new(max_entries: usize, configured_max_ttl: Duration) -> Self {
        Self {
            max_entries: max_entries.max(1),
            configured_max_ttl: configured_max_ttl.min(MAX_DECISION_CACHE_TTL),
            entries: BTreeMap::new(),
            insertion_order: VecDeque::new(),
        }
    }

    /// Met à jour le plafond configurable tout en conservant le plafond absolu de 300 s.
    pub fn set_configured_ttl(&mut self, ttl: Duration) {
        self.configured_max_ttl = ttl.min(MAX_DECISION_CACHE_TTL);
    }

    /// Recherche une décision encore valide selon BOTH l'horloge UTC (`expiry`) et
    /// l'horloge monotone (`<= 300s`). Évince automatiquement toute entrée expirée.
    pub fn get(
        &mut self,
        key: &DecisionCacheKey,
        now_utc: TimestampMs,
        now_mono: Duration,
    ) -> Option<CachedDecision> {
        let entry = self.entries.get(key)?.clone();
        if !is_entry_valid(&entry, now_utc, now_mono) {
            self.remove_key(key);
            return None;
        }
        Some(entry)
    }

    /// Insère une décision `Allow` uniquement si `expiry` est présente et strictement future.
    #[expect(
        clippy::too_many_arguments,
        reason = "Insertion explicite de tous les champs d'expiration UTC et monotone"
    )]
    pub fn insert(
        &mut self,
        key: DecisionCacheKey,
        source_evaluation_id: AuthorizationEvaluationId,
        decision_ref: GuardDecisionRef,
        obligations: Vec<InternalObligation>,
        expiry: Option<TimestampMs>,
        now_utc: TimestampMs,
        now_mono: Duration,
        subject_keys: Vec<String>,
    ) {
        let Some(expires_at_utc) = expiry else {
            // Règle canonique : sans expiry, aucun cache.
            return;
        };
        let remaining_ms = expires_at_utc.as_i64().saturating_sub(now_utc.as_i64());
        if remaining_ms <= 0 {
            return;
        }
        let Ok(remaining_u64) = u64::try_from(remaining_ms) else {
            return;
        };
        let effective_monotonic_ttl = Duration::from_millis(remaining_u64)
            .min(self.configured_max_ttl)
            .min(MAX_DECISION_CACHE_TTL);
        if effective_monotonic_ttl.is_zero() {
            return;
        }

        if self.entries.contains_key(&key) {
            self.remove_key(&key);
        }
        while self.entries.len() >= self.max_entries {
            if let Some(oldest) = self.insertion_order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }

        let project_id = key.project_id;
        let cached = CachedDecision {
            source_evaluation_id,
            decision_ref,
            obligations,
            expires_at_utc,
            cached_at_monotonic: now_mono,
            effective_monotonic_ttl,
            subject_keys,
            project_id,
        };
        self.insertion_order.push_back(key.clone());
        self.entries.insert(key, cached);
    }

    /// Invalide instantanément toutes les entrées liées au sujet `subject_key`.
    pub fn invalidate_subject(&mut self, subject_key: &str) {
        let doomed: Vec<DecisionCacheKey> = self
            .entries
            .iter()
            .filter(|(_, val)| val.subject_keys.iter().any(|s| s == subject_key))
            .map(|(k, _)| k.clone())
            .collect();
        for key in doomed {
            self.remove_key(&key);
        }
    }

    /// Invalide instantanément toutes les entrées associées à `project_id` (ou toutes si `None`).
    pub fn invalidate_project(&mut self, project_id: Option<ProjectId>) {
        let Some(pid) = project_id else {
            self.clear();
            return;
        };
        let doomed: Vec<DecisionCacheKey> = self
            .entries
            .iter()
            .filter(|(k, _)| k.project_id == Some(pid) || k.project_id.is_none())
            .map(|(k, _)| k.clone())
            .collect();
        for key in doomed {
            self.remove_key(&key);
        }
    }

    /// Vide entièrement le cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.insertion_order.clear();
    }

    /// Nombre d'entrées actuellement présentes en cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Indique si le cache est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn remove_key(&mut self, key: &DecisionCacheKey) {
        self.entries.remove(key);
        self.insertion_order.retain(|k| k != key);
    }
}

fn is_entry_valid(entry: &CachedDecision, now_utc: TimestampMs, now_mono: Duration) -> bool {
    if now_utc >= entry.expires_at_utc {
        return false;
    }
    let elapsed = now_mono.saturating_sub(entry.cached_at_monotonic);
    elapsed < entry.effective_monotonic_ttl && elapsed < MAX_DECISION_CACHE_TTL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_enforces_300s_cap_even_when_utc_rewinds_or_expiry_longer() {
        let mut cache = DecisionCache::new(16, Duration::from_secs(600));
        let key = DecisionCacheKey {
            project_id: None,
            subject_key: String::from("principal:u1"),
            verb: String::from("fs.write"),
            effects_class: EffectsClass::LocalWrite,
            payload_digest: Sha256Digest::from_bytes(b"p1"),
            policy_revision: Revision::new(1),
            scope_revision: Revision::new(1),
            mapping_profile_id: String::from("unleash.guard_mapping.none_accepted.v1"),
        };

        let t0_utc = TimestampMs::new(1_000_000);
        let t0_mono = Duration::from_secs(10);

        // Sans expiry -> aucun cache.
        cache.insert(
            key.clone(),
            AuthorizationEvaluationId::from_u128(1),
            GuardDecisionRef::new("d0").unwrap(),
            Vec::new(),
            None,
            t0_utc,
            t0_mono,
            vec![String::from("principal:u1")],
        );
        assert!(cache.is_empty());

        // Avec expiry à +600s -> plafonné à 300s monotones.
        cache.insert(
            key.clone(),
            AuthorizationEvaluationId::from_u128(1),
            GuardDecisionRef::new("d1").unwrap(),
            Vec::new(),
            Some(TimestampMs::new(1_600_000)),
            t0_utc,
            t0_mono,
            vec![String::from("principal:u1")],
        );
        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .get(&key, TimestampMs::new(1_200_000), Duration::from_secs(210))
                .is_some()
        );
        // Même si l'horloge murale recule, l'horloge monotone dépasse 300s -> éviction.
        assert!(
            cache
                .get(&key, TimestampMs::new(900_000), Duration::from_secs(310))
                .is_none()
        );
    }
}
