// SPDX-License-Identifier: Apache-2.0
//! Double déterministe de [`GuardPort`], minuteur asynchrone sans `sleep` sur [`ManualClock`],
//! et espions indépendants vérifiant zéro dispatch et zéro résolution de secret sur refus
//! (`F017`, `T-F017-001`..`T-F017-011`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use secrecy::SecretString;
use unleash_action_broker::{
    ActionBrokerError, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort,
    ActionIntent, AuthorizationDecision, AuthorizedSecretAccess, BoxFuture, EffectsClass,
    GUARD_PORT_PROTOCOL_VERSION, GuardAuditReference, GuardCapabilities, GuardConstraint,
    GuardDecisionRef, GuardEvaluateRequest, GuardEvaluationResponse, GuardExecuteRequest,
    GuardExecutionOutcome, GuardHealthReport, GuardHealthStatus, GuardMappingProfile, GuardPort,
    GuardPortError, GuardRemotePolicySummary, GuardRevocationReceipt, GuardSubjectActionRequest,
    GuardSuspensionReceipt, InternalObligation, IsolationBackendStatus, IsolationCapability,
    IsolationMode, IsolationProbePort, IsolationRequirements, MonotonicClockPort,
    NegotiateCapabilitiesRequest, ObligationCapabilityPort, ResolvedSecret, SecretStore,
    SecretStoreError, SleepPort,
};
use unleash_domain::{
    ActionId, ApprovalScopeId, ClockPort, DispatchId, ProjectId, Revision, SecretRef, Sha256Digest,
    TimestampMs,
};
use zeroize::Zeroize;

use crate::clock::{ClockWakeup, ManualClock};

impl MonotonicClockPort for ManualClock {
    fn monotonic_now(&self) -> Duration {
        self.monotonic_now()
    }
}

/// Adaptateur asynchrone de [`SleepPort`] piloté par [`ManualClock`] sans aucun `sleep` réel.
#[derive(Debug, Clone)]
pub struct ManualClockTimer {
    clock: ManualClock,
}

impl ManualClockTimer {
    /// Construit un minuteur déterministe rattaché à `clock`.
    #[must_use]
    pub const fn new(clock: ManualClock) -> Self {
        Self { clock }
    }
}

struct ManualSleepFuture {
    wakeup: ClockWakeup,
    clock: ManualClock,
}

impl Drop for ManualSleepFuture {
    fn drop(&mut self) {
        self.clock.cancel_wakeup(&self.wakeup);
    }
}

impl Future for ManualSleepFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.wakeup.is_triggered() {
            Poll::Ready(())
        } else {
            self.wakeup.set_waker(cx.waker().clone());
            if self.wakeup.is_triggered() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }
}

impl SleepPort for ManualClockTimer {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        let wakeup = self.clock.register_monotonic_wakeup(duration);
        Box::pin(ManualSleepFuture {
            wakeup,
            clock: self.clock.clone(),
        })
    }
}

/// Issue scriptée retournée par [`DeterministicGuardDouble`].
#[derive(Debug, Clone)]
pub enum ScriptedGuardOutcome {
    /// Retourne `Allow` avec `expiry = now + offset` et contraintes éventuelles.
    Allow {
        /// Décalage en millisecondes par rapport à l'horloge UTC courante (`None` = sans expiry).
        expiry_offset_ms: Option<i64>,
        /// Contraintes additionnelles imposées par AI-Guard.
        constraints: Vec<GuardConstraint>,
    },
    /// Retourne un refus formel `Deny`.
    Deny {
        /// Code de refus.
        reason_code: String,
        /// Détail du refus.
        detail: String,
    },
    /// Demande une approbation humaine sur `approval_scope`.
    RequireApproval {
        /// Périmètre d'approbation corrélé.
        approval_scope: ApprovalScopeId,
        /// Motif de la demande.
        reason: String,
    },
    /// Réponse `Unavailable`.
    Unavailable {
        /// Motif d'indisponibilité.
        reason: String,
    },
    /// Retourne directement `GuardPortError::Timeout`.
    TimeoutError,
    /// Bloque indéfiniment jusqu'à annulation par le timeout du broker (sans `sleep`).
    HangUntilCancelled,
    /// Retourne `GuardPortError::MalformedResponse`.
    Malformed {
        /// Détail de malformation.
        detail: String,
    },
    /// Retourne `UnsupportedCapability`.
    UnsupportedCapability {
        /// Capacité manquante.
        capability: String,
    },
    /// Retourne `GuardPortError::IncompatibleProtocol`.
    IncompatibleProtocol {
        /// Version attendue.
        expected: String,
        /// Version observée.
        observed: String,
    },
}

