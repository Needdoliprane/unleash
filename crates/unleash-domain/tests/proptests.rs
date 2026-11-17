//! Tests basés sur les propriétés (`proptest`) pour `unleash-domain`.
//!
//! Vérifie formellement les invariants critiques :
//! - Non-comparabilité d'ordre inter-lignées (INV-007)
//! - Non-réouverture terminale irréversible (INV-018, Q-03)
//! - Irréversibilité de tentative (`AttemptState`)
//! - Absence absolue de retry silencieux depuis `OutcomeUnknown` (INV-012, C4/C5)
//! - Normalisation bijective et idempotente de `Sha256Digest`

#![allow(
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::similar_names
)]

use proptest::prelude::*;
use unleash_domain::*;
use uuid::Uuid;

proptest! {
    /// Invariant 1 (INV-007) : Deux lignées d'autorité distinctes sont STRICTEMENT INCOMPARABLES par ordre.
    #[test]
    fn prop_lineage_fences_distinct_generations_are_never_comparable(
        u1_bytes in any::<[u8; 16]>(),
        u2_bytes in any::<[u8; 16]>(),
        epoch1 in any::<u64>(),
        epoch2 in any::<u64>(),
        fence1 in any::<u64>(),
        fence2 in any::<u64>(),
    ) {
        let u1 = Uuid::from_bytes(u1_bytes);
        let u2 = Uuid::from_bytes(u2_bytes);

        prop_assume!(u1 != u2);

        let g1 = AuthorityGeneration::from_uuid(u1);
        let g2 = AuthorityGeneration::from_uuid(u2);

        let lf1 = LineageFence::new(g1, DaemonEpoch::new(epoch1), Fence::new(fence1));
        let lf2 = LineageFence::new(g2, DaemonEpoch::new(epoch2), Fence::new(fence2));

        // Incomparabilité dans PartialOrd : partial_cmp renvoie TOUJOURS None
        prop_assert_eq!(lf1.partial_cmp(&lf2), None);
        prop_assert_eq!(lf2.partial_cmp(&lf1), None);

        // check_monotone_successor échoue obligatoirement avec LineageMismatch
        let is_mismatch = matches!(
            lf1.check_monotone_successor(&lf2),
            Err(FenceError::LineageMismatch { .. })
        );
        prop_assert!(is_mismatch);
    }

    /// Invariant 1b : Dans la même lignée, LineageFence est strictement monotone et ordonnée.
    #[test]
    fn prop_lineage_fences_same_generation_order(
        u_bytes in any::<[u8; 16]>(),
        epoch in any::<u64>(),
        fence1 in any::<u64>(),
        fence2 in any::<u64>(),
    ) {
        let g = AuthorityGeneration::from_uuid(Uuid::from_bytes(u_bytes));
        let lf1 = LineageFence::new(g, DaemonEpoch::new(epoch), Fence::new(fence1));
        let lf2 = LineageFence::new(g, DaemonEpoch::new(epoch), Fence::new(fence2));

        prop_assert_eq!(lf1.partial_cmp(&lf2), Some(fence1.cmp(&fence2)));

        if fence2 >= fence1 {
            prop_assert!(lf1.check_monotone_successor(&lf2).is_ok());
        } else {
            let is_stale = matches!(
                lf1.check_monotone_successor(&lf2),
                Err(FenceError::StaleFence { .. })
            );
            prop_assert!(is_stale);
        }
    }

    /// Invariant 2 (INV-018, Q-03) : Un run dans un état terminal (Succeeded, Failed, Cancelled)
    /// rejette TOUJOURS toute commande ou événement avec TerminalImmutable.
    #[test]
    fn prop_run_terminal_immutable(
        terminal_idx in 0usize..3,
        event_selector in 0usize..8,
    ) {
        let terminal_state = match terminal_idx {
            0 => RunState::Succeeded,
            1 => RunState::Failed,
            _ => RunState::Cancelled,
        };

        let event = match event_selector {
            0 => RunEvent::StartRun,
            1 => RunEvent::PauseRun { activities_reconciled: true },
            2 => RunEvent::ResumeRun,
            3 => RunEvent::CancelRun,
            4 => RunEvent::BodyClosed { done: GuardTruth::True },
            5 => RunEvent::ResolveBlock { resolved_guard: None, pause_requested: false },
            6 => RunEvent::ObservationRecorded,
            _ => RunEvent::EffectOutcomeResolved { all_barriers_cleared: true, pause_requested: false },
        };

        let res = transition_run(terminal_state, event);
        let is_terminal_immutable = matches!(
            res,
            Err(DomainTransitionError::TerminalImmutable { entity: "Run", .. })
        );
        prop_assert!(is_terminal_immutable);
    }

    /// Invariant 3 : Une tentative d'exécution terminale (Ended, Failed, Fenced)
    /// ne peut JAMAIS transitionner vers un autre état (AttemptIrreversible).
    #[test]
    fn prop_attempt_state_terminal_irreversible(
        terminal_idx in 0usize..3,
        target_selector in 0usize..7,
    ) {
        let terminal = match terminal_idx {
            0 => AttemptState::Ended,
            1 => AttemptState::Failed,
            _ => AttemptState::Fenced,
        };

        let target = match target_selector {
            0 => AttemptState::Admitted,
            1 => AttemptState::Running,
            2 => AttemptState::Checkpointing,
            3 => AttemptState::Reconciling,
            4 => AttemptState::Ended,
            5 => AttemptState::Failed,
            _ => AttemptState::Fenced,
        };

        let res = transition_attempt(terminal, target);
        let is_attempt_irreversible = matches!(
            res,
            Err(DomainTransitionError::AttemptIrreversible { .. })
        );
        prop_assert!(is_attempt_irreversible);
    }

    /// Invariant 4 (INV-012, C4/C5) : Interdiction absolue de retry ou dispatch direct depuis OutcomeUnknown.
    #[test]
    fn prop_action_cannot_retry_from_outcome_unknown(
        event_selector in 0usize..3,
        strategy_allows in any::<bool>(),
        auth_valid in any::<bool>(),
    ) {
        let event = match event_selector {
            0 => ActionEvent::CommitDispatchMarker,
            1 => ActionEvent::ConfirmDispatchDeparture,
            _ => ActionEvent::RetryAfterObservedFailure {
                strategy_allows_retry: strategy_allows,
                authorization_still_valid: auth_valid,
            },
        };

        let res = transition_action(ActionState::OutcomeUnknown, event);
        let is_cannot_retry = matches!(
            res,
            Err(DomainTransitionError::CannotRetryOutcomeUnknown)
        );
        prop_assert!(is_cannot_retry);
    }

    /// Invariant 5 : Normalisation et idempotence de Sha256Digest pour tout octet arbitraire.
    #[test]
    fn prop_sha256_digest_roundtrip(raw_bytes in any::<Vec<u8>>()) {
        let digest = Sha256Digest::from_bytes(&raw_bytes);
        let serialized = serde_json::to_string(&digest).unwrap();

        // Le JSON produit doit toujours être une chaîne débutant par sha256:
        prop_assert!(serialized.starts_with("\"sha256:"));

        // La désérialisation doit redonner exactement le même digest
        let deserialized: Sha256Digest = serde_json::from_str(&serialized).unwrap();
        prop_assert_eq!(digest, deserialized);

        // L'encodage hexadécimal doit comporter exactement 64 caractères minuscules
        let hex_str = digest.hex();
        prop_assert_eq!(hex_str.len(), 64);
        prop_assert!(hex_str.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    }
}
