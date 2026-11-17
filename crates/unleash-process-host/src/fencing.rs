// SPDX-License-Identifier: Apache-2.0
//! Vérification stricte de lignée, époque daemon, fence monotone et rotation de capacité (`REQ-TERM-003`).
//!
//! Invariants (`process-supervision.md`, `INV-007`) :
//! - Aucune comparaison d'ordre entre deux `AuthorityGeneration` (UUIDs incomparables).
//! - Un autre `authority_generation` est toujours rejeté (`StaleAuthorityGeneration`), même si son
//!   `daemon_epoch` ou son `resource_fence` paraît numériquement supérieur.
//! - Dans la même lignée, `daemon_epoch` et `resource_fence` sont monotones via [`LineageFence`].

use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, AuthorityGeneration, CommandId, DaemonEpoch, Fence, FenceError, LineageFence,
    ProcessRunUuid, Sha256Digest,
};

use crate::error::ProcessHostError;

/// En-tête commun exigé pour toute commande mutante ou de contrôle vers le process-host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCommandHeader {
    /// Version du protocole interne host (`1`).
    pub protocol_version: u32,
    /// UUID unique de l'exécution physique ciblée.
    pub process_run_uuid: ProcessRunUuid,
    /// Lignée d'autorité émettrice.
    pub authority_generation: AuthorityGeneration,
    /// Époque daemon courante dans cette lignée.
    pub daemon_epoch: DaemonEpoch,
    /// Fence de ressource/bail dans cette époque.
    pub resource_fence: Fence,
    /// Identifiant de commande idempotent.
    pub command_id: CommandId,
    /// Action autorisant la commande.
    pub action_id: ActionId,
    /// Digest canonique SHA-256 de la charge utile de commande.
    pub payload_hash: Sha256Digest,
}

/// État courant de fencing détenu en mémoire et persisté dans `HostManifest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FencingGate {
    process_run_uuid: ProcessRunUuid,
    current: LineageFence,
}

impl FencingGate {
    /// Version courante du protocole interne host.
    pub const PROTOCOL_VERSION: u32 = 1;

    /// Initialise la barrière de fencing à partir de la génération initiale du host.
    #[must_use]
    pub const fn new(
        process_run_uuid: ProcessRunUuid,
        authority_generation: AuthorityGeneration,
        daemon_epoch: DaemonEpoch,
        highest_fence: Fence,
    ) -> Self {
        Self {
            process_run_uuid,
            current: LineageFence::new(authority_generation, daemon_epoch, highest_fence),
        }
    }

    /// Retourne la barrière courante.
    #[must_use]
    pub const fn current(&self) -> LineageFence {
        self.current
    }

    /// Valide l'en-tête d'une commande courante et fait avancer `(epoch, fence)` si monotone.
    ///
    /// # Errors
    /// - [`ProcessHostError::InvalidArgument`] si `protocol_version != 1`.
    /// - [`ProcessHostError::IdentityUnverified`] si `process_run_uuid` diverge.
    /// - [`ProcessHostError::StaleAuthorityGeneration`] si `authority_generation` diffère.
    /// - [`ProcessHostError::StaleEpochOrFence`] si `daemon_epoch` ou `resource_fence` recule.
    pub fn check_and_advance(
        &mut self,
        header: &HostCommandHeader,
    ) -> Result<(), ProcessHostError> {
        self.validate_common(header)?;
        let candidate = LineageFence::new(
            header.authority_generation,
            header.daemon_epoch,
            header.resource_fence,
        );
        self.apply_candidate(candidate)
    }

    /// Valide un handshake de rattachement après redémarrage daemon dans la même lignée.
    ///
    /// Exige que `candidate.daemon_epoch >= self.current.epoch` (et si `require_strictly_higher_epoch`
    /// est vrai lors d'une récupération post-crash, exige `candidate.daemon_epoch > self.current.epoch`).
    ///
    /// # Errors
    /// Mêmes erreurs que [`Self::check_and_advance`], plus rejet si l'époque n'a pas progressé
    /// lorsque `require_strictly_higher_epoch` est actif.
    pub fn check_recovery_handshake(
        &mut self,
        header: &HostCommandHeader,
        require_strictly_higher_epoch: bool,
    ) -> Result<(), ProcessHostError> {
        self.validate_common(header)?;
        if require_strictly_higher_epoch && header.daemon_epoch <= self.current.epoch {
            return Err(ProcessHostError::StaleEpochOrFence {
                reason: format!(
                    "le rattachement exige un daemon_epoch strictement supérieur à {} (reçu {})",
                    self.current.epoch, header.daemon_epoch
                ),
            });
        }
        let candidate = LineageFence::new(
            header.authority_generation,
            header.daemon_epoch,
            header.resource_fence,
        );
        self.apply_candidate(candidate)
    }

