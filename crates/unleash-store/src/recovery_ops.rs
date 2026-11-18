//! Opérations F018 de récupération, réconciliation et approbations humaines :
//! `InspectRecovery`, `ReconcileAction`, `ResolveActionOutcome` et `ResolveActionApproval`.

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unleash_domain::{
    ActionId, ActionState, AuthorityGeneration, BackupId, CommandId, DaemonEpoch, DaemonMode,
    Fence, IdGeneratorPort, InvocationId, ProjectId, ReceiptId, ReconcileOutcome,
    RecoveryClassification, RestoreId, Revision, RunId, Sha256Digest, StreamId, TimestampMs,
};
use unleash_protocol::{EventCausationKind, ReasonCode, Sensitivity};
use uuid::Uuid;

use crate::commands::{AggregateKind, EventAppendSpec, read_daemon_state_conn};
use crate::config::StoreConfig;
use crate::error::StoreError;
use crate::policies_secrets::{CallerAuthority, require_human_decider, require_reconciler};
use crate::qualification::inspect_and_verify_build;
use crate::restore::{
    CapsuleCoverage, MaintenanceManifestEntry, MaintenanceRestoreManifest, RestoreCapsule,
    RestoreRequestState, read_maintenance_manifest, read_maintenance_manifest_for_diagnostics,
};

/// Disposition humaine ou autorisée enregistrée sur une action `outcome_unknown` (`ResolveActionOutcome`).
///
/// L'action reste à l'état `outcome_unknown` (aucune fabrication d'`observed_success`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcomeDisposition {
    /// Autorise la création ultérieure d'une nouvelle action liée (stratégie qualifiée / risque accepté).
    AcceptRiskRetry,
    /// Abandonne l'effet externe incertain et autorise la poursuite du run selon décision.
    AbandonEffect,
    /// Confirmation humaine hors bande avec référence de preuve et limite causale explicite.
    ExternalEffectConfirmedByEvidence,
}

impl ActionOutcomeDisposition {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AcceptRiskRetry => "accept_risk_retry",
            Self::AbandonEffect => "abandon_effect",
            Self::ExternalEffectConfirmedByEvidence => "external_effect_confirmed_by_evidence",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "accept_risk_retry" => Ok(Self::AcceptRiskRetry),
            "abandon_effect" => Ok(Self::AbandonEffect),
            "external_effect_confirmed_by_evidence" => Ok(Self::ExternalEffectConfirmedByEvidence),
            other => Err(StoreError::InvariantViolation(format!(
                "disposition inconnue : '{other}'"
            ))),
        }
    }
}

/// Décision humaine d'approbation (`ResolveActionApproval`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Approuve l'action sur son `approval_scope` et `payload_digest` exacts.
    Approve,
    /// Refuse définitivement l'action sans résolution de secret ni dispatch.
    Deny,
}

impl ApprovalDecision {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
        }
    }
}

/// Enregistrement immuable d'une décision dans `approval_resolutions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalResolutionRecord {
    /// Identifiant de résolution.
    pub resolution_id: String,
    /// Identifiant de l'action résolue.
    pub action_id: ActionId,
    /// Scope exact vérifié.
    pub approval_scope: String,
    /// Digest exact vérifié.
    pub payload_digest: Sha256Digest,
    /// Décision prise (`approve` / `deny`).
    pub decision: ApprovalDecision,
    /// Motif explicite.
    pub reason: String,
    /// Acteur décideur authentifié.
    pub decided_by_actor: String,
    /// Type du décideur (`human_operator` ou `policy_admin`).
    pub decider_kind: String,
    /// Révision d'action avant résolution.
    pub action_revision_before: Revision,
    /// Révision d'action après résolution.
    pub action_revision_after: Revision,
    /// Reçu de commande associé.
    pub command_receipt_id: ReceiptId,
    /// Horodatage de décision.
    pub created_at_ms: TimestampMs,
}

/// Entrée d'enregistrement d'une tentative de dispatch dans `action_dispatches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDispatchInput {
    /// Identifiant de dispatch.
    pub dispatch_id: String,
    /// Action concernée.
    pub action_id: ActionId,
    /// Numéro ordinal (>= 1).
    pub ordinal: i64,
    /// Identifiant de l'adaptateur émetteur.
    pub adapter_id: String,
    /// Empreinte de la requête émise.
    pub request_hash: Sha256Digest,
    /// Identifiant corrélé chez le fournisseur éventuel.
    pub provider_request_id: Option<String>,
    /// État transport (`dispatched`, `sent`, `unknown_after_send`, `acked`, ...).
    pub transport_state: String,
    /// Empreinte du reçu transport éventuel.
    pub receipt_hash: Option<Sha256Digest>,
}

/// Item d'inventaire retourné par `InspectRecovery`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryItem {
    /// Provenance de capsule si cet effet provient d'une restauration.
    pub provenance: Option<RecoveryActionProvenance>,
    /// Identifiant de l'action.
    pub action_id: ActionId,
    /// Run parent éventuel.
    pub run_id: Option<RunId>,
    /// Invocation parente éventuelle.
    pub invocation_id: Option<InvocationId>,
    /// Projet associé éventuel.
    pub project_id: Option<ProjectId>,
    /// Clé de scope concernée éventuelle.
    pub scope_key: Option<String>,
    /// État courant dans `actions`.
    pub state: ActionState,
    /// Classification analytique de récupération.
    pub classification: RecoveryClassification,
    /// Stratégie de réconciliation déclarée.
    pub reconcile_strategy: String,
    /// Dernier statut de réconciliation éventuel.
    pub reconciliation_status: Option<String>,
    /// Disposition explicite enregistrée éventuelle.
    pub disposition: Option<ActionOutcomeDisposition>,
    /// Motif de disposition éventuel.
    pub disposition_reason: Option<String>,
    /// Périmètre d'approbation éventuel.
    pub approval_scope: Option<String>,
    /// Empreinte du payload de l'action.
    pub payload_hash: Sha256Digest,
    /// Dernier identifiant de dispatch éventuel.
    pub last_dispatch_id: Option<String>,
    /// Empreinte de reçu de dispatch éventuelle.
    pub last_receipt_hash: Option<String>,
    /// Motif descriptif de l'état de récupération.
    pub reason: String,
    /// Opérations publiques applicables sur cet item.
    pub allowed_operations: Vec<String>,
    /// Révision optimiste de l'action.
    pub revision: Revision,
}