struct DoubleState {
    capabilities: GuardCapabilities,
    health: GuardHealthStatus,
    default_outcome: ScriptedGuardOutcome,
    queued_outcomes: VecDeque<ScriptedGuardOutcome>,
    verb_outcomes: BTreeMap<String, ScriptedGuardOutcome>,
    fail_revoke: bool,
    fail_suspend: bool,
    fail_audit_read: bool,
    recorded_evaluates: Vec<GuardEvaluateRequest>,
    recorded_revocations: Vec<GuardSubjectActionRequest>,
    recorded_suspensions: Vec<GuardSubjectActionRequest>,
    decision_seq: u64,
}

/// Double déterministe injectable implémentant [`GuardPort`] (`unleash-testkit`).
///
/// Ne constitue pas une recette d'intégration réelle AI-Guard (`EXT-GUARD`, `AC-F017-012`).
pub struct DeterministicGuardDouble {
    clock: Arc<dyn ClockPort>,
    negotiate_calls: AtomicU64,
    health_calls: AtomicU64,
    evaluate_calls: AtomicU64,
    revoke_calls: AtomicU64,
    suspend_calls: AtomicU64,
    resume_calls: AtomicU64,
    audit_calls: AtomicU64,
    execute_calls: AtomicU64,
    hang_notify: Arc<tokio::sync::Notify>,
    state: Mutex<DoubleState>,
}

impl DeterministicGuardDouble {
    /// Crée un double avec les capacités exactement fournies et `Allow` par défaut (TTL 60 s).
    #[must_use]
    pub fn with_capabilities(clock: Arc<dyn ClockPort>, capabilities: GuardCapabilities) -> Self {
        Self {
            clock,
            negotiate_calls: AtomicU64::new(0),
            health_calls: AtomicU64::new(0),
            evaluate_calls: AtomicU64::new(0),
            revoke_calls: AtomicU64::new(0),
            suspend_calls: AtomicU64::new(0),
            resume_calls: AtomicU64::new(0),
            audit_calls: AtomicU64::new(0),
            execute_calls: AtomicU64::new(0),
            hang_notify: Arc::new(tokio::sync::Notify::new()),
            state: Mutex::new(DoubleState {
                capabilities,
                health: GuardHealthStatus::Healthy,
                default_outcome: ScriptedGuardOutcome::Allow {
                    expiry_offset_ms: Some(60_000),
                    constraints: Vec::new(),
                },
                queued_outcomes: VecDeque::new(),
                verb_outcomes: BTreeMap::new(),
                fail_revoke: false,
                fail_suspend: false,
                fail_audit_read: false,
                recorded_evaluates: Vec::new(),
                recorded_revocations: Vec::new(),
                recorded_suspensions: Vec::new(),
                decision_seq: 0,
            }),
        }
    }

    /// Crée un double configuré avec un profil de mapping donné (`NoneAccepted` ou `Qualified`).
    #[must_use]
    pub fn with_profile(clock: Arc<dyn ClockPort>, profile: GuardMappingProfile) -> Self {
        let (supports_remote_approval, supported_obligations) = match &profile {
            GuardMappingProfile::NoneAccepted => (false, BTreeSet::new()),
            GuardMappingProfile::Qualified {
                supported_obligations,
                supports_remote_approval,
                ..
            } => (*supports_remote_approval, supported_obligations.clone()),
        };
        Self::with_capabilities(
            clock,
            GuardCapabilities {
                protocol_version: String::from(GUARD_PORT_PROTOCOL_VERSION),
                mapping_profile: profile,
                supports_suspend_resume: false,
                supports_execute_authorized_action: false,
                supports_remote_approval,
                supports_remote_policy_write: false,
                supported_obligations,
            },
        )
    }

