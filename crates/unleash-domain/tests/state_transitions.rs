//! Tests table-driven exhaustifs des machines à états d'Unleash.
//!
//! Reproduit chaque ligne des tables de transitions de :
//! - `session-model.md`
//! - `state-machines.md`
//! - `runtime-lifecycle.md`
//! - `failure-recovery.md`
//!
//! Couvre les exigences :
//! // REQ-CORE-001 / INV-001 / INV-007 / INV-012 / INV-018 / INV-035 / REQ-PERSIST-001

#![allow(
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::similar_names
)]

use std::str::FromStr;
use unleash_domain::*;

#[test]
fn test_daemon_mode_transitions_table() {
    // runtime-lifecycle.md section "Machine d'état du daemon"
    let valid_transitions = vec![
        (DaemonMode::Starting, DaemonMode::Recovering),
        (DaemonMode::Recovering, DaemonMode::Ready),
        (DaemonMode::Recovering, DaemonMode::Degraded),
        (DaemonMode::Recovering, DaemonMode::ReadOnlyRecovery),
        (DaemonMode::Recovering, DaemonMode::Draining),
        (DaemonMode::Ready, DaemonMode::Draining),
        (DaemonMode::Ready, DaemonMode::Degraded),
        (DaemonMode::Ready, DaemonMode::ReadOnlyRecovery),
        (DaemonMode::Degraded, DaemonMode::Ready),
        (DaemonMode::Degraded, DaemonMode::ReadOnlyRecovery),
        (DaemonMode::Degraded, DaemonMode::Draining),
        (DaemonMode::ReadOnlyRecovery, DaemonMode::Draining),
        (DaemonMode::ReadOnlyRecovery, DaemonMode::Stopped),
        (DaemonMode::Draining, DaemonMode::Stopped),
        (DaemonMode::Stopped, DaemonMode::Starting),
    ];

    for (from, to) in valid_transitions {
        assert_eq!(
            transition_daemon_mode(from, to),
            Ok(to),
            "Transition attendue valide de {:?} à {:?}",
            from,
            to
        );
    }

    // Cas interdits
    let invalid_transitions = vec![
        (DaemonMode::Starting, DaemonMode::Ready),
        (DaemonMode::Starting, DaemonMode::Stopped),
        (DaemonMode::Ready, DaemonMode::Starting),
        (DaemonMode::Ready, DaemonMode::Stopped),
        (DaemonMode::Draining, DaemonMode::Ready),
        (DaemonMode::Stopped, DaemonMode::Ready),
    ];

    for (from, to) in invalid_transitions {
        assert!(
            transition_daemon_mode(from, to).is_err(),
            "Transition de {:?} à {:?} aurait dû être rejetée",
            from,
            to
        );
    }
}