/// Provenance durable d'un effet présent dans une capsule de restauration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryActionProvenance {
    /// Restauration ayant conservé l'effet.
    pub restore_id: RestoreId,
    /// Capsule vérifiée portant l'effet et ses coûts/dispatches.
    pub capsule_object_hash: Sha256Digest,
    /// Lignée historique de l'effet.
    pub source_authority_generation: AuthorityGeneration,
    /// Faux si des parents absents du snapshot empêchent l'import dans actions.
    pub present_in_active_ledger: bool,
}

/// Demande de restauration enregistrée dans `restore_requests`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreRequestRecord {
    /// Identifiant de restauration.
    pub restore_id: RestoreId,
    /// Reçu associé.
    pub command_receipt_id: ReceiptId,
    /// Sauvegarde ciblée.
    pub backup_id: BackupId,
    /// Lignée source.
    pub source_authority_generation: AuthorityGeneration,
    /// État courant.
    pub state: RestoreRequestState,
    /// Raison de blocage éventuelle.
    pub blocked_reason: Option<String>,
    /// Révision.
    pub revision: Revision,
}

/// Importation de restauration enregistrée dans `restore_imports`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreImportRecord {
    /// Référence durable épinglée de la capsule.
    pub capsule_object_hash: Sha256Digest,
    /// Delta vérifié, y compris les effets dont les parents sont absents du snapshot.
    pub capsule: Option<RestoreCapsule>,
    /// Preuve manquante ou corrompue, sans masquer les scopes gelés.
    pub capsule_error: Option<String>,
    /// Identifiant de restauration.
    pub restore_id: RestoreId,
    /// Reçu conservé.
    pub command_receipt_id: ReceiptId,
    /// Sauvegarde importée.
    pub backup_id: BackupId,
    /// Lignée source.
    pub source_authority_generation: AuthorityGeneration,
    /// Nouvelle lignée activée.
    pub target_authority_generation: AuthorityGeneration,
    /// Couverture connue du delta.
    pub coverage: CapsuleCoverage,
    /// État (`prepared` ou `activated`).
    pub state: String,
    /// Scopes gelés enregistrés.
    pub frozen_scopes: Vec<String>,
    /// Entrées du manifeste privé importées depuis la lignée source éventuelle.
    pub imported_manifest_entries: Vec<MaintenanceManifestEntry>,
    /// Révision.
    pub revision: Revision,
}

/// Disposition interne explicite d'un scope gelé après restauration.
/// Cette décision autorise une suite distincte et conserve l'incertitude des effets historiques.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeFreezeDisposition {
    /// Scope exact à disposer.
    pub scope_id: String,
    /// Restauration courante ayant posé le gel.
    pub restore_id: RestoreId,
    /// Identifiant serveur dédié au reçu interne engagé atomiquement avec cette décision.
    pub decision_receipt_id: ReceiptId,
    /// Décideur résolu par le service appelant.
    pub caller: CallerAuthority,
    /// Justification explicite et risques acceptés.
    pub reason: String,
    /// Portée de la politique appliquée (global ou project).
    pub policy_scope_kind: String,
    /// Identifiant exact de politique.
    pub policy_scope_id: String,
    /// Révision de politique vérifiée transactionnellement.
    pub policy_revision: i64,
    /// Références des preuves examinées, sans les transformer en observation d'effet.
    pub evidence_digests: Vec<Sha256Digest>,
    /// Impact décidé sur dépendances et budgets.
    pub dependency_budget_impact: String,
}
/// Scope gelé enregistré dans `recovery_scope_freezes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryScopeFreezeRecord {
    /// Décision explicite durable si le gel a été levé.
    pub disposition: Option<ScopeFreezeDisposition>,
    /// Identifiant du scope gelé.
    pub scope_id: String,
    /// Restauration ayant posé le gel.
    pub restore_id: RestoreId,
    /// Raison du gel.
    pub reason: String,
    /// Vrai s'il s'agit d'un gel global (`coverage == Unknown`).
    pub global_freeze: bool,
    /// État (`frozen` ou `disposed`).
    pub state: String,
    /// Horodatage de création.
    pub created_at_ms: TimestampMs,
    /// Horodatage de levée éventuel.
    pub disposed_at_ms: Option<TimestampMs>,
    /// Reçu de disposition éventuel.
    pub disposition_receipt_id: Option<ReceiptId>,
}

/// Rapport global retourné par `InspectRecovery`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    /// Purges engagées, état physique et checkpoint, y compris issues bloquées.
    pub retention_purges: Vec<crate::events::RetentionPurgeStatusRecord>,
    /// Mode courant du daemon si base lisible.
    pub daemon_mode: Option<DaemonMode>,
    /// Lignée active si base lisible.
    pub active_authority_generation: Option<AuthorityGeneration>,
    /// Flux actif si base lisible.
    pub active_stream_id: Option<StreamId>,
    /// Vrai si la base source a été constatée corrompue ou illisible (repli manifeste privé).
    pub storage_corrupt: bool,
    /// Contenu du manifeste privé de maintenance `0600` lorsqu'il existe.
    pub maintenance_manifest: Option<MaintenanceRestoreManifest>,
    /// Demandes de restauration en base.
    pub restore_requests: Vec<RestoreRequestRecord>,
    /// Restaurations activées en base cible.
    pub restore_imports: Vec<RestoreImportRecord>,
    /// Scopes gelés en base cible.
    pub frozen_scopes: Vec<RecoveryScopeFreezeRecord>,
    /// Runs actuellement bloqués.
    pub blocked_runs: Vec<RunId>,
    /// Éléments d'actions inventoriés.
    pub items: Vec<RecoveryItem>,
}