    /// Crée un double autorisant inconditionnellement sans contrainte additionnelle.
    #[must_use]
    pub fn allowing_unconditional(clock: Arc<dyn ClockPort>) -> Self {
        Self::with_profile(clock, GuardMappingProfile::NoneAccepted)
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, DoubleState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Empile une issue FIFO consommée par les prochains `evaluate_action`.
    pub fn push_outcome(&self, outcome: ScriptedGuardOutcome) {
        self.lock_state().queued_outcomes.push_back(outcome);
    }

    /// Définit l'issue par défaut lorsque la file FIFO est vide.
    pub fn set_default_outcome(&self, outcome: ScriptedGuardOutcome) {
        self.lock_state().default_outcome = outcome;
    }

    /// Définit une issue spécifique pour un verbe exact.
    pub fn set_outcome_for_verb(&self, verb: impl Into<String>, outcome: ScriptedGuardOutcome) {
        self.lock_state().verb_outcomes.insert(verb.into(), outcome);
    }

    /// Met à jour le profil de mapping annoncé par `negotiate_capabilities`.
    pub fn set_profile(&self, profile: GuardMappingProfile) {
        let mut st = self.lock_state();
        st.capabilities.mapping_profile = profile;
    }

    /// Configure le support d'approbation humaine distante.
    pub fn set_supports_remote_approval(&self, supported: bool) {
        self.lock_state().capabilities.supports_remote_approval = supported;
    }

    /// Configure l'état de santé retourné par `check_health`.
    pub fn set_health_status(&self, status: GuardHealthStatus) {
        self.lock_state().health = status;
    }

    /// Force `revoke_subject` à échouer pour tester les pannes pendant révocation.
    pub fn set_fail_revoke(&self, fail: bool) {
        self.lock_state().fail_revoke = fail;
    }

    /// Force `suspend_subject` à échouer.
    pub fn set_fail_suspend(&self, fail: bool) {
        self.lock_state().fail_suspend = fail;
    }

    /// Force `read_audit_reference` à échouer (journal distant indisponible).
    pub fn set_fail_audit_read(&self, fail: bool) {
        self.lock_state().fail_audit_read = fail;
    }

    /// Nombre total d'appels à `evaluate_action`.
    #[must_use]
    pub fn evaluate_call_count(&self) -> u64 {
        self.evaluate_calls.load(Ordering::SeqCst)
    }

    /// Nombre total d'appels à `negotiate_capabilities`.
    #[must_use]
    pub fn negotiate_call_count(&self) -> u64 {
        self.negotiate_calls.load(Ordering::SeqCst)
    }

    /// Nombre total d'appels à `check_health`.
    #[must_use]
    pub fn health_call_count(&self) -> u64 {
        self.health_calls.load(Ordering::SeqCst)
    }

    /// Nombre total d'appels à `revoke_subject`.
    #[must_use]
    pub fn revoke_call_count(&self) -> u64 {
        self.revoke_calls.load(Ordering::SeqCst)
    }

    /// Nombre total d'appels à `suspend_subject`.
    #[must_use]
    pub fn suspend_call_count(&self) -> u64 {
        self.suspend_calls.load(Ordering::SeqCst)
    }

    /// Nombre total d'appels à `resume_subject`.
    #[must_use]
    pub fn resume_call_count(&self) -> u64 {
        self.resume_calls.load(Ordering::SeqCst)
    }

    /// Nombre total d'appels à `read_audit_reference`.
    #[must_use]
    pub fn audit_call_count(&self) -> u64 {
        self.audit_calls.load(Ordering::SeqCst)
    }

    /// Nombre total d'appels à `execute_authorized_action`.
    #[must_use]
    pub fn execute_call_count(&self) -> u64 {
        self.execute_calls.load(Ordering::SeqCst)
    }

    /// Liste des requêtes `evaluate_action` observées.
    #[must_use]
    pub fn recorded_evaluate_requests(&self) -> Vec<GuardEvaluateRequest> {
        self.lock_state().recorded_evaluates.clone()
    }
}

impl GuardPort for DeterministicGuardDouble {
    fn protocol_version(&self) -> &'static str {
        GUARD_PORT_PROTOCOL_VERSION
    }