#[test]
fn test_run_transitions_table() {
    // state-machines.md section "Run"

    // 1. prepared + StartRun -> running
    assert_eq!(
        transition_run(RunState::Prepared, RunEvent::StartRun),
        Ok(RunState::Running)
    );

    // 2. running + PauseRun
    assert_eq!(
        transition_run(
            RunState::Running,
            RunEvent::PauseRun {
                activities_reconciled: true
            }
        ),
        Ok(RunState::Paused)
    );
    assert_eq!(
        transition_run(
            RunState::Running,
            RunEvent::PauseRun {
                activities_reconciled: false
            }
        ),
        Ok(RunState::Blocked)
    );

    // 3. running + NeedEvidence, NeedAuthorization, EffectUnknown -> blocked
    assert_eq!(
        transition_run(RunState::Running, RunEvent::NeedEvidence),
        Ok(RunState::Blocked)
    );
    assert_eq!(
        transition_run(RunState::Running, RunEvent::NeedAuthorization),
        Ok(RunState::Blocked)
    );
    assert_eq!(
        transition_run(RunState::Running, RunEvent::EffectUnknown),
        Ok(RunState::Blocked)
    );

    // 4. blocked + ResolveBlock
    assert_eq!(
        transition_run(
            RunState::Blocked,
            RunEvent::ResolveBlock {
                resolved_guard: None,
                pause_requested: false
            }
        ),
        Ok(RunState::Running)
    );
    assert_eq!(
        transition_run(
            RunState::Blocked,
            RunEvent::ResolveBlock {
                resolved_guard: None,
                pause_requested: true
            }
        ),
        Ok(RunState::Paused)
    );

    // 5. paused + ResumeRun -> running
    assert_eq!(
        transition_run(RunState::Paused, RunEvent::ResumeRun),
        Ok(RunState::Running)
    );

    // 6. prepared/running/paused/blocked + CancelRun -> cancelling
    for state in [
        RunState::Prepared,
        RunState::Running,
        RunState::Paused,
        RunState::Blocked,
    ] {
        assert_eq!(
            transition_run(state, RunEvent::CancelRun),
            Ok(RunState::Cancelling)
        );
    }

    // 7. cancelling + CancellationReconciled -> cancelled
    assert_eq!(
        transition_run(RunState::Cancelling, RunEvent::CancellationReconciled),
        Ok(RunState::Cancelled)
    );

    // 8. running + BodyClosed:
    // done vrai -> succeeded
    assert_eq!(
        transition_run(
            RunState::Running,
            RunEvent::BodyClosed {
                done: GuardTruth::True
            }
        ),
        Ok(RunState::Succeeded)
    );
    // done faux -> failed
    assert_eq!(
        transition_run(
            RunState::Running,
            RunEvent::BodyClosed {
                done: GuardTruth::False
            }
        ),
        Ok(RunState::Failed)
    );
    // done inconnu -> blocked (GUARD_UNKNOWN)
    assert_eq!(
        transition_run(
            RunState::Running,
            RunEvent::BodyClosed {
                done: GuardTruth::Unknown
            }
        ),
        Ok(RunState::Blocked)
    );

    // 9. blocked (suite à done inconnu) + ResolveBlock avec réévaluation de done
    assert_eq!(
        transition_run(
            RunState::Blocked,
            RunEvent::ResolveBlock {
                resolved_guard: Some(GuardTruth::True),
                pause_requested: false
            }
        ),
        Ok(RunState::Succeeded)
    );
    assert_eq!(
        transition_run(
            RunState::Blocked,
            RunEvent::ResolveBlock {
                resolved_guard: Some(GuardTruth::False),
                pause_requested: false
            }
        ),
        Ok(RunState::Failed)
    );
    assert_eq!(
        transition_run(
            RunState::Blocked,
            RunEvent::ResolveBlock {
                resolved_guard: Some(GuardTruth::Unknown),
                pause_requested: false
            }
        ),
        Ok(RunState::Blocked)
    );

    // 10. running/blocked + DeadlineExceeded / BudgetExhausted / UnhandledFailure
    assert_eq!(
        transition_run(
            RunState::Running,
            RunEvent::DeadlineExceeded {
                activities_closed: true
            }
        ),
        Ok(RunState::Failed)
    );
    assert_eq!(
        transition_run(
            RunState::Running,
            RunEvent::DeadlineExceeded {
                activities_closed: false
            }
        ),
        Ok(RunState::Blocked)
    );

    // 11. progression diagnostique sous EffectUnknown (state-machines.md lignes 85-90)
    assert_eq!(
        transition_run(RunState::Blocked, RunEvent::ObservationRecorded),
        Ok(RunState::Blocked)
    );
    assert_eq!(
        transition_run(RunState::Blocked, RunEvent::DiagnosticAdmitted),
        Ok(RunState::Blocked)
    );
    assert_eq!(
        transition_run(
            RunState::Blocked,
            RunEvent::EffectOutcomeResolved {
                all_barriers_cleared: false,
                pause_requested: false
            }
        ),
        Ok(RunState::Blocked)
    );
    assert_eq!(
        transition_run(
            RunState::Blocked,
            RunEvent::EffectOutcomeResolved {
                all_barriers_cleared: true,
                pause_requested: false
            }
        ),
        Ok(RunState::Running)
    );
    assert_eq!(
        transition_run(
            RunState::Blocked,
            RunEvent::EffectOutcomeResolved {
                all_barriers_cleared: true,
                pause_requested: true
            }
        ),
        Ok(RunState::Paused)
    );
}