pub(crate) fn parse_action_state(raw: &str) -> Result<ActionState, StoreError> {
    match raw {
        "prepared" => Ok(ActionState::Prepared),
        "authorizing" => Ok(ActionState::Authorizing),
        "approval_required" => Ok(ActionState::ApprovalRequired),
        "authorized" => Ok(ActionState::Authorized),
        "denied" => Ok(ActionState::Denied),
        "dispatching" => Ok(ActionState::Dispatching),
        "in_flight" => Ok(ActionState::InFlight),
        "observed_success" => Ok(ActionState::ObservedSuccess),
        "observed_failure" => Ok(ActionState::ObservedFailure),
        "outcome_unknown" => Ok(ActionState::OutcomeUnknown),
        "cancel_requested" => Ok(ActionState::CancelRequested),
        "cancelled" => Ok(ActionState::Cancelled),
        other => Err(StoreError::InvariantViolation(format!(
            "état d'action inconnu : '{other}'"
        ))),
    }
}

pub(crate) fn record_action_dispatch_tx(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    input: &ActionDispatchInput,
) -> Result<(), StoreError> {
    let exists: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM actions WHERE action_id = ?1",
            [input.action_id.to_string()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if exists == 0 {
        return Err(StoreError::NotFound(format!("action {}", input.action_id)));
    }
    let ended = if input.receipt_hash.is_some() {
        Some(now_ms.as_i64())
    } else {
        None
    };
    tx.execute(
        "INSERT INTO action_dispatches
         (dispatch_id, action_id, ordinal, adapter_id, request_hash, provider_request_id,
          started_at_ms, ended_at_ms, transport_state, receipt_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            input.dispatch_id,
            input.action_id.to_string(),
            input.ordinal,
            input.adapter_id,
            input.request_hash.as_prefixed_str(),
            input.provider_request_id,
            now_ms.as_i64(),
            ended,
            input.transport_state,
            input
                .receipt_hash
                .as_ref()
                .map(Sha256Digest::as_prefixed_str),
        ],
    )
    .map_err(StoreError::from_rusqlite)?;
    Ok(())
}

pub(crate) fn reconcile_recovered_actions_after_restart_tx(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
) -> Result<Vec<ActionId>, StoreError> {
    let cur_epoch: i64 = tx
        .query_row(
            "SELECT daemon_epoch FROM daemon_state WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;

    let mut stmt = tx
        .prepare(
            "SELECT action_id FROM actions
             WHERE state IN ('dispatching', 'in_flight', 'cancel_requested')
               AND daemon_epoch < ?1
             ORDER BY created_at_ms ASC, action_id ASC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([cur_epoch], |r| r.get::<_, String>(0))
        .map_err(StoreError::from_rusqlite)?;
    let mut ids = Vec::new();
    for item in rows {
        let id_str = item.map_err(StoreError::from_rusqlite)?;
        ids.push(id_str);
    }
    drop(stmt);

    let mut out = Vec::with_capacity(ids.len());
    for id_str in ids {
        tx.execute(
            "UPDATE actions
             SET state = 'outcome_unknown',
                 reconciliation_status = 'outcome_unknown',
                 revision = revision + 1,
                 updated_at_ms = ?1
             WHERE action_id = ?2",
            rusqlite::params![now_ms.as_i64(), id_str],
        )
        .map_err(StoreError::from_rusqlite)?;
        out.push(id_str.parse::<ActionId>()?);
    }
    Ok(out)
}

fn resolve_blocked_run_tx(
    tx: &Transaction<'_>,
    run_id: RunId,
    action_id: ActionId,
    project_id: Option<ProjectId>,
    resolution_kind: &str,
    generated_events: &mut Vec<EventAppendSpec>,
) -> Result<(), StoreError> {
    let run_row: Option<(String, i64)> = tx
        .query_row(
            "SELECT state, revision FROM workflow_runs WHERE run_id = ?1",
            [run_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    if let Some((run_state, run_rev)) = run_row {
        let remaining_blocking: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM actions
                 WHERE run_id = ?1
                   AND state IN ('dispatching', 'in_flight', 'cancel_requested', 'outcome_unknown')
                   AND disposition IS NULL",
                [run_id.to_string()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;

        let next_run_rev = run_rev.saturating_add(1);
        let next_run_state = if remaining_blocking == 0 && run_state == "blocked" {
            "running"
        } else {
            run_state.as_str()
        };
        tx.execute(
            "UPDATE workflow_runs SET state = ?1, revision = ?2 WHERE run_id = ?3",
            rusqlite::params![next_run_state, next_run_rev, run_id.to_string()],
        )
        .map_err(StoreError::from_rusqlite)?;

        let mut payload = Map::new();
        payload.insert("action_id".to_owned(), Value::String(action_id.to_string()));
        payload.insert(
            "resolution_kind".to_owned(),
            Value::String(resolution_kind.to_owned()),
        );
        payload.insert(
            "remaining_unresolved_actions".to_owned(),
            Value::Number(remaining_blocking.into()),
        );
        payload.insert(
            "new_run_state".to_owned(),
            Value::String(next_run_state.to_owned()),
        );

        generated_events.push(EventAppendSpec {
            event_type: "EffectOutcomeResolved".to_owned(),
            aggregate_type: AggregateKind::WorkflowRun,
            aggregate_id: run_id.as_uuid(),
            aggregate_revision: Revision::new(u64::try_from(next_run_rev).unwrap_or(1)),
            project_id,
            correlation_id: action_id.as_uuid(),
            causation_kind: EventCausationKind::Command,
            causation_id: None,
            payload,
            sensitivity: Sensitivity::Project,
            outbox_destinations: vec!["scheduler".to_owned()],
        });
    }

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "Réconciliation atomique ReconcileAction avec contrôle d'époque/fence, preuve et déblocage du run"
)]
pub(crate) fn tx_reconcile_action(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    daemon_epoch: DaemonEpoch,
    expected_rev: Revision,
    cmd_id: CommandId,
    action_id: ActionId,
    caller: &CallerAuthority,
    outcome: ReconcileOutcome,
    evidence_digest: Option<Sha256Digest>,
    receipt_hash: Option<Sha256Digest>,
    reason: &str,
    expected_source_epoch: Option<DaemonEpoch>,
    expected_source_fence: Option<Fence>,
    generated_events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    require_reconciler(caller, "ReconcileAction")?;
    if reason.trim().is_empty() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "motif obligatoire pour ReconcileAction".to_owned(),
        });
    }

    #[expect(
        clippy::type_complexity,
        reason = "Tuple SQLite temporaire projetant l'action courante"
    )]
    let row: Option<(
        String,
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = tx
        .query_row(
            "SELECT state, revision, daemon_epoch, fence, run_id, project_id, scope_key
             FROM actions WHERE action_id = ?1",
            [action_id.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((cur_state_s, cur_rev, action_epoch, action_fence, run_id_s, proj_id_s, _scope_key)) =
        row
    else {
        return Err(StoreError::NotFound(format!("action {action_id}")));
    };

    if let Some(exp_ep) = expected_source_epoch {
        let ep_u64 = exp_ep.as_u64();
        if ep_u64 != daemon_epoch.as_u64() || i64::try_from(ep_u64).unwrap_or(0) < action_epoch {
            return Err(StoreError::StaleFence {
                message: format!(
                    "époque obsolète pour ReconcileAction : fournie {ep_u64}, active {}",
                    daemon_epoch.as_u64()
                ),
            });
        }
    }
    if let Some(exp_f) = expected_source_fence
        && i64::try_from(exp_f.as_u64()).unwrap_or(0) < action_fence
    {
        return Err(StoreError::StaleFence {
            message: format!(
                "fence obsolète pour ReconcileAction : fourni {}, courant {action_fence}",
                exp_f.as_u64()
            ),
        });
    }

    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if cur_rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(cur_rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }

    let cur_state = parse_action_state(&cur_state_s)?;
    if !matches!(
        cur_state,
        ActionState::Dispatching
            | ActionState::InFlight
            | ActionState::OutcomeUnknown
            | ActionState::CancelRequested
    ) {
        return Err(StoreError::ContractViolation {
            reason: Some(ReasonCode::TerminalImmutable),
            message: format!(
                "action {action_id} en état '{cur_state_s}' non éligible à ReconcileAction"
            ),
        });
    }

    if outcome == ReconcileOutcome::ConfirmedSuccess
        && evidence_digest.is_none()
        && receipt_hash.is_none()
    {
        return Err(StoreError::EvidenceUnavailable(
            "ConfirmedSuccess exige evidence_digest ou receipt_hash vérifiable".to_owned(),
        ));
    }

    let (next_state, rec_status, is_terminal_resolved) = match outcome {
        ReconcileOutcome::ConfirmedSuccess => {
            (ActionState::ObservedSuccess, "effect_observed", true)
        }
        ReconcileOutcome::ConfirmedFailureNoEffect => {
            let st = if cur_state == ActionState::CancelRequested {
                ActionState::Cancelled
            } else {
                ActionState::ObservedFailure
            };
            (st, "confirmed_failure_no_effect", true)
        }
        ReconcileOutcome::StillRunning => (ActionState::InFlight, "still_running", false),
        ReconcileOutcome::Conflict => (ActionState::OutcomeUnknown, "conflict", false),
        ReconcileOutcome::Unknown => (ActionState::OutcomeUnknown, "outcome_unknown", false),
    };

    let next_rev = cur_rev.saturating_add(1);
    tx.execute(
        "UPDATE actions
         SET state = ?1,
             reconciliation_status = ?2,
             revision = ?3,
             updated_at_ms = ?4
         WHERE action_id = ?5",
        rusqlite::params![
            next_state.to_string(),
            rec_status,
            next_rev,
            now_ms.as_i64(),
            action_id.to_string()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    if let Some(ref rh) = receipt_hash {
        tx.execute(
            "UPDATE action_dispatches
             SET receipt_hash = ?1, ended_at_ms = COALESCE(ended_at_ms, ?2), transport_state = ?3
             WHERE action_id = ?4
               AND ordinal = (SELECT MAX(ordinal) FROM action_dispatches WHERE action_id = ?4)",
            rusqlite::params![
                rh.as_prefixed_str(),
                now_ms.as_i64(),
                rec_status,
                action_id.to_string()
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
    }

    let project_id = proj_id_s
        .as_deref()
        .map(str::parse::<ProjectId>)
        .transpose()?;
    let run_id = run_id_s.as_deref().map(str::parse::<RunId>).transpose()?;

    let mut payload = Map::new();
    payload.insert("outcome".to_owned(), Value::String(outcome.to_string()));
    payload.insert(
        "new_state".to_owned(),
        Value::String(next_state.to_string()),
    );
    payload.insert("reason".to_owned(), Value::String(reason.to_owned()));
    if let Some(evd) = evidence_digest {
        payload.insert(
            "evidence_digest".to_owned(),
            Value::String(evd.as_prefixed_str()),
        );
    }

    generated_events.push(EventAppendSpec {
        event_type: "ActionReconciled".to_owned(),
        aggregate_type: AggregateKind::Action,
        aggregate_id: action_id.as_uuid(),
        aggregate_revision: Revision::new(u64::try_from(next_rev).unwrap_or(1)),
        project_id,
        correlation_id: action_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload,
        sensitivity: Sensitivity::Project,
        outbox_destinations: vec![],
    });

    if is_terminal_resolved && let Some(rid) = run_id {
        resolve_blocked_run_tx(
            tx,
            rid,
            action_id,
            project_id,
            "reconcile_action",
            generated_events,
        )?;
    }

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "Disposition atomique ResolveActionOutcome : laisse outcome_unknown + émet RecoveryDecisionRecorded & EffectOutcomeResolved"
)]
pub(crate) fn tx_resolve_action_outcome(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: CommandId,
    receipt_id: ReceiptId,
    action_id: ActionId,
    caller: &CallerAuthority,
    disposition: ActionOutcomeDisposition,
    reason: &str,
    evidence_ref: Option<&str>,
    generated_events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    require_human_decider(caller, "ResolveActionOutcome")?;
    let trimmed_reason = reason.trim();
    if trimmed_reason.is_empty() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "motif explicite obligatoire pour ResolveActionOutcome".to_owned(),
        });
    }

    let row: Option<(String, i64, Option<String>, Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT state, revision, run_id, project_id, scope_key
             FROM actions WHERE action_id = ?1",
            [action_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((state_s, cur_rev, run_id_s, proj_id_s, _scope_key)) = row else {
        return Err(StoreError::NotFound(format!("action {action_id}")));
    };
    let cur_state = parse_action_state(&state_s)?;
    if cur_state != ActionState::OutcomeUnknown {
        return Err(StoreError::ContractViolation {
            reason: None,
            message: format!(
                "ResolveActionOutcome exige l'état outcome_unknown (actuel : '{state_s}')"
            ),
        });
    }
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if cur_rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(cur_rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }

    let next_rev = cur_rev.saturating_add(1);
    // Invariant F018 : `actions.state` demeure impérativement `'outcome_unknown'`, seule la disposition est écrite !
    tx.execute(
        "UPDATE actions
         SET state = 'outcome_unknown',
             disposition = ?1,
             disposition_reason = ?2,
             disposition_evidence_ref = ?3,
             disposition_receipt_id = ?4,
             disposition_decided_by = ?5,
             disposition_decided_at_ms = ?6,
             revision = ?7,
             updated_at_ms = ?6
         WHERE action_id = ?8",
        rusqlite::params![
            disposition.as_str(),
            trimmed_reason,
            evidence_ref,
            receipt_id.to_string(),
            caller.actor_id.as_str(),
            now_ms.as_i64(),
            next_rev,
            action_id.to_string()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    let project_id = proj_id_s
        .as_deref()
        .map(str::parse::<ProjectId>)
        .transpose()?;
    let run_id = run_id_s.as_deref().map(str::parse::<RunId>).transpose()?;

    let mut payload = Map::new();
    payload.insert(
        "disposition".to_owned(),
        Value::String(disposition.as_str().to_owned()),
    );
    payload.insert(
        "action_state".to_owned(),
        Value::String("outcome_unknown".to_owned()),
    );
    payload.insert(
        "reason".to_owned(),
        Value::String(trimmed_reason.to_owned()),
    );
    if let Some(er) = evidence_ref {
        payload.insert("evidence_ref".to_owned(), Value::String(er.to_owned()));
    }

    generated_events.push(EventAppendSpec {
        event_type: "RecoveryDecisionRecorded".to_owned(),
        aggregate_type: AggregateKind::Action,
        aggregate_id: action_id.as_uuid(),
        aggregate_revision: Revision::new(u64::try_from(next_rev).unwrap_or(1)),
        project_id,
        correlation_id: action_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload,
        sensitivity: Sensitivity::Project,
        outbox_destinations: vec![],
    });

    if let Some(rid) = run_id {
        resolve_blocked_run_tx(
            tx,
            rid,
            action_id,
            project_id,
            disposition.as_str(),
            generated_events,
        )?;
    }

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "Approbation humaine atomique ResolveActionApproval sur approval_scope et payload_digest exacts"
)]
pub(crate) fn tx_resolve_action_approval(
    tx: &Transaction<'_>,
    id_gen: &dyn IdGeneratorPort,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: CommandId,
    receipt_id: ReceiptId,
    action_id: ActionId,
    caller: &CallerAuthority,
    approval_scope: &str,
    payload_digest: Sha256Digest,
    decision: ApprovalDecision,
    reason: &str,
    generated_events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    require_human_decider(caller, "ResolveActionApproval")?;
    let trimmed_reason = reason.trim();
    if trimmed_reason.is_empty() || approval_scope.trim().is_empty() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "approval_scope et reason obligatoires pour ResolveActionApproval".to_owned(),
        });
    }

    let row: Option<(String, i64, Option<String>, String, Option<String>)> = tx
        .query_row(
            "SELECT state, revision, approval_scope, payload_hash, project_id
             FROM actions WHERE action_id = ?1",
            [action_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((state_s, cur_rev, stored_scope, stored_payload_hash, proj_id_s)) = row else {
        return Err(StoreError::NotFound(format!("action {action_id}")));
    };
    if cur_rev != i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX) {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(cur_rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }
    if parse_action_state(&state_s)? != ActionState::ApprovalRequired {
        return Err(StoreError::ContractViolation {
            reason: None,
            message: format!(
                "action {action_id} n'est pas en 'approval_required' (état : '{state_s}')"
            ),
        });
    }
    if stored_scope.as_deref() != Some(approval_scope) {
        return Err(StoreError::ContractViolation {
            reason: None,
            message: format!(
                "approval_scope divergent : attendu '{}', reçu '{approval_scope}'",
                stored_scope.as_deref().unwrap_or("<absent>")
            ),
        });
    }
    if stored_payload_hash != payload_digest.as_prefixed_str() {
        return Err(StoreError::ContractViolation {
            reason: None,
            message: "payload_digest divergent sur ResolveActionApproval".to_owned(),
        });
    }

    let resolution_id = id_gen.new_uuid().to_string();
    let (next_state, auth_ref, event_type) = match decision {
        ApprovalDecision::Approve => (
            ActionState::Authorized,
            Some(resolution_id.clone()),
            "ActionApproved",
        ),
        ApprovalDecision::Deny => (ActionState::Denied, None, "ActionDenied"),
    };
    let next_rev = cur_rev.saturating_add(1);

    tx.execute(
        "UPDATE actions
         SET state = ?1, authorization_ref = ?2, revision = ?3, updated_at_ms = ?4
         WHERE action_id = ?5",
        rusqlite::params![
            next_state.to_string(),
            auth_ref,
            next_rev,
            now_ms.as_i64(),
            action_id.to_string()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    tx.execute(
        "INSERT INTO approval_resolutions
         (resolution_id, action_id, approval_scope, payload_digest, decision, reason,
          decided_by_actor, decider_kind, action_revision_before, action_revision_after,
          command_receipt_id, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            resolution_id,
            action_id.to_string(),
            approval_scope,
            payload_digest.as_prefixed_str(),
            decision.as_str(),
            trimmed_reason,
            caller.actor_id.as_str(),
            caller.role.as_str(),
            cur_rev,
            next_rev,
            receipt_id.to_string(),
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    let project_id = proj_id_s
        .as_deref()
        .map(str::parse::<ProjectId>)
        .transpose()?;
    let mut payload = Map::new();
    payload.insert(
        "decision".to_owned(),
        Value::String(decision.as_str().to_owned()),
    );
    payload.insert(
        "approval_scope".to_owned(),
        Value::String(approval_scope.to_owned()),
    );
    payload.insert(
        "payload_digest".to_owned(),
        Value::String(payload_digest.as_prefixed_str()),
    );

    generated_events.push(EventAppendSpec {
        event_type: event_type.to_owned(),
        aggregate_type: AggregateKind::Action,
        aggregate_id: action_id.as_uuid(),
        aggregate_revision: Revision::new(u64::try_from(next_rev).unwrap_or(1)),
        project_id,
        correlation_id: action_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload,
        sensitivity: Sensitivity::Project,
        outbox_destinations: vec![],
    });

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

fn classify_action_for_recovery(
    state: ActionState,
    reconciliation_status: Option<&str>,
    has_dispatch: bool,
) -> (RecoveryClassification, Vec<String>) {
    match state {
        ActionState::Prepared
        | ActionState::Authorizing
        | ActionState::ApprovalRequired
        | ActionState::Authorized => {
            if has_dispatch {
                (
                    RecoveryClassification::InFlight,
                    vec!["ReconcileAction".to_owned()],
                )
            } else {
                (RecoveryClassification::NotDispatched, vec![])
            }
        }
        ActionState::Dispatching | ActionState::InFlight | ActionState::CancelRequested => (
            RecoveryClassification::InFlight,
            vec!["ReconcileAction".to_owned()],
        ),
        ActionState::ObservedSuccess => (RecoveryClassification::ConfirmedSuccess, vec![]),
        ActionState::ObservedFailure | ActionState::Cancelled | ActionState::Denied => {
            (RecoveryClassification::ConfirmedFailureNoEffect, vec![])
        }
        ActionState::OutcomeUnknown => {
            let class = if reconciliation_status == Some("conflict") {
                RecoveryClassification::Conflict
            } else {
                RecoveryClassification::Unknown
            };
            (
                class,
                vec![
                    "ReconcileAction".to_owned(),
                    "ResolveActionOutcome".to_owned(),
                ],
            )
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Agrégation transactionnelle cohérente de l'inventaire F018 (actions, blocages, restore_requests, restore_imports, gels)"
)]
pub(crate) fn inspect_recovery_conn(
    conn: &Connection,
    config: &StoreConfig,
) -> Result<RecoveryReport, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    let maintenance_manifest = read_maintenance_manifest_for_diagnostics(&config.maintenance_dir)?;

    let mut restore_requests = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT restore_id, command_receipt_id, backup_id, source_authority_generation,
                        state, blocked_reason, revision
                 FROM restore_requests ORDER BY rowid ASC",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        for item in rows {
            let (rid, rcpt, bid, src_auth, st_s, br, rev) =
                item.map_err(StoreError::from_rusqlite)?;
            let state = match st_s.as_str() {
                "requested" => RestoreRequestState::Requested,
                "neutralizing" => RestoreRequestState::Neutralizing,
                "capsule_ready" => RestoreRequestState::CapsuleReady,
                "blocked" => RestoreRequestState::Blocked,
                "handed_over" => RestoreRequestState::HandedOver,
                "aborted" => RestoreRequestState::Aborted,
                other => {
                    return Err(StoreError::InvariantViolation(format!(
                        "état restore_requests inconnu : '{other}'"
                    )));
                }
            };
            restore_requests.push(RestoreRequestRecord {
                restore_id: rid.parse::<RestoreId>()?,
                command_receipt_id: rcpt.parse::<ReceiptId>()?,
                backup_id: bid.parse::<BackupId>()?,
                source_authority_generation: src_auth.parse::<AuthorityGeneration>()?,
                state,
                blocked_reason: br,
                revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
            });
        }
    }

    let mut restore_imports = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT restore_id, command_receipt_id, backup_id, source_authority_generation,
                        target_authority_generation, coverage, state, frozen_scopes_json,
                        imported_manifest_json, revision, capsule_object_hash
                 FROM restore_imports ORDER BY rowid ASC",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, String>(10)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        for item in rows {
            let (
                rid,
                rcpt,
                bid,
                src_auth,
                tgt_auth,
                cov_s,
                st,
                frozen_s,
                imp_man_s,
                rev,
                capsule_hash,
            ) = item.map_err(StoreError::from_rusqlite)?;
            let coverage = match cov_s.as_str() {
                "complete" => CapsuleCoverage::Complete,
                "partial" => CapsuleCoverage::Partial,
                _ => CapsuleCoverage::Unknown,
            };
            let frozen_scopes: Vec<String> = serde_json::from_str(&frozen_s)?;
            let mut imported_manifest_entries: Vec<MaintenanceManifestEntry> = match imp_man_s {
                Some(raw) => serde_json::from_str(&raw)?,
                None => Vec::new(),
            };
            if imported_manifest_entries.is_empty() {
                let mut m_stmt = conn
                    .prepare(
                        "SELECT source_restore_id, command_receipt_id, actor_id, command_id,
                                command_digest, backup_id, source_authority_generation, state,
                                blocked_reason, updated_at_ms
                         FROM restore_manifest_records
                         WHERE imported_by_restore_id = ?1
                         ORDER BY rowid ASC",
                    )
                    .map_err(StoreError::from_rusqlite)?;
                let m_rows = m_stmt
                    .query_map([&rid], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, String>(5)?,
                            r.get::<_, String>(6)?,
                            r.get::<_, String>(7)?,
                            r.get::<_, Option<String>>(8)?,
                            r.get::<_, i64>(9)?,
                        ))
                    })
                    .map_err(StoreError::from_rusqlite)?;
                for mr in m_rows {
                    let (srid, crcpt, aid, cid, cdig, mbid, msauth, mst, mbr, mup) =
                        mr.map_err(StoreError::from_rusqlite)?;
                    let r_st = match mst.as_str() {
                        "blocked" => RestoreRequestState::Blocked,
                        "handed_over" => RestoreRequestState::HandedOver,
                        "neutralizing" => RestoreRequestState::Neutralizing,
                        "capsule_ready" => RestoreRequestState::CapsuleReady,
                        "aborted" => RestoreRequestState::Aborted,
                        _ => RestoreRequestState::Requested,
                    };
                    imported_manifest_entries.push(MaintenanceManifestEntry {
                        restore_id: srid.parse::<RestoreId>()?,
                        command_receipt_id: crcpt.parse::<ReceiptId>()?,
                        actor_id: aid,
                        command_id: cid.parse::<CommandId>()?,
                        command_digest: cdig.parse::<Sha256Digest>()?,
                        backup_id: mbid.parse::<BackupId>()?,
                        source_authority_generation: msauth.parse::<AuthorityGeneration>()?,
                        state: r_st,
                        blocked_reason: mbr,
                        updated_at_ms: TimestampMs::new(mup),
                    });
                }
            }
            let capsule_object_hash = capsule_hash.parse::<Sha256Digest>()?;
            let capsule_result =
                crate::objects::read_verified_object_bytes_conn(conn, config, capsule_object_hash)
                    .and_then(|bytes| {
                        serde_json::from_slice::<RestoreCapsule>(&bytes).map_err(StoreError::from)
                    });
            let (capsule, capsule_error) = match capsule_result {
                Ok(capsule)
                    if capsule.restore_id.to_string() == rid
                        && capsule.source_authority_generation.to_string() == src_auth =>
                {
                    (Some(capsule), None)
                }
                Ok(_) => (None, Some("capsule provenance mismatch".to_owned())),
                Err(error) => (None, Some(error.to_string())),
            };
            restore_imports.push(RestoreImportRecord {
                capsule_object_hash,
                capsule,
                capsule_error,
                restore_id: rid.parse::<RestoreId>()?,
                command_receipt_id: rcpt.parse::<ReceiptId>()?,
                backup_id: bid.parse::<BackupId>()?,
                source_authority_generation: src_auth.parse::<AuthorityGeneration>()?,
                target_authority_generation: tgt_auth.parse::<AuthorityGeneration>()?,
                coverage,
                state: st,
                frozen_scopes,
                imported_manifest_entries,
                revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
            });
        }
    }

    let mut frozen_scopes = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT scope_id, restore_id, reason, global_freeze, state,
                        created_at_ms, disposed_at_ms, disposition_receipt_id
                 FROM recovery_scope_freezes ORDER BY scope_id ASC",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        for item in rows {
            let (scope_id, rid, reason, gf, state, cr, disp, disp_rcpt) =
                item.map_err(StoreError::from_rusqlite)?;
            let decision_json: Option<String> = conn.query_row(
                "SELECT decision_json FROM recovery_scope_dispositions WHERE scope_id = ?1 AND restore_id = ?2",
                rusqlite::params![scope_id, rid], |r| r.get(0),
            ).optional().map_err(StoreError::from_rusqlite)?;
            frozen_scopes.push(RecoveryScopeFreezeRecord {
                disposition: decision_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?,
                scope_id,
                restore_id: rid.parse::<RestoreId>()?,
                reason,
                global_freeze: gf != 0,
                state,
                created_at_ms: TimestampMs::new(cr),
                disposed_at_ms: disp.map(TimestampMs::new),
                disposition_receipt_id: disp_rcpt
                    .as_deref()
                    .map(str::parse::<ReceiptId>)
                    .transpose()?,
            });
        }
    }

    let mut blocked_runs = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT run_id FROM workflow_runs WHERE state = 'blocked' ORDER BY started_at_ms ASC")
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(StoreError::from_rusqlite)?;
        for item in rows {
            blocked_runs.push(item.map_err(StoreError::from_rusqlite)?.parse::<RunId>()?);
        }
    }

    let mut items = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT a.action_id, a.run_id, a.invocation_id, a.project_id, a.scope_key,
                        a.state, a.reconcile_strategy, a.reconciliation_status, a.disposition,
                        a.disposition_reason, a.approval_scope, a.payload_hash, a.revision,
                        (SELECT d.dispatch_id FROM action_dispatches d
                         WHERE d.action_id = a.action_id ORDER BY d.ordinal DESC LIMIT 1),
                        (SELECT d.receipt_hash FROM action_dispatches d
                         WHERE d.action_id = a.action_id ORDER BY d.ordinal DESC LIMIT 1)
                 FROM actions a
                 ORDER BY a.created_at_ms ASC, a.action_id ASC",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, Option<String>>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, i64>(12)?,
                    r.get::<_, Option<String>>(13)?,
                    r.get::<_, Option<String>>(14)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        for row in rows {
            let (
                aid_s,
                rid_s,
                iid_s,
                pid_s,
                scope_key,
                state_s,
                strat,
                rec_st,
                disp_s,
                disp_reason,
                app_scope,
                ph_s,
                rev,
                last_disp,
                last_rcpt_hash,
            ) = row.map_err(StoreError::from_rusqlite)?;

            let state = parse_action_state(&state_s)?;
            let (classification, allowed_operations) =
                classify_action_for_recovery(state, rec_st.as_deref(), last_disp.is_some());
            let reason_text = disp_reason
                .clone()
                .unwrap_or_else(|| rec_st.clone().unwrap_or_else(|| format!("state:{state_s}")));

            items.push(RecoveryItem {
                provenance: None,
                action_id: aid_s.parse::<ActionId>()?,
                run_id: rid_s.as_deref().map(str::parse::<RunId>).transpose()?,
                invocation_id: iid_s
                    .as_deref()
                    .map(str::parse::<InvocationId>)
                    .transpose()?,
                project_id: pid_s.as_deref().map(str::parse::<ProjectId>).transpose()?,
                scope_key,
                state,
                classification,
                reconcile_strategy: strat,
                reconciliation_status: rec_st,
                disposition: disp_s
                    .as_deref()
                    .map(ActionOutcomeDisposition::parse)
                    .transpose()?,
                disposition_reason: disp_reason,
                approval_scope: app_scope,
                payload_hash: ph_s.parse::<Sha256Digest>()?,
                last_dispatch_id: last_disp,
                last_receipt_hash: last_rcpt_hash,
                reason: reason_text,
                allowed_operations,
                revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
            });
        }
    }

    for import in &restore_imports {
        let Some(capsule) = &import.capsule else {
            continue;
        };
        for action in &capsule.actions {
            let provenance = RecoveryActionProvenance {
                restore_id: import.restore_id,
                capsule_object_hash: import.capsule_object_hash,
                source_authority_generation: import.source_authority_generation,
                present_in_active_ledger: false,
            };
            if let Some(item) = items
                .iter_mut()
                .find(|item| item.action_id == action.action_id)
            {
                item.provenance = Some(RecoveryActionProvenance {
                    present_in_active_ledger: true,
                    ..provenance
                });
                continue;
            }
            let dispatch = capsule
                .dispatches
                .iter()
                .filter(|d| d.action_id == action.action_id)
                .max_by_key(|d| d.ordinal);
            let state = parse_action_state(&action.state)?;
            let (classification, _) = classify_action_for_recovery(state, None, dispatch.is_some());
            items.push(RecoveryItem {
                provenance: Some(provenance),
                action_id: action.action_id,
                run_id: action.run_id.as_deref().map(str::parse).transpose()?,
                invocation_id: action
                    .invocation_id
                    .as_deref()
                    .map(str::parse)
                    .transpose()?,
                project_id: action.project_id.as_deref().map(str::parse).transpose()?,
                scope_key: Some(action.scope_id.clone()),
                state,
                classification,
                reconcile_strategy: action.reconcile_strategy.clone(),
                reconciliation_status: None,
                disposition: None,
                disposition_reason: None,
                approval_scope: None,
                payload_hash: action.payload_hash.ok_or_else(|| {
                    StoreError::EvidenceUnavailable("capsule action payload absent".to_owned())
                })?,
                last_dispatch_id: dispatch.map(|d| d.dispatch_id.clone()),
                last_receipt_hash: dispatch
                    .and_then(|d| d.receipt_hash.as_ref().map(ToString::to_string)),
                reason: "post_snapshot_parent_absent".to_owned(),
                // L'absence du parent n'accorde aucune autorité pour recréer son intention.
                allowed_operations: vec![],
                revision: Revision::new(action.source_revision),
            });
        }
    }

    Ok(RecoveryReport {
        retention_purges: crate::events::read_retention_purge_statuses_conn(conn)?,
        daemon_mode: Some(daemon.mode),
        active_authority_generation: Some(daemon.authority_generation),
        active_stream_id: Some(daemon.stream_id),
        storage_corrupt: false,
        maintenance_manifest,
        restore_requests,
        restore_imports,
        frozen_scopes,
        blocked_runs,
        items,
    })
}