    fn negotiate_capabilities(
        &self,
        _request: NegotiateCapabilitiesRequest,
    ) -> BoxFuture<'_, Result<GuardCapabilities, GuardPortError>> {
        self.negotiate_calls.fetch_add(1, Ordering::SeqCst);
        let caps = self.lock_state().capabilities.clone();
        Box::pin(async move { Ok(caps) })
    }

    fn check_health(&self) -> BoxFuture<'_, Result<GuardHealthReport, GuardPortError>> {
        self.health_calls.fetch_add(1, Ordering::SeqCst);
        let (status, protocol_version) = {
            let st = self.lock_state();
            (st.health, st.capabilities.protocol_version.clone())
        };
        let checked_at_ms = self.clock.now_ms();
        Box::pin(async move {
            Ok(GuardHealthReport {
                status,
                protocol_version,
                policy_revision: Some(Revision::new(1)),
                checked_at_ms,
                detail: None,
            })
        })
    }

    fn evaluate_action<'a>(
        &'a self,
        request: &'a GuardEvaluateRequest,
    ) -> BoxFuture<'a, Result<GuardEvaluationResponse, GuardPortError>> {
        self.evaluate_calls.fetch_add(1, Ordering::SeqCst);
        let (outcome, seq) = {
            let mut st = self.lock_state();
            st.recorded_evaluates.push(request.clone());
            st.decision_seq = st.decision_seq.saturating_add(1);
            let next_outcome = if let Some(q) = st.queued_outcomes.pop_front() {
                q
            } else if let Some(v) = st.verb_outcomes.get(&request.verb).cloned() {
                v
            } else {
                st.default_outcome.clone()
            };
            (next_outcome, st.decision_seq)
        };
        let now = self.clock.now_ms();
        let hang_notify = Arc::clone(&self.hang_notify);

        Box::pin(async move {
            match outcome {
                ScriptedGuardOutcome::Allow {
                    expiry_offset_ms,
                    constraints,
                } => {
                    let decision_ref =
                        GuardDecisionRef::new(format!("guard-dec-{seq}")).map_err(|e| {
                            GuardPortError::MalformedResponse {
                                detail: e.to_string(),
                            }
                        })?;
                    let expiry = expiry_offset_ms
                        .map(|off| TimestampMs::new(now.as_i64().saturating_add(off)));
                    Ok(GuardEvaluationResponse::Allow {
                        decision_ref,
                        constraints,
                        expiry,
                    })
                }
                ScriptedGuardOutcome::Deny {
                    reason_code,
                    detail,
                } => {
                    let decision_ref =
                        GuardDecisionRef::new(format!("guard-deny-{seq}")).map_err(|e| {
                            GuardPortError::MalformedResponse {
                                detail: e.to_string(),
                            }
                        })?;
                    Ok(GuardEvaluationResponse::Deny {
                        reason_code,
                        reason_detail: detail,
                        decision_ref,
                    })
                }
                ScriptedGuardOutcome::RequireApproval {
                    approval_scope,
                    reason,
                } => {
                    let decision_ref =
                        GuardDecisionRef::new(format!("guard-app-{seq}")).map_err(|e| {
                            GuardPortError::MalformedResponse {
                                detail: e.to_string(),
                            }
                        })?;
                    Ok(GuardEvaluationResponse::RequireApproval {
                        approval_scope,
                        decision_ref,
                        reason,
                    })
                }
                ScriptedGuardOutcome::Unavailable { reason } => {
                    Ok(GuardEvaluationResponse::Unavailable { reason })
                }
                ScriptedGuardOutcome::TimeoutError => {
                    Err(GuardPortError::Timeout { timeout_ms: 2_000 })
                }
                ScriptedGuardOutcome::HangUntilCancelled => {
                    hang_notify.notified().await;
                    Err(GuardPortError::Unavailable {
                        reason: String::from("interrupted"),
                    })
                }
                ScriptedGuardOutcome::Malformed { detail } => {
                    Err(GuardPortError::MalformedResponse { detail })
                }
                ScriptedGuardOutcome::UnsupportedCapability { capability } => {
                    Ok(GuardEvaluationResponse::UnsupportedCapability {
                        capability,
                        detail: String::from("capacité non prise en charge par le double"),
                    })
                }
                ScriptedGuardOutcome::IncompatibleProtocol { expected, observed } => {
                    Err(GuardPortError::IncompatibleProtocol { expected, observed })
                }
            }
        })
    }

    fn revoke_subject<'a>(
        &'a self,
        request: &'a GuardSubjectActionRequest,
    ) -> BoxFuture<'a, Result<GuardRevocationReceipt, GuardPortError>> {
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        let fail = {
            let mut st = self.lock_state();
            st.recorded_revocations.push(request.clone());
            st.fail_revoke
        };
        let now = self.clock.now_ms();
        Box::pin(async move {
            if fail {
                return Err(GuardPortError::Unavailable {
                    reason: String::from("panne simulée lors de RevokeSubject"),
                });
            }
            Ok(GuardRevocationReceipt {
                remote_ref: format!("guard-revoke:{}", request.subject_key),
                revoked_at_ms: now,
            })
        })
    }

    fn read_policy_summary(
        &self,
        _project_id: Option<ProjectId>,
    ) -> BoxFuture<'_, Result<GuardRemotePolicySummary, GuardPortError>> {
        Box::pin(async {
            Ok(GuardRemotePolicySummary {
                policy_revision: Revision::new(1),
                remote_write_supported: false,
                external_admin_url: None,
                summary: String::from("DeterministicGuardDouble read-only policy"),
            })
        })
    }

    fn read_audit_reference<'a>(
        &'a self,
        action_id: ActionId,
        _decision_ref: Option<&'a GuardDecisionRef>,
    ) -> BoxFuture<'a, Result<GuardAuditReference, GuardPortError>> {
        self.audit_calls.fetch_add(1, Ordering::SeqCst);
        let fail = self.lock_state().fail_audit_read;
        let now = self.clock.now_ms();
        Box::pin(async move {
            if fail {
                return Err(GuardPortError::Unavailable {
                    reason: String::from("journal d'audit distant indisponible"),
                });
            }
            Ok(GuardAuditReference {
                audit_id: format!("guard-audit:{action_id}"),
                status: String::from("recorded"),
                observed_at_ms: now,
            })
        })
    }

    fn suspend_subject<'a>(
        &'a self,
        request: &'a GuardSubjectActionRequest,
    ) -> BoxFuture<'a, Result<GuardSuspensionReceipt, GuardPortError>> {
        self.suspend_calls.fetch_add(1, Ordering::SeqCst);
        let (supported, fail) = {
            let mut st = self.lock_state();
            st.recorded_suspensions.push(request.clone());
            (st.capabilities.supports_suspend_resume, st.fail_suspend)
        };
        let now = self.clock.now_ms();
        Box::pin(async move {
            if !supported {
                return Err(GuardPortError::UnsupportedCapability {
                    capability: String::from("SuspendSubject"),
                    protocol_version: Some(String::from(GUARD_PORT_PROTOCOL_VERSION)),
                });
            }
            if fail {
                return Err(GuardPortError::Unavailable {
                    reason: String::from("panne simulée lors de SuspendSubject"),
                });
            }
            Ok(GuardSuspensionReceipt {
                remote_ref: format!("guard-suspend:{}", request.subject_key),
                recorded_at_ms: now,
            })
        })
    }

    fn resume_subject<'a>(
        &'a self,
        request: &'a GuardSubjectActionRequest,
    ) -> BoxFuture<'a, Result<GuardSuspensionReceipt, GuardPortError>> {
        self.resume_calls.fetch_add(1, Ordering::SeqCst);
        let (supported, fail) = {
            let st = self.lock_state();
            (st.capabilities.supports_suspend_resume, st.fail_suspend)
        };
        let now = self.clock.now_ms();
        Box::pin(async move {
            if !supported {
                return Err(GuardPortError::UnsupportedCapability {
                    capability: String::from("ResumeSubject"),
                    protocol_version: Some(String::from(GUARD_PORT_PROTOCOL_VERSION)),
                });
            }
            if fail {
                return Err(GuardPortError::Unavailable {
                    reason: String::from("panne simulée lors de ResumeSubject"),
                });
            }
            Ok(GuardSuspensionReceipt {
                remote_ref: format!("guard-resume:{}", request.subject_key),
                recorded_at_ms: now,
            })
        })
    }

    fn execute_authorized_action<'a>(
        &'a self,
        request: &'a GuardExecuteRequest,
    ) -> BoxFuture<'a, Result<GuardExecutionOutcome, GuardPortError>> {
        self.execute_calls.fetch_add(1, Ordering::SeqCst);
        let supported = self
            .lock_state()
            .capabilities
            .supports_execute_authorized_action;
        let now = self.clock.now_ms();
        Box::pin(async move {
            if !supported {
                return Err(GuardPortError::UnsupportedCapability {
                    capability: String::from("ExecuteAuthorizedAction"),
                    protocol_version: Some(String::from(GUARD_PORT_PROTOCOL_VERSION)),
                });
            }
            Ok(GuardExecutionOutcome {
                execution_ref: format!("guard-exec:{}", request.action_id),
                succeeded: true,
                output_digest: request.payload_digest,
                observed_at_ms: now,
            })
        })
    }
}