#[test]
fn test_run_terminal_immutable_invariant() {
    // INV-018, Q-03 : Un run terminal (succeeded, failed, cancelled) n'est jamais rouvert
    let terminal_states = [RunState::Succeeded, RunState::Failed, RunState::Cancelled];

    let events = vec![
        RunEvent::StartRun,
        RunEvent::PauseRun {
            activities_reconciled: true,
        },
        RunEvent::ResumeRun,
        RunEvent::CancelRun,
        RunEvent::BodyClosed {
            done: GuardTruth::True,
        },
        RunEvent::ObservationRecorded,
    ];

    for terminal in terminal_states {
        assert!(terminal.is_terminal());
        for evt in &events {
            let res = transition_run(terminal, evt.clone());
            assert!(
                matches!(res, Err(DomainTransitionError::TerminalImmutable { .. })),
                "Mutation sur état terminal {:?} aurait dû renvoyer TerminalImmutable",
                terminal
            );
        }
    }
}

#[test]
fn test_predecessor_run_validation() {
    // Q-03, state-machines.md ligne 67 : CreateRun refuse PREDECESSOR_NOT_TERMINAL tant que le prédécesseur n'est pas terminal
    for non_terminal in [
        RunState::Prepared,
        RunState::Running,
        RunState::Paused,
        RunState::Blocked,
        RunState::Cancelling,
    ] {
        assert!(
            validate_predecessor_run(non_terminal).is_err(),
            "Prédécesseur dans l'état {:?} aurait dû être rejeté",
            non_terminal
        );
    }

    for terminal in [RunState::Succeeded, RunState::Failed, RunState::Cancelled] {
        assert!(
            validate_predecessor_run(terminal).is_ok(),
            "Prédécesseur terminal {:?} aurait dû être accepté",
            terminal
        );
    }
}

#[test]
fn test_invocation_slot_ownership_exact() {
    // session-model.md section "Machine d'état de l'invocation" & INV-001
    // admitted, running, checkpointing possèdent un slot
    assert!(InvocationState::Admitted.owns_slot(false));
    assert!(InvocationState::Admitted.owns_slot(true));
    assert!(InvocationState::Running.owns_slot(false));
    assert!(InvocationState::Running.owns_slot(true));
    assert!(InvocationState::Checkpointing.owns_slot(false));
    assert!(InvocationState::Checkpointing.owns_slot(true));

    // reconciling possède un slot TANT QUE l'ancien exécuteur n'est pas neutralisé
    assert!(InvocationState::Reconciling.owns_slot(false)); // ancien exécuteur non neutralisé -> possède le slot
    assert!(!InvocationState::Reconciling.owns_slot(true)); // ancien exécuteur neutralisé -> slot libéré

    // queued, waiting_durable, completed, failed, cancelled ne possèdent JAMAIS de slot
    for state in [
        InvocationState::Queued,
        InvocationState::WaitingDurable,
        InvocationState::Completed,
        InvocationState::Failed,
        InvocationState::Cancelled,
    ] {
        assert!(!state.owns_slot(false));
        assert!(!state.owns_slot(true));
    }
}

#[test]
fn test_attempt_state_irreversibility() {
    // session-model.md :
    // Une tentative active porte l'état admitted, running, checkpointing ou reconciling.
    // Elle se termine en ended, failed ou fenced et ne redevient JAMAIS active.
    for active in [
        AttemptState::Admitted,
        AttemptState::Running,
        AttemptState::Checkpointing,
        AttemptState::Reconciling,
    ] {
        assert!(active.is_active());
        assert!(!active.is_terminal());
    }

    for terminal in [
        AttemptState::Ended,
        AttemptState::Failed,
        AttemptState::Fenced,
    ] {
        assert!(!terminal.is_active());
        assert!(terminal.is_terminal());

        // Rejet absolu de toute transition depuis un état terminal
        for target in [
            AttemptState::Admitted,
            AttemptState::Running,
            AttemptState::Checkpointing,
            AttemptState::Reconciling,
            AttemptState::Ended,
            AttemptState::Failed,
            AttemptState::Fenced,
        ] {
            let res = transition_attempt(terminal, target);
            assert!(
                matches!(res, Err(DomainTransitionError::AttemptIrreversible { .. })),
                "Tentative terminale {:?} n'aurait pas dû pouvoir transitionner vers {:?}",
                terminal,
                target
            );
        }
    }
}