    fn validate_common(&self, header: &HostCommandHeader) -> Result<(), ProcessHostError> {
        if header.protocol_version != Self::PROTOCOL_VERSION {
            return Err(ProcessHostError::InvalidArgument {
                reason: format!(
                    "version protocole host non supportée : {}",
                    header.protocol_version
                ),
            });
        }
        if header.process_run_uuid != self.process_run_uuid {
            return Err(ProcessHostError::IdentityUnverified {
                reason: format!(
                    "process_run_uuid divergent : attendu {}, reçu {}",
                    self.process_run_uuid, header.process_run_uuid
                ),
            });
        }
        if header.authority_generation != self.current.authority_generation {
            return Err(ProcessHostError::StaleAuthorityGeneration {
                expected: self.current.authority_generation.to_string(),
                actual: header.authority_generation.to_string(),
            });
        }
        Ok(())
    }

    fn apply_candidate(&mut self, candidate: LineageFence) -> Result<(), ProcessHostError> {
        match self.current.check_monotone_successor(&candidate) {
            Ok(()) => {
                self.current = candidate;
                Ok(())
            }
            Err(FenceError::LineageMismatch { expected, actual }) => {
                Err(ProcessHostError::StaleAuthorityGeneration { expected, actual })
            }
            Err(err @ (FenceError::StaleFence { .. } | FenceError::IncomparableLineage)) => {
                Err(ProcessHostError::StaleEpochOrFence {
                    reason: err.to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_header(
        run_uuid: ProcessRunUuid,
        gen_id: AuthorityGeneration,
        epoch: u64,
        fence: u64,
    ) -> HostCommandHeader {
        HostCommandHeader {
            protocol_version: FencingGate::PROTOCOL_VERSION,
            process_run_uuid: run_uuid,
            authority_generation: gen_id,
            daemon_epoch: DaemonEpoch::new(epoch),
            resource_fence: Fence::new(fence),
            command_id: CommandId::from_u128(10),
            action_id: ActionId::from_u128(20),
            payload_hash: Sha256Digest::from_bytes(b"payload"),
        }
    }

    #[test]
    fn rejects_foreign_authority_generation_even_with_higher_epoch() {
        let run_uuid = ProcessRunUuid::from_u128(1);
        let gen_a = AuthorityGeneration::from_u128(100);
        let gen_b = AuthorityGeneration::from_u128(200);
        let mut gate = FencingGate::new(run_uuid, gen_a, DaemonEpoch::new(2), Fence::new(5));

        let foreign = mk_header(run_uuid, gen_b, 9999, 9999);
        let err = gate.check_and_advance(&foreign).unwrap_err();
        assert!(matches!(
            err,
            ProcessHostError::StaleAuthorityGeneration { .. }
        ));
        assert_eq!(gate.current().epoch, DaemonEpoch::new(2));
    }

    #[test]
    fn rejects_lower_epoch_or_lower_fence_in_same_lineage() {
        let run_uuid = ProcessRunUuid::from_u128(1);
        let gen_a = AuthorityGeneration::from_u128(100);
        let mut gate = FencingGate::new(run_uuid, gen_a, DaemonEpoch::new(3), Fence::new(5));

        // Epoch inférieur même avec fence supérieur -> rejet
        let old_epoch = mk_header(run_uuid, gen_a, 2, 99);
        assert!(matches!(
            gate.check_and_advance(&old_epoch),
            Err(ProcessHostError::StaleEpochOrFence { .. })
        ));

        // Même epoch mais fence inférieur -> rejet
        let old_fence = mk_header(run_uuid, gen_a, 3, 4);
        assert!(matches!(
            gate.check_and_advance(&old_fence),
            Err(ProcessHostError::StaleEpochOrFence { .. })
        ));

        // Epoch supérieur même avec fence remis à 1 -> accepté (epoch domine dans la même lignée)
        let next_epoch = mk_header(run_uuid, gen_a, 4, 1);
        gate.check_and_advance(&next_epoch).unwrap();
        assert_eq!(gate.current().epoch, DaemonEpoch::new(4));
        assert_eq!(gate.current().fence, Fence::new(1));
    }
}