/// Échec d'exécution simulé injectable dans [`BrokerExecutionSpy`].
#[derive(Debug, Clone)]
pub enum SimulatedExecutionFailure {
    /// Échec confirmé sans effet externe résiduel.
    ConfirmedFailureNoEffect {
        /// Motif simulé.
        reason: String,
    },
    /// Coupure ou crash après envoi : issue impossible à exclure (`OutcomeUnknown`).
    OutcomeUnknown {
        /// Motif d'ambiguïté.
        reason: String,
    },
}

/// Enregistrement d'un dispatch réellement observé par l'espion indépendant [`BrokerExecutionSpy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchedActionRecord {
    /// Action exécutée.
    pub action_id: ActionId,
    /// Identifiant du dispatch.
    pub dispatch_id: DispatchId,
    /// Verbe exécuté.
    pub verb: String,
    /// Classe d'effet.
    pub effects_class: EffectsClass,
    /// Référence de décision corrélée.
    pub decision_ref: GuardDecisionRef,
    /// Surface sérialisée expurgée du dispatch (sans valeurs secrètes).
    pub serialized_surface: String,
    /// Nombre de secrets résolus livrés au transport.
    pub resolved_secret_count: usize,
    /// Horodatage UTC d'observation.
    pub observed_at_ms: TimestampMs,
}