#[test]
fn test_action_state_transitions_table() {
    // failure-recovery.md section "Machine d'état d'action"

    // 1. prepared -> authorizing
    assert_eq!(
        transition_action(ActionState::Prepared, ActionEvent::SubmitToBroker),
        Ok(ActionState::Authorizing)
    );

    // 2. authorizing -> authorized (GuardAllow)
    assert_eq!(
        transition_action(ActionState::Authorizing, ActionEvent::GuardAllow),
        Ok(ActionState::Authorized)
    );

    // 3. authorizing -> denied (GuardDeny)
    assert_eq!(
        transition_action(
            ActionState::Authorizing,
            ActionEvent::GuardDeny {
                reason: "policy".into()
            }
        ),
        Ok(ActionState::Denied)
    );

    // 4. authorizing -> approval_required (GuardRequireApproval)
    assert_eq!(
        transition_action(ActionState::Authorizing, ActionEvent::GuardRequireApproval),
        Ok(ActionState::ApprovalRequired)
    );

    // 5. authorizing + GuardUnavailable sous guard_required : fail-closed
    assert!(matches!(
        transition_action(
            ActionState::Authorizing,
            ActionEvent::GuardUnavailable {
                mode: GuardMode::GuardRequired,
                fallback_decision: None
            }
        ),
        Err(DomainTransitionError::GuardUnavailableFailClosed)
    ));

    // 6. authorizing + GuardUnavailable sous guard_optional : bascule sur fallback locale
    assert_eq!(
        transition_action(
            ActionState::Authorizing,
            ActionEvent::GuardUnavailable {
                mode: GuardMode::GuardOptional,
                fallback_decision: Some(true)
            }
        ),
        Ok(ActionState::Authorized)
    );
    assert_eq!(
        transition_action(
            ActionState::Authorizing,
            ActionEvent::GuardUnavailable {
                mode: GuardMode::GuardOptional,
                fallback_decision: Some(false)
            }
        ),
        Ok(ActionState::Denied)
    );

    // 7. approval_required + HumanApprovalDecision
    assert_eq!(
        transition_action(
            ActionState::ApprovalRequired,
            ActionEvent::HumanApprovalDecision { approved: true }
        ),
        Ok(ActionState::Authorized)
    );
    assert_eq!(
        transition_action(
            ActionState::ApprovalRequired,
            ActionEvent::HumanApprovalDecision { approved: false }
        ),
        Ok(ActionState::Denied)
    );

    // 8. authorized + ReevaluateAuthorized
    assert_eq!(
        transition_action(
            ActionState::Authorized,
            ActionEvent::ReevaluateAuthorized { revoked: false }
        ),
        Ok(ActionState::Authorizing)
    );
    assert_eq!(
        transition_action(
            ActionState::Authorized,
            ActionEvent::ReevaluateAuthorized { revoked: true }
        ),
        Ok(ActionState::Denied)
    );

    // 9. authorized + CommitDispatchMarker -> dispatching
    assert_eq!(
        transition_action(ActionState::Authorized, ActionEvent::CommitDispatchMarker),
        Ok(ActionState::Dispatching)
    );

    // 10. dispatching + ConfirmDispatchDeparture -> in_flight
    assert_eq!(
        transition_action(
            ActionState::Dispatching,
            ActionEvent::ConfirmDispatchDeparture
        ),
        Ok(ActionState::InFlight)
    );

    // 11. dispatching / in_flight + ObserveResult
    assert_eq!(
        transition_action(
            ActionState::Dispatching,
            ActionEvent::ObserveResult { success: true }
        ),
        Ok(ActionState::ObservedSuccess)
    );
    assert_eq!(
        transition_action(
            ActionState::InFlight,
            ActionEvent::ObserveResult { success: false }
        ),
        Ok(ActionState::ObservedFailure)
    );

    // 12. dispatching / in_flight + ObserveDepartureImpossibleToExclude -> outcome_unknown
    assert_eq!(
        transition_action(
            ActionState::Dispatching,
            ActionEvent::ObserveDepartureImpossibleToExclude
        ),
        Ok(ActionState::OutcomeUnknown)
    );
    assert_eq!(
        transition_action(
            ActionState::InFlight,
            ActionEvent::ObserveDepartureImpossibleToExclude
        ),
        Ok(ActionState::OutcomeUnknown)
    );

    // 13. Réconciliation depuis OutcomeUnknown
    assert_eq!(
        transition_action(
            ActionState::OutcomeUnknown,
            ActionEvent::ApplyReconciliation(ReconcileOutcome::ConfirmedSuccess)
        ),
        Ok(ActionState::ObservedSuccess)
    );
    assert_eq!(
        transition_action(
            ActionState::OutcomeUnknown,
            ActionEvent::ApplyReconciliation(ReconcileOutcome::ConfirmedFailureNoEffect)
        ),
        Ok(ActionState::ObservedFailure)
    );
    assert_eq!(
        transition_action(
            ActionState::OutcomeUnknown,
            ActionEvent::ApplyReconciliation(ReconcileOutcome::StillRunning)
        ),
        Ok(ActionState::InFlight)
    );
    assert_eq!(
        transition_action(
            ActionState::OutcomeUnknown,
            ActionEvent::ApplyReconciliation(ReconcileOutcome::Conflict)
        ),
        Ok(ActionState::OutcomeUnknown)
    );
    assert_eq!(
        transition_action(
            ActionState::OutcomeUnknown,
            ActionEvent::ApplyReconciliation(ReconcileOutcome::Unknown)
        ),
        Ok(ActionState::OutcomeUnknown)
    );

    // 14. INTERDICTION ABSOLUE de retry direct depuis OutcomeUnknown (INV-012, C4/C5)
    assert!(matches!(
        transition_action(
            ActionState::OutcomeUnknown,
            ActionEvent::CommitDispatchMarker
        ),
        Err(DomainTransitionError::CannotRetryOutcomeUnknown)
    ));
    assert!(matches!(
        transition_action(
            ActionState::OutcomeUnknown,
            ActionEvent::ConfirmDispatchDeparture
        ),
        Err(DomainTransitionError::CannotRetryOutcomeUnknown)
    ));
    assert!(matches!(
        transition_action(
            ActionState::OutcomeUnknown,
            ActionEvent::RetryAfterObservedFailure {
                strategy_allows_retry: true,
                authorization_still_valid: true
            }
        ),
        Err(DomainTransitionError::CannotRetryOutcomeUnknown)
    ));

    // 15. Retry depuis ObservedFailure si la stratégie le permet
    assert_eq!(
        transition_action(
            ActionState::ObservedFailure,
            ActionEvent::RetryAfterObservedFailure {
                strategy_allows_retry: true,
                authorization_still_valid: true
            }
        ),
        Ok(ActionState::Dispatching)
    );
    assert_eq!(
        transition_action(
            ActionState::ObservedFailure,
            ActionEvent::RetryAfterObservedFailure {
                strategy_allows_retry: true,
                authorization_still_valid: false
            }
        ),
        Ok(ActionState::Authorizing)
    );
    assert!(
        transition_action(
            ActionState::ObservedFailure,
            ActionEvent::RetryAfterObservedFailure {
                strategy_allows_retry: false,
                authorization_still_valid: true
            }
        )
        .is_err()
    );

    // 16. Annulation
    assert_eq!(
        transition_action(ActionState::Prepared, ActionEvent::CancelAction),
        Ok(ActionState::Cancelled)
    );
    assert_eq!(
        transition_action(ActionState::Authorizing, ActionEvent::CancelAction),
        Ok(ActionState::Cancelled)
    );
    assert_eq!(
        transition_action(ActionState::Dispatching, ActionEvent::CancelAction),
        Ok(ActionState::CancelRequested)
    );
    assert_eq!(
        transition_action(ActionState::InFlight, ActionEvent::CancelAction),
        Ok(ActionState::CancelRequested)
    );
}