/// Inspecte la récupération sans supposer la base ouvrable en écriture (`InspectRecovery` offline).
///
/// Lorsque la base est corrompue ou absente, bascule sur le manifeste privé `0600` sous `<data_root>/maintenance`.
///
/// # Errors
/// Retourne [`StoreError`] en cas d'échec de lecture du manifeste privé.
pub(crate) fn inspect_recovery_offline_cfg(
    config: &StoreConfig,
) -> Result<RecoveryReport, StoreError> {
    if config.db_path.exists()
        && let Ok(conn) = Connection::open_with_flags(
            &config.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        && inspect_and_verify_build(&conn, &config.build_policy).is_ok()
        && let Ok(quick) = conn.pragma_query_value(None, "quick_check", |r| r.get::<_, String>(0))
        && quick.eq_ignore_ascii_case("ok")
        && let Ok(report) = inspect_recovery_conn(&conn, config)
    {
        return Ok(report);
    }

    let maintenance_manifest = read_maintenance_manifest_for_diagnostics(&config.maintenance_dir)?;
    Ok(RecoveryReport {
        retention_purges: vec![],
        daemon_mode: Some(DaemonMode::ReadOnlyRecovery),
        active_authority_generation: None,
        active_stream_id: None,
        storage_corrupt: true,
        maintenance_manifest,
        restore_requests: vec![],
        restore_imports: vec![],
        frozen_scopes: vec![],
        blocked_runs: vec![],
        items: vec![],
    })
}

/// Recherche un reçu `RestoreBackup` par `(actor_id, command_id)` dans le manifeste privé `0600`
/// ou dans les `restore_manifest_records` / `restore_imports` après bascule.
///
/// # Errors
/// Retourne [`StoreError`] si la lecture du manifeste ou de la base échoue.
pub(crate) fn lookup_restore_receipt_cfg(
    config: &StoreConfig,
    actor_id: &str,
    command_id: CommandId,
) -> Result<Option<MaintenanceRestoreManifest>, StoreError> {
    if let Some(m) = read_maintenance_manifest(&config.maintenance_dir)? {
        if m.actor_id == actor_id && m.command_id == command_id {
            return Ok(Some(m));
        }
        if let Some(entry) = m
            .history
            .iter()
            .rev()
            .find(|e| e.actor_id == actor_id && e.command_id == command_id)
            .cloned()
        {
            return Ok(Some(MaintenanceRestoreManifest {
                restore_id: entry.restore_id,
                command_receipt_id: entry.command_receipt_id,
                actor_id: entry.actor_id,
                command_id: entry.command_id,
                command_digest: entry.command_digest,
                backup_id: entry.backup_id,
                source_authority_generation: entry.source_authority_generation,
                state: entry.state,
                blocked_reason: entry.blocked_reason,
                updated_at_ms: entry.updated_at_ms,
                history: m.history,
            }));
        }
    }

    if config.db_path.exists()
        && let Ok(conn) = Connection::open_with_flags(
            &config.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        && let Ok(Some(found)) = conn
            .query_row(
                "SELECT source_restore_id, command_receipt_id, command_digest, backup_id,
                        source_authority_generation, state, blocked_reason, updated_at_ms
                 FROM restore_manifest_records
                 WHERE actor_id = ?1 AND command_id = ?2
                 ORDER BY rowid DESC LIMIT 1",
                rusqlite::params![actor_id, command_id.to_string()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()
    {
        let (srid, rcpt, dig, bid, sauth, st_s, br, up) = found;
        let st = match st_s.as_str() {
            "blocked" => RestoreRequestState::Blocked,
            "handed_over" => RestoreRequestState::HandedOver,
            "neutralizing" => RestoreRequestState::Neutralizing,
            "capsule_ready" => RestoreRequestState::CapsuleReady,
            "aborted" => RestoreRequestState::Aborted,
            _ => RestoreRequestState::Requested,
        };
        return Ok(Some(MaintenanceRestoreManifest {
            restore_id: srid.parse::<RestoreId>()?,
            command_receipt_id: rcpt.parse::<ReceiptId>()?,
            actor_id: actor_id.to_owned(),
            command_id,
            command_digest: dig.parse::<Sha256Digest>()?,
            backup_id: bid.parse::<BackupId>()?,
            source_authority_generation: sauth.parse::<AuthorityGeneration>()?,
            state: st,
            blocked_reason: br,
            updated_at_ms: TimestampMs::new(up),
            history: vec![],
        }));
    }

    let _ = Uuid::nil();
    Ok(None)
}