struct ExecutionSpyState {
    supported_obligations: BTreeMap<String, bool>,
    next_failure: Option<SimulatedExecutionFailure>,
    dispatches: Vec<DispatchedActionRecord>,
}

/// Espion indépendant du mapper et du broker mesurant dispatches réels et capacités
/// d'obligations (`AC-F017-005/008/010`, `T-F017-005`).
pub struct BrokerExecutionSpy {
    dispatch_calls: AtomicU64,
    state: Mutex<ExecutionSpyState>,
}

impl Default for BrokerExecutionSpy {
    fn default() -> Self {
        Self::new()
    }
}

impl BrokerExecutionSpy {
    /// Crée un espion vierge avec 0 dispatch et aucune obligation supportée par défaut.
    #[must_use]
    pub fn new() -> Self {
        Self {
            dispatch_calls: AtomicU64::new(0),
            state: Mutex::new(ExecutionSpyState {
                supported_obligations: BTreeMap::new(),
                next_failure: None,
                dispatches: Vec::new(),
            }),
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, ExecutionSpyState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Définit si une capacité d'obligation est actuellement imposable.
    pub fn set_obligation_supported(&self, capability: &str, supported: bool) {
        self.lock_state()
            .supported_obligations
            .insert(capability.to_owned(), supported);
    }

    /// Programme un échec ou `OutcomeUnknown` pour le prochain dispatch.
    pub fn set_next_execution_failure(&self, failure: SimulatedExecutionFailure) {
        self.lock_state().next_failure = Some(failure);
    }

    /// Nombre total de dispatches physiques réellement engagés.
    #[must_use]
    pub fn dispatch_count(&self) -> u64 {
        self.dispatch_calls.load(Ordering::SeqCst)
    }

    /// Copie des dispatches enregistrés.
    #[must_use]
    pub fn dispatches(&self) -> Vec<DispatchedActionRecord> {
        self.lock_state().dispatches.clone()
    }

    /// Vérifie qu'aucun dispatch n'a été exécuté.
    ///
    /// # Panics
    /// Panique en test si au moins un dispatch a été enregistré.
    pub fn assert_zero_dispatches(&self) {
        let count = self.dispatch_count();
        assert_eq!(
            count, 0,
            "Zéro dispatch attendu, {count} dispatch(s) observé(s)"
        );
    }

    /// Vérifie que la chaîne `canary` n'apparaît dans aucune surface de dispatch sérialisée.
    ///
    /// # Panics
    /// Panique en test si le canari apparaît dans un payload/log de dispatch.
    pub fn assert_no_canary_in_dispatches(&self, canary: &str) {
        for rec in &self.lock_state().dispatches {
            assert!(
                !rec.serialized_surface.contains(canary),
                "Fuite du canari détectée dans la surface du dispatch {}",
                rec.dispatch_id
            );
        }
    }
}

impl ObligationCapabilityPort for BrokerExecutionSpy {
    fn is_obligation_enforceable(&self, obligation: &InternalObligation) -> bool {
        self.lock_state()
            .supported_obligations
            .get(&obligation.required_capability)
            .copied()
            .unwrap_or(false)
    }
}

impl ActionExecutorPort for BrokerExecutionSpy {
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        intent: &'a ActionIntent,
        decision: &'a AuthorizationDecision,
        resolved_secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        self.dispatch_calls.fetch_add(1, Ordering::SeqCst);
        let mut st = self.lock_state();
        let failure = st.next_failure.take();
        let observed_at_ms = intent.action_deadline;

        let serialized_surface = serde_json::json!({
            "action_id": intent.action_id,
            "verb": intent.verb,
            "effects_class": intent.effects_class,
            "decision_ref": decision.decision_ref,
            "secret_count": resolved_secrets.len(),
        })
        .to_string();

        st.dispatches.push(DispatchedActionRecord {
            action_id: intent.action_id,
            dispatch_id,
            verb: intent.verb.clone(),
            effects_class: intent.effects_class,
            decision_ref: decision.decision_ref.clone(),
            serialized_surface,
            resolved_secret_count: resolved_secrets.len(),
            observed_at_ms,
        });
        drop(st);

        Box::pin(async move {
            match failure {
                Some(SimulatedExecutionFailure::ConfirmedFailureNoEffect { reason }) => {
                    Err(ActionExecutionFailure::ConfirmedFailureNoEffect {
                        dispatch_id,
                        reason,
                    })
                }
                Some(SimulatedExecutionFailure::OutcomeUnknown { reason }) => {
                    Err(ActionExecutionFailure::OutcomeUnknown {
                        dispatch_id,
                        reason,
                    })
                }
                None => Ok(ActionExecutionResult {
                    dispatch_id,
                    provider_request_id: Some(format!("spy-req-{}", intent.action_id)),
                    output_digest: Sha256Digest::from_bytes(intent.verb.as_bytes()),
                    observed_at_ms,
                }),
            }
        })
    }
}

/// Coffre en mémoire dédié aux tests du double sans couplage supplémentaire.
#[derive(Default)]
pub struct InMemoryTestSecretStore {
    entries: Mutex<BTreeMap<String, SecretString>>,
}

impl Drop for InMemoryTestSecretStore {
    fn drop(&mut self) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, mut secret) in std::mem::take(&mut *entries) {
            secret.zeroize();
        }
    }
}