#[test]
fn test_mission_and_completion_policy_forbidden() {
    // state-machines.md ligne 50 : si agent, jugement N+1/N+2, jamais LLM forcé sous forbidden
    let policy_mech = CompletionPolicy::Mechanical;
    let policy_human = CompletionPolicy::Human;
    let policy_agent = CompletionPolicy::OwnerAgent;

    assert!(
        policy_mech
            .validate_with_model_policy(ModelPolicy::Forbidden)
            .is_ok()
    );
    assert!(
        policy_human
            .validate_with_model_policy(ModelPolicy::Forbidden)
            .is_ok()
    );

    // OwnerAgent est STRICTEMENT REFUSÉ sous ModelPolicy::Forbidden
    assert!(matches!(
        policy_agent.validate_with_model_policy(ModelPolicy::Forbidden),
        Err(DomainTransitionError::PolicyViolation { .. })
    ));
    assert!(
        policy_agent
            .validate_with_model_policy(ModelPolicy::AllowedBounded)
            .is_ok()
    );
}

#[test]
fn test_action_origin_xor_strict() {
    let inv_id = InvocationId::new_v4();
    let att_id = AttemptId::new_v4();
    let origin_inv = ActionOrigin::Invocation {
        invocation_id: inv_id,
        attempt_id: att_id,
    };
    let json_inv = serde_json::to_string(&origin_inv).unwrap();
    assert!(json_inv.contains("invocation"));

    let rep_id = ReplayWorkerId::new_v4();
    let origin_rep = ActionOrigin::Replay {
        replay_worker_id: rep_id,
    };
    let json_rep = serde_json::to_string(&origin_rep).unwrap();
    assert!(json_rep.contains("replay"));

    let act_id = ActorId::new("actor-123").unwrap();
    let origin_ctrl = ActionOrigin::ControlPlane {
        control_principal_id: act_id,
    };
    let json_ctrl = serde_json::to_string(&origin_ctrl).unwrap();
    assert!(json_ctrl.contains("control_plane"));
}

