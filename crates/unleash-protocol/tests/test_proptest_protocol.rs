//! Tests de propriétés (proptest) pour le codec de trames, les opérations et les sérialisations du protocole.

#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation
)]

use proptest::prelude::*;
use std::io::Cursor;
use unleash_protocol::{FrameBuffer, ReasonCode, read_frame, write_frame};

proptest! {
    /// Propriété : Tout payload d'octets de taille <= 64 KiB peut être encodé puis décodé
    /// de manière strictement identique par le codec de trames.
    #[test]
    fn prop_frame_codec_roundtrip(payload in proptest::collection::vec(any::<u8>(), 0..65536)) {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &payload).expect("write_frame");
        prop_assert_eq!(buffer.len(), 4 + payload.len());

        let mut cursor = Cursor::new(buffer);
        let decoded = read_frame(&mut cursor).expect("read_frame").expect("frame présente");
        prop_assert_eq!(decoded, payload);
    }

    /// Propriété : FrameBuffer reconstitue exactement le flux quelles que soient les tailles
    /// de fragments dans lesquelles les octets sont injectés.
    #[test]
    fn prop_frame_buffer_chunked(
        payload in proptest::collection::vec(any::<u8>(), 1..8192),
        chunk_size in 1usize..64
    ) {
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &payload).unwrap();

        let mut fb = FrameBuffer::new();
        let mut result = None;

        for chunk in encoded.chunks(chunk_size) {
            fb.push_bytes(chunk);
            if let Some(frame) = fb.next_frame().unwrap() {
                result = Some(frame);
            }
        }

        prop_assert_eq!(result, Some(payload));
        prop_assert!(fb.is_empty());
    }

    /// Propriété : Tout ReasonCode survit à l'aller-retour chaîne / JSON.
    #[test]
    fn prop_reason_code_roundtrip(idx in 0usize..72) {
        let all_reasons = [
            ReasonCode::PublicationConflict,
            ReasonCode::ContractVersionConflict,
            ReasonCode::StaleEpoch,
            ReasonCode::OwnerEpochChanged,
            ReasonCode::StaleAuthorityGeneration,
            ReasonCode::BindingAlreadyEngaged,
            ReasonCode::StaleEvidence,
            ReasonCode::DoneNotMet,
            ReasonCode::LoopExhausted,
            ReasonCode::ContractRejected,
            ReasonCode::TerminalImmutable,
            ReasonCode::IdempotencyWindowExpired,
            ReasonCode::AppliedDraftConflict,
            ReasonCode::AppliedDraftImmutable,
            ReasonCode::ExpectedExecutionMissing,
            ReasonCode::EvidenceExecutionMismatch,
            ReasonCode::EvidenceProjectionInvalid,
            ReasonCode::EvidenceAmbiguous,
            ReasonCode::EvidenceSnapshotMismatch,
            ReasonCode::ClaimContradicted,
            ReasonCode::DiscoveryRequired,
            ReasonCode::DiscoveryEvidenceMissing,
            ReasonCode::RestoreIncomplete,
            ReasonCode::EvidenceUnavailable,
            ReasonCode::AutomationModePolicyMismatch,
            ReasonCode::AutomationBundleUnavailable,
            ReasonCode::AutomationTargetMissing,
            ReasonCode::AutomationTargetAmbiguous,
            ReasonCode::AutomationTargetDrift,
            ReasonCode::BusinessConstraintViolated,
            ReasonCode::BusinessPostconditionFailed,
            ReasonCode::AutomationAccountMismatch,
            ReasonCode::ExternalStateChanged,
            ReasonCode::ActionabilityTimeout,
            ReasonCode::AssertionTimeout,
            ReasonCode::UnresolvedEffect,
            ReasonCode::CapabilityUnavailable,
            ReasonCode::DecisionAuthorityRequired,
            ReasonCode::DefinitionRevoked,
            ReasonCode::ModelCallsForbidden,
            ReasonCode::OperationNotAuthorized,
            ReasonCode::IdentityUnverified,
            ReasonCode::BudgetExhausted,
            ReasonCode::SupervisionBudgetExhausted,
            ReasonCode::PredecessorNotTerminal,
            ReasonCode::ObjectDeleting,
            ReasonCode::Backpressure,
            ReasonCode::StorageBusy,
            ReasonCode::StorageLowSpace,
            ReasonCode::RecoveryInProgress,
            ReasonCode::AuthorityAlreadyActive,
            ReasonCode::DaemonAlreadyRunning,
            ReasonCode::RestorationBlocked,
            ReasonCode::AccountingMismatch,
            ReasonCode::AutomationHumanStepRequired,
            ReasonCode::SuccessorExists,
            ReasonCode::ClientUploadMismatch,
            ReasonCode::ControlReserveIncoherent,
            ReasonCode::AutomationInputInvalid,
            ReasonCode::UnknownValidator,
            ReasonCode::InvalidRuleParameters,
            ReasonCode::UnknownClaimPredicate,
            ReasonCode::SchemaLimitExceeded,
            ReasonCode::StorageFull,
            ReasonCode::StorageUnavailable,
            ReasonCode::StorageCorrupt,
            ReasonCode::StorageBuildUnqualified,
            ReasonCode::InvariantViolation,
            ReasonCode::EvidenceMissing,
            ReasonCode::AutomationProfileUnavailable,
            ReasonCode::CursorExpired,
            ReasonCode::StreamResyncRequired,
        ];

        let reason = all_reasons[idx % all_reasons.len()];
        let s = reason.as_str();
        let parsed: ReasonCode = s.parse().unwrap();
        prop_assert_eq!(parsed, reason);

        let _ = reason.error_code();
        let _ = reason.default_retry_class();

        let json_str = serde_json::to_string(&reason).unwrap();
        prop_assert_eq!(json_str.as_str(), format!("\"{s}\""));
        let deserialized: ReasonCode = serde_json::from_str(&json_str).unwrap();
        prop_assert_eq!(deserialized, reason);
    }
}