impl InMemoryTestSecretStore {
    /// Crée un coffre mémoire vide pour les tests.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemoryTestSecretStore {
    fn resolve_for_authorized_transport(
        &self,
        reference: &SecretRef,
        grant: &AuthorizedSecretAccess,
    ) -> Result<ResolvedSecret, SecretStoreError> {
        if !grant.allows_reference(reference) {
            return Err(SecretStoreError::AccessDenied {
                reason: String::from("Reference outside authorized intention"),
            });
        }
        let guard = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .get(reference.as_opaque_str())
            .cloned()
            .map(ResolvedSecret::new)
            .ok_or(SecretStoreError::NotFound)
    }

    fn status(
        &self,
        reference: &SecretRef,
    ) -> Result<unleash_action_broker::secrets::KeyringStatus, SecretStoreError> {
        let guard = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.contains_key(reference.as_opaque_str()) {
            Ok(unleash_action_broker::secrets::KeyringStatus::Present)
        } else {
            Ok(unleash_action_broker::secrets::KeyringStatus::Missing)
        }
    }

    fn store(&self, reference: &SecretRef, secret: SecretString) -> Result<(), SecretStoreError> {
        let mut guard = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mut prev) = guard.insert(reference.as_opaque_str().to_owned(), secret) {
            prev.zeroize();
        }
        Ok(())
    }

    fn revoke(&self, reference: &SecretRef) -> Result<(), SecretStoreError> {
        let mut guard = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(mut removed) = guard.remove(reference.as_opaque_str()) else {
            return Err(SecretStoreError::NotFound);
        };
        removed.zeroize();
        Ok(())
    }
}

/// Espion indépendant vérifiant le nombre exact de résolutions de secrets (`REQ-SEC-003`, `R6`).
pub struct SecretResolutionSpy<S: SecretStore + ?Sized> {
    inner: Arc<S>,
    attempts: AtomicU64,
    successes: AtomicU64,
}

impl<S: SecretStore + ?Sized> SecretResolutionSpy<S> {
    /// Enveloppe n'importe quel [`SecretStore`] pour compter tentatives et résolutions réussies.
    #[must_use]
    pub const fn new(inner: Arc<S>) -> Self {
        Self {
            inner,
            attempts: AtomicU64::new(0),
            successes: AtomicU64::new(0),
        }
    }

    /// Nombre total de tentatives de résolution interceptées.
    #[must_use]
    pub fn resolution_attempts(&self) -> u64 {
        self.attempts.load(Ordering::SeqCst)
    }

    /// Nombre de secrets effectivement résolus et retournés au transport.
    #[must_use]
    pub fn successful_resolutions(&self) -> u64 {
        self.successes.load(Ordering::SeqCst)
    }

    /// Vérifie que zéro secret n'a été résolu avec succès ni tenté.
    ///
    /// # Panics
    /// Panique en test si une tentative de résolution de secret a eu lieu.
    pub fn assert_zero_secret_resolutions(&self) {
        let attempts = self.resolution_attempts();
        let successes = self.successful_resolutions();
        assert_eq!(
            attempts, 0,
            "Zéro résolution de secret attendue, {attempts} tentative(s) ({successes} réussie(s))"
        );
    }
}

impl SecretResolutionSpy<InMemoryTestSecretStore> {
    /// Construit un espion autonome adossé à un coffre en mémoire.
    #[must_use]
    pub fn in_memory() -> Self {
        Self::new(Arc::new(InMemoryTestSecretStore::new()))
    }
}