#[test]
fn test_secret_ref_redaction() {
    let secret = SecretRef::new("super-secret-token-key-12345").unwrap();
    let debug_str = format!("{:?}", secret);
    let display_str = format!("{}", secret);

    assert_eq!(debug_str, "SecretRef([REDACTED])");
    assert_eq!(display_str, "SecretRef([REDACTED])");
    assert!(!debug_str.contains("12345"));
    assert!(!display_str.contains("12345"));
}

#[test]
fn test_sha256_digest_strict_validation() {
    let raw_hex = "a0b1c2d3e4f50123456789abcdef0123456789abcdef0123456789abcdef0123";
    let prefixed = format!("sha256:{}", raw_hex);

    // 1. Parse depuis raw hex
    let d1: Sha256Digest = raw_hex.parse().unwrap();
    assert_eq!(d1.to_string(), prefixed);
    assert_eq!(d1.hex(), raw_hex);

    // 2. Parse depuis sha256:<hex>
    let d2: Sha256Digest = prefixed.parse().unwrap();
    assert_eq!(d1, d2);

    // 3. Désérialisation JSON depuis chaîne
    let json_str = format!("\"{}\"", prefixed);
    let d3: Sha256Digest = serde_json::from_str(&json_str).unwrap();
    assert_eq!(d1, d3);

    let json_raw = format!("\"{}\"", raw_hex);
    let d4: Sha256Digest = serde_json::from_str(&json_raw).unwrap();
    assert_eq!(d1, d4);

    // 4. Désérialisation JSON depuis objet { algorithm: "sha256", hex: "..." }
    let json_map = format!(r#"{{ "algorithm": "sha256", "hex": "{}" }}"#, raw_hex);
    let d5: Sha256Digest = serde_json::from_str(&json_map).unwrap();
    assert_eq!(d1, d5);

    // 5. Rejets stricts
    // Majuscules interdites
    let uppercase_hex = raw_hex.to_uppercase();
    assert!(Sha256Digest::from_str(&uppercase_hex).is_err());

    // Longueur != 64
    assert!(Sha256Digest::from_str("a0b1c2").is_err());

    // Caractères non-hex
    let invalid_chars = "z0b1c2d3e4f50123456789abcdef0123456789abcdef0123456789abcdef0123";
    assert!(Sha256Digest::from_str(invalid_chars).is_err());

    // Objet avec mauvais algorithme
    let invalid_algo = format!(r#"{{ "algorithm": "sha512", "hex": "{}" }}"#, raw_hex);
    let res: Result<Sha256Digest, _> = serde_json::from_str(&invalid_algo);
    assert!(res.is_err());
}