impl<S: SecretStore + ?Sized> SecretStore for SecretResolutionSpy<S> {
    fn resolve_for_authorized_transport(
        &self,
        reference: &SecretRef,
        grant: &AuthorizedSecretAccess,
    ) -> Result<ResolvedSecret, SecretStoreError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let res = self
            .inner
            .resolve_for_authorized_transport(reference, grant);
        if res.is_ok() {
            self.successes.fetch_add(1, Ordering::SeqCst);
        }
        res
    }

    fn status(
        &self,
        reference: &SecretRef,
    ) -> Result<unleash_action_broker::secrets::KeyringStatus, SecretStoreError> {
        self.inner.status(reference)
    }

    fn store(&self, reference: &SecretRef, secret: SecretString) -> Result<(), SecretStoreError> {
        self.inner.store(reference, secret)
    }

    fn revoke(&self, reference: &SecretRef) -> Result<(), SecretStoreError> {
        self.inner.revoke(reference)
    }
}

/// Sonde d'isolation OS injectable pour tester les modes `Standard` et `Strict` sans dépendance machine.
pub struct TestIsolationProbe {
    status: Mutex<IsolationBackendStatus>,
}

impl TestIsolationProbe {
    /// Sonde rapportant une disponibilité standard/stricte complète.
    #[must_use]
    pub fn standard_available() -> Self {
        Self::available_with("test-seatbelt")
    }

    /// Sonde rapportant un backend disponible avec toutes les capacités d'isolation.
    #[must_use]
    pub fn available_with(backend_name: &str) -> Self {
        Self {
            status: Mutex::new(IsolationBackendStatus::Available {
                backend_name: backend_name.to_owned(),
                supported_capabilities: BTreeSet::from([
                    IsolationCapability::FilesystemWriteScoping,
                    IsolationCapability::ForbiddenPathsReadDeny,
                    IsolationCapability::NetworkEgressProxyOnly,
                    IsolationCapability::CleanEnvironment,
                    IsolationCapability::SubprocessInheritance,
                ]),
                abi_version: String::from("test-abi-1"),
            }),
        }
    }

    /// Bascule la sonde vers un backend sain et complet.
    pub fn set_available(&self, backend_name: &str) {
        let mut st = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        *st = IsolationBackendStatus::Available {
            backend_name: backend_name.to_owned(),
            supported_capabilities: BTreeSet::from([
                IsolationCapability::FilesystemWriteScoping,
                IsolationCapability::ForbiddenPathsReadDeny,
                IsolationCapability::NetworkEgressProxyOnly,
                IsolationCapability::CleanEnvironment,
                IsolationCapability::SubprocessInheritance,
            ]),
            abi_version: String::from("test-abi-1"),
        };
    }

    /// Simule l'absence de backend (`ISOLATION_BACKEND_MISSING`).
    pub fn set_missing_backend(&self, backend_name: &str, remediation: &str) {
        let mut st = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        *st = IsolationBackendStatus::MissingBackend {
            backend_name: backend_name.to_owned(),
            remediation: remediation.to_owned(),
        };
    }

    /// Simule un backend présent mais incapable d'imposer les obligations (`ISOLATION_OBLIGATION_UNENFORCEABLE`).
    pub fn set_unenforceable(&self, backend_name: &str, reason: &str) {
        let mut st = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        *st = IsolationBackendStatus::Unenforceable {
            backend_name: backend_name.to_owned(),
            missing_capabilities: BTreeSet::from([IsolationCapability::NetworkEgressProxyOnly]),
            reason: reason.to_owned(),
            remediation: String::from(
                "Installer un backend complet ou repasser explicitement le projet en standard",
            ),
        };
    }
}

impl IsolationProbePort for TestIsolationProbe {
    fn probe_isolation(&self, req: &IsolationRequirements) -> IsolationBackendStatus {
        if req.mode == IsolationMode::Standard && !req.require_process_confinement {
            return IsolationBackendStatus::Available {
                backend_name: String::from("none_standard"),
                supported_capabilities: BTreeSet::new(),
                abi_version: String::from("standard-unconfined"),
            };
        }
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Vérifie rapidement qu'un refus du broker implique l'absence d'effets côté espions.
///
/// # Errors
/// Retourne [`ActionBrokerError`] en cas d'échec inattendu.
///
/// # Panics
/// Panique uniquement dans le test appelant si le résultat est un succès inattendu.
#[expect(
    clippy::panic,
    reason = "Assertion helper in the dedicated testkit must reject unexpected success"
)]
pub fn assert_refused_with_zero_side_effects<T>(
    result: Result<T, ActionBrokerError>,
    spy: &BrokerExecutionSpy,
) -> ActionBrokerError {
    let Err(err) = result else {
        panic!("Une erreur était attendue mais l'opération a réussi")
    };
    spy.assert_zero_dispatches();
    err
}
