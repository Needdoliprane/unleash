//! Writer du journal broker. Aucun appel Guard, exécuteur ou I/O externe dans ces transactions.
use crate::broker_phases::{
    BROKER_PHASE_SCHEMA_VERSION, BrokerActionSnapshot, BrokerAuthorizationCause,
    BrokerAuthorizationDecision, BrokerAuthorizationOutcome, BrokerCause, BrokerEvaluation,
    BrokerExecutor, BrokerIntent, BrokerObservationCause, BrokerPhase, BrokerPhaseCommit,
    BrokerPhaseRequest, BrokerPhaseStore,
};
use crate::commands::{DaemonStateRow, ensure_scope_not_frozen_tx, read_daemon_state_tx};
use crate::policies_secrets::{
    GuardPolicyMode, PolicyIsolationMode, SubjectKind, ensure_subject_not_blocked_tx,
    read_policy_summary_conn,
};
use crate::{StoreConfig, StoreCutpoint, StoreError};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use unleash_domain::{
    ActionId, ActionOrigin, ActionState, DaemonMode, DispatchId, Fence, LineageFence, Revision,
    Sequence, Sha256Digest, TimestampMs,
};
use unleash_protocol::{EventCursor, RetryClass, canonical_sha256};

fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}
fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("entier broker hors plage SQLite"))
}
fn phase_key(phase: &BrokerPhase) -> String {
    match phase {
        BrokerPhase::Authorization {
            cause:
                BrokerAuthorizationCause::HumanApproval {
                    decision_receipt_id,
                    ..
                },
            ..
        } => format!("authorization:approval:{decision_receipt_id}"),
        BrokerPhase::Observation {
            observation_revision,
            ..
        } => format!("observation:{}", observation_revision.as_u64()),
        _ => phase.kind().to_owned(),
    }
}
fn intent_digest(phase: &BrokerPhase) -> Sha256Digest {
    match phase {
        BrokerPhase::Authorizing { intent } => intent.intent_digest,
        BrokerPhase::Authorization { intent_digest, .. }
        | BrokerPhase::Dispatch { intent_digest, .. }
        | BrokerPhase::Observation { intent_digest, .. } => *intent_digest,
    }
}

pub(crate) fn inspect_action_conn(
    conn: &Connection,
    action: ActionId,
) -> Result<Option<BrokerActionSnapshot>, StoreError> {
    type SnapshotRow = (
        String,
        i64,
        String,
        String,
        String,
        i64,
        i64,
        Option<String>,
    );
    let row:Option<SnapshotRow> = conn.query_row(
        "SELECT a.state,a.revision,b.intent_digest,e.evaluation_id,a.authority_generation,a.daemon_epoch,a.fence,d.dispatch_id
         FROM actions a JOIN broker_intents b USING(action_id)
         JOIN broker_evaluations e ON e.action_id=a.action_id
         LEFT JOIN broker_dispatches d ON d.action_id=a.action_id
         WHERE a.action_id=?1 ORDER BY e.ordinal DESC LIMIT 1",[action.to_string()],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))
    ).optional().map_err(StoreError::from_rusqlite)?;
    row.map(
        |(state, revision, digest, evaluation, authority, epoch, fence, dispatch)| {
            let (stream,sequence):(String,i64)=conn.query_row(
                "SELECT e.stream_id,e.sequence FROM broker_phase_records p JOIN events e ON e.event_id=p.event_id
                 WHERE p.action_id=?1 ORDER BY p.action_revision DESC LIMIT 1",
                [action.to_string()],|r|Ok((r.get(0)?,r.get(1)?))
            ).map_err(StoreError::from_rusqlite)?;
            Ok(BrokerActionSnapshot {
                action_id: action,
                state: crate::recovery_ops::parse_action_state(&state)?,
                revision: Revision::new(
                    u64::try_from(revision).map_err(|_| invalid("révision broker corrompue"))?,
                ),
                intent_digest: digest.parse()?,
                evaluation_id: evaluation.parse()?,
                fence: unleash_domain::LineageFence::new(
                    authority.parse()?,
                    unleash_domain::DaemonEpoch::new(
                        u64::try_from(epoch).map_err(|_| invalid("epoch broker corrompu"))?,
                    ),
                    Fence::new(u64::try_from(fence).map_err(|_| invalid("fence broker corrompu"))?),
                ),
                dispatch_id: dispatch.map(|d| d.parse()).transpose()?,
                last_event_cursor: EventCursor::new(stream.parse()?,Sequence::new(u64::try_from(sequence).map_err(|_|invalid("séquence broker corrompue"))?)),
            })
        },
    )
    .transpose()
}

#[expect(
    clippy::too_many_lines,
    reason = "Validation du manifeste complet et de ses champs d’autorité sans dépendance vers le broker"
)]
pub(crate) fn validate_manifest(intent: &BrokerIntent) -> Result<(), StoreError> {
    const KEYS: &[&str] = &[
        "action_id",
        "subject",
        "project_id",
        "verb",
        "resource_refs",
        "payload_digest",
        "reconcile_strategy",
        "effects_class",
        "scope_revision",
        "policy_revision",
        "lineage_fence",
        "action_deadline",
        "isolation_requirements",
        "secret_refs",
        "provenance",
    ];
    if intent.manifest.len() != KEYS.len()
        || intent.manifest.keys().any(|k| !KEYS.contains(&k.as_str()))
        || canonical_sha256(&intent.manifest)? != intent.intent_digest
    {
        return Err(invalid("manifeste broker ou digest invalide"));
    }
    let ActionOrigin::ControlPlane {
        control_principal_id,
    } = &intent.origin
    else {
        return Err(invalid(
            "producteur invocation/replay et parents non qualifiés",
        ));
    };
    if control_principal_id != &intent.principal_id {
        return Err(invalid("principal broker divergent"));
    }
    let cause_ref = match intent.cause {
        BrokerCause::CommandReceipt { receipt_id } => format!("receipt:{receipt_id}"),
        BrokerCause::Event { event_id } => format!("event:{event_id}"),
        BrokerCause::Timer { .. } => return Err(invalid("producteur timer non qualifié")),
    };
    let expected_subject = json!({"kind":"control_plane","principal_id":intent.principal_id,"cause_ref":cause_ref,"invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null});
    // Les métadonnées d'autorité doivent être exactement celles du manifeste haché.
    for (key, value) in [
        ("action_id", json!(intent.action_id)),
        ("subject", expected_subject),
        ("project_id", json!(intent.project_id)),
        ("payload_digest", json!(intent.payload_digest)),
        ("scope_revision", json!(intent.scope_revision)),
        ("policy_revision", json!(intent.policy_revision)),
        ("lineage_fence", json!(intent.lineage_fence)),
        ("action_deadline", json!(intent.action_deadline)),
        ("reconcile_strategy", json!(intent.reconcile_strategy)),
    ] {
        if intent.manifest.get(key) != Some(&value) {
            return Err(invalid("autorité broker divergente du manifeste"));
        }
    }
    if intent
        .manifest
        .get("verb")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(invalid("verbe broker absent"));
    }
    let class = intent
        .manifest
        .get("effects_class")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("classe effet broker absente"))?;
    if ![
        "local_read",
        "local_write",
        "process_exec",
        "terminal_input",
        "git_local_write",
        "git_remote",
        "connector_read",
        "connector_write",
        "browser_action",
        "model_dispatch",
        "network_egress",
    ]
    .contains(&class)
    {
        return Err(invalid("classe effet broker inconnue"));
    }
    for key in ["resource_refs", "secret_refs"] {
        if !intent.manifest.get(key).is_some_and(Value::is_array) {
            return Err(invalid("références broker invalides"));
        }
    }
    if !intent
        .manifest
        .get("isolation_requirements")
        .is_some_and(Value::is_object)
        || !intent
            .manifest
            .get("provenance")
            .is_some_and(Value::is_object)
    {
        return Err(invalid("exigences/provenance broker absentes"));
    }
    Ok(())
}

fn validate_live_intent(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    intent: &BrokerIntent,
) -> Result<(), StoreError> {
    validate_live_intent_with_connect(tx, daemon, now, intent, false)
}

fn validate_live_intent_with_connect(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    intent: &BrokerIntent,
    connect_authorized: bool,
) -> Result<(), StoreError> {
    crate::backup_requests::validate_backup_intent_tx(tx, daemon, intent)?;
    if !connect_authorized && !crate::provider_probe_connect::validate_intent(tx, daemon, intent)? {
        crate::provider_probes::validate_intent(tx, daemon, intent)?;
    }
    crate::terminal_allocations::validate_intent(tx, daemon, now, intent)?;
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "daemon indisponible pour dispatch broker".to_owned(),
        ));
    }
    if intent.lineage_fence.authority_generation != daemon.authority_generation
        || intent.lineage_fence.epoch != daemon.daemon_epoch
        || intent.lineage_fence.fence != Fence::new(0)
    {
        return Err(StoreError::StaleFence {
            message: "lignée/epoch/fence de contrôle divergent".to_owned(),
        });
    }
    if intent.action_deadline.as_i64() <= now.as_i64() {
        return Err(invalid("deadline broker expirée"));
    }
    let actor: Option<String> = tx
        .query_row(
            "SELECT actor_id FROM principals WHERE principal_id=?1 AND state='active'",
            [intent.principal_id.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let actor = actor.ok_or_else(|| invalid("principal broker absent ou inactif"))?;
    ensure_subject_not_blocked_tx(
        tx,
        SubjectKind::Actor,
        intent.principal_id.as_str(),
        intent.project_id,
    )
    .map_err(|err| match err {
        StoreError::CapabilityDenied { reason, .. } => StoreError::CapabilityDenied {
            reason,
            message: "principal broker suspendu ou révoqué".to_owned(),
        },
        other => other,
    })?;
    if actor != intent.principal_id.as_str() {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, &actor, intent.project_id).map_err(
            |err| match err {
                StoreError::CapabilityDenied { reason, .. } => StoreError::CapabilityDenied {
                    reason,
                    message: "acteur broker suspendu ou révoqué".to_owned(),
                },
                other => other,
            },
        )?;
    }
    ensure_scope_not_frozen_tx(tx, intent.project_id, None, None)?;
    let scope: i64 = if let Some(project) = intent.project_id {
        tx.query_row(
            "SELECT revision FROM projects WHERE project_id=?1 AND state='active'",
            [project.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?
        .ok_or_else(|| invalid("projet broker absent ou inactif"))?
    } else {
        tx.query_row("SELECT COALESCE((SELECT revision FROM settings WHERE scope_kind='user' AND scope_id='default'),0)",[],|r|r.get(0)).map_err(StoreError::from_rusqlite)?
    };
    if scope != integer(intent.scope_revision.as_u64())? {
        return Err(invalid("révision du scope broker périmée"));
    }
    let valid_cause = match intent.cause {
        BrokerCause::CommandReceipt { receipt_id } => tx.query_row(
            "SELECT COUNT(*) FROM command_receipts c WHERE receipt_id=?1 AND actor_id=?2
             AND authority_generation=?3 AND state IN ('accepted','completed') AND response_state='full'
             AND EXISTS (SELECT 1 FROM events e WHERE e.causation_receipt_id=c.receipt_id
                 AND e.stream_id=?4 AND e.actor_id=?2 AND e.project_id IS ?5)
             AND NOT EXISTS (SELECT 1 FROM events e WHERE e.causation_receipt_id=c.receipt_id AND e.project_id IS NOT ?5)",
            params![receipt_id.to_string(), actor, daemon.authority_generation.to_string(), daemon.stream_id.to_string(), intent.project_id.map(|p|p.to_string())],
            |r| r.get::<_, i64>(0),
        ),
        BrokerCause::Event {event_id}=>tx.query_row("SELECT COUNT(*) FROM events WHERE event_id=?1 AND stream_id=?2 AND actor_id=?3 AND payload_state='full' AND project_id IS ?4",params![event_id.to_string(),daemon.stream_id.to_string(),actor,intent.project_id.map(|p|p.to_string())],|r|r.get::<_,i64>(0)),
        BrokerCause::Timer {..}=>return Err(invalid("producteur timer non qualifié")),
    }.map_err(StoreError::from_rusqlite)?;
    if valid_cause != 1 {
        return Err(invalid("cause durable broker absente ou non attribuée"));
    }
    Ok(())
}

fn policy_snapshot(
    tx: &Transaction<'_>,
    intent: &BrokerIntent,
) -> Result<(String, Sha256Digest), StoreError> {
    let summary = read_policy_summary_conn(tx, intent.project_id)?;
    let revision = summary
        .global_policy
        .as_ref()
        .map_or(0, |p| p.revision.as_u64())
        .max(
            summary
                .project_policy
                .as_ref()
                .map_or(0, |p| p.revision.as_u64()),
        );
    if revision != intent.policy_revision.as_u64() {
        return Err(invalid("révision de politique broker périmée"));
    }
    let value = json!({"schema_version":1,"global_policy":summary.global_policy,"project_policy":summary.project_policy,"guard_config":summary.guard_config});
    Ok((serde_json::to_string(&value)?, canonical_sha256(&value)?))
}

fn require_policy_unchanged(
    tx: &Transaction<'_>,
    intent: &BrokerIntent,
    evaluation: BrokerEvaluation,
) -> Result<(), StoreError> {
    let (_, digest) = policy_snapshot(tx, intent)?;
    let stored:String=tx.query_row("SELECT policy_snapshot_hash FROM broker_evaluations WHERE evaluation_id=?1 AND action_id=?2",params![evaluation.evaluation_id.to_string(),intent.action_id.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
    if stored != digest.as_prefixed_str() {
        return Err(invalid(
            "politique ou configuration Guard changée depuis intention",
        ));
    }
    Ok(())
}

fn validate_decision(
    intent: &BrokerIntent,
    evaluation: BrokerEvaluation,
    now: TimestampMs,
    decision: &BrokerAuthorizationDecision,
) -> Result<(), StoreError> {
    if decision.from_cache || decision.source_evaluation_id.is_some() {
        return Err(invalid(
            "provenance et TTL de cache non attestés au port durable",
        ));
    }
    if decision.action_id != intent.action_id
        || decision.evaluation_id != evaluation.evaluation_id
        || decision.payload_digest != intent.payload_digest
        || decision.scope_revision != intent.scope_revision
        || decision.policy_revision != intent.policy_revision
        || decision.decision_ref.is_empty()
        || decision
            .expiry
            .is_some_and(|expiry| expiry.as_i64() <= now.as_i64())
        || decision.from_cache != decision.source_evaluation_id.is_some()
    {
        return Err(invalid("décision broker divergente ou expirée"));
    }
    Ok(())
}

fn check_expected(
    current: Option<&BrokerActionSnapshot>,
    expected: Revision,
) -> Result<Revision, StoreError> {
    let revision = current.map_or(Revision::INITIAL, |r| r.revision);
    if revision != expected {
        return Err(StoreError::RevisionConflict {
            current_revision: revision,
            expected_revision: expected,
            command_id: None,
        });
    }
    let next = revision
        .as_u64()
        .checked_add(1)
        .ok_or_else(|| invalid("révision broker débordée"))?;
    integer(next)?;
    Ok(Revision::new(next))
}

pub(crate) fn commit_phase_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &BrokerPhaseRequest,
) -> Result<BrokerPhaseCommit, StoreError> {
    commit_phase_tx_inner(tx, config, now, request, None, None, None)
}
pub(crate) fn commit_terminal_observation_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &BrokerPhaseRequest,
    proof: &crate::terminal_allocations::TerminalObservationProof,
) -> Result<BrokerPhaseCommit, StoreError> {
    commit_phase_tx_inner(tx, config, now, request, Some(proof), None, None)
}

pub(crate) fn commit_connect_phase_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &BrokerPhaseRequest,
    permission: &crate::provider_probe_connect::ConnectPhasePermission,
) -> Result<BrokerPhaseCommit, StoreError> {
    commit_phase_tx_inner(
        tx,
        config,
        now,
        request,
        None,
        Some(crate::provider_probe_connect::ConnectPhaseAccess::Live(
            permission,
        )),
        None,
    )
}

pub(crate) fn commit_inventory_phase_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &BrokerPhaseRequest,
    permission: &crate::provider_probe_inventory::InventoryPhasePermission,
) -> Result<BrokerPhaseCommit, StoreError> {
    commit_phase_tx_inner(
        tx,
        config,
        now,
        request,
        None,
        Some(crate::provider_probe_connect::ConnectPhaseAccess::InventoryLive(permission)),
        None,
    )
}

#[derive(Clone, Copy)]
pub(crate) enum TransportObservation<'a> {
    Connect(&'a crate::provider_probe_connect_observation::ConnectObservationPermission),
    Inventory(&'a crate::provider_probe_inventory_observation::InventoryObservationPermission),
}

pub(crate) fn commit_transport_observation_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &BrokerPhaseRequest,
    observation: TransportObservation<'_>,
) -> Result<BrokerPhaseCommit, StoreError> {
    use crate::provider_probe_connect::ConnectPhaseAccess;
    let access = match &observation {
        TransportObservation::Connect(proof) => ConnectPhaseAccess::Observation(proof.permission()),
        TransportObservation::Inventory(proof) => {
            ConnectPhaseAccess::InventoryObservation(proof.permission())
        }
    };
    commit_phase_tx_inner(
        tx,
        config,
        now,
        request,
        None,
        Some(access),
        Some(observation),
    )
}

/// Canonical immutable phase replay lookup shared with specialized preflights.
/// The caller must validate its own authority and forbid dispatch replay first.
pub(crate) fn lookup_phase_record(
    conn: &Connection,
    daemon: &DaemonStateRow,
    request: &BrokerPhaseRequest,
) -> Result<Option<BrokerPhaseCommit>, StoreError> {
    let action = request.phase.action_id();
    let exact = json!({"schema_version":BROKER_PHASE_SCHEMA_VERSION,"evaluation":request.evaluation,"phase":request.phase});
    let digest = canonical_sha256(&exact)?;
    let key = phase_key(&request.phase);
    let prior:Option<(String,i64,i64)>=conn.query_row("SELECT phase_digest,action_revision,event_sequence FROM broker_phase_records WHERE action_id=?1 AND evaluation_id=?2 AND phase_key=?3",params![action.to_string(),request.evaluation.evaluation_id.to_string(),key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(StoreError::from_rusqlite)?;
    prior
        .map(|(stored, revision, sequence)| {
            if stored != digest.as_prefixed_str() {
                return Err(invalid("phase broker divergente pour identité existante"));
            }
            Ok(BrokerPhaseCommit {
                action_id: action,
                revision: Revision::new(
                    u64::try_from(revision).map_err(|_| invalid("révision de phase négative"))?,
                ),
                event_cursor: EventCursor::new(
                    daemon.stream_id,
                    Sequence::new(
                        u64::try_from(sequence)
                            .map_err(|_| invalid("séquence de phase négative"))?,
                    ),
                ),
                replayed: true,
            })
        })
        .transpose()
}

/// Résout la phase avant toute I/O ; un dispatch durable n'est jamais acquitté une seconde fois.
#[expect(
    clippy::too_many_lines,
    reason = "Algorithme writer fermé : identité, rejeu, CAS et quatre phases atomiques"
)]
fn commit_phase_tx_inner(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &BrokerPhaseRequest,
    terminal_proof: Option<&crate::terminal_allocations::TerminalObservationProof>,
    connect_access: Option<crate::provider_probe_connect::ConnectPhaseAccess<'_>>,
    transport_observation: Option<TransportObservation<'_>>,
) -> Result<BrokerPhaseCommit, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    if daemon.authority_generation != request.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: request.authority_generation,
            command_id: None,
        });
    }
    let action = request.phase.action_id();
    let action_authority: Option<String> = tx
        .query_row(
            "SELECT authority_generation FROM actions WHERE action_id=?1",
            [action.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    if let Some(authority) = action_authority {
        let authority = authority.parse()?;
        if authority != daemon.authority_generation {
            return Err(StoreError::StaleAuthorityGeneration {
                expected: daemon.authority_generation,
                actual: authority,
                command_id: None,
            });
        }
    }
    let connect_authorized = crate::provider_probe_connect::guard_phase_access(
        tx,
        &daemon,
        now,
        &request.phase,
        connect_access,
    )?;
    crate::provider_probes::validate_incarnation(tx, &daemon, action)?;
    crate::terminal_allocations::validate_incarnation(tx, &daemon, action)?;
    if matches!(request.phase, BrokerPhase::Dispatch { .. }) {
        let markers: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM action_dispatches WHERE action_id=?1",
                [action.to_string()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if markers != 0 {
            return Err(invalid(
                "dispatch déjà durable : réconciliation obligatoire",
            ));
        }
    }
    let key = phase_key(&request.phase);
    let exact = json!({"schema_version":BROKER_PHASE_SCHEMA_VERSION,"evaluation":request.evaluation,"phase":request.phase});
    let digest = canonical_sha256(&exact)?;
    if let Some(replayed) = lookup_phase_record(tx, &daemon, request)? {
        return Ok(replayed);
    }
    crate::terminal_allocations::guard_observation(tx, &request.phase, terminal_proof)?;
    let current = inspect_action_conn(tx, action)?;
    let revision = check_expected(current.as_ref(), request.expected_revision)?;
    let intent = if let BrokerPhase::Authorizing { intent } = &request.phase {
        intent.clone()
    } else {
        let encoded: String = tx
            .query_row(
                "SELECT intent_json FROM broker_intents WHERE action_id=?1",
                [action.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?
            .ok_or_else(|| invalid("intention broker non engagée"))?;
        serde_json::from_str(&encoded).map_err(|_| invalid("intention broker durable illisible"))?
    };
    validate_manifest(&intent)?;
    if intent_digest(&request.phase) != intent.intent_digest {
        return Err(invalid("digest intention broker divergent"));
    }
    let mut transport_completion = None;
    let (state, event_name, extra) = match &request.phase {
        BrokerPhase::Authorizing { .. } => {
            validate_live_intent_with_connect(tx, &daemon, now, &intent, connect_authorized)?;
            let (snapshot, policy_hash) = policy_snapshot(tx, &intent)?;
            let ordinal = if let Some(prior) = current.as_ref() {
                if prior.intent_digest != intent.intent_digest
                    || prior.state != ActionState::Authorizing
                    || request.evaluation.previous_evaluation_id != Some(prior.evaluation_id)
                {
                    return Err(invalid("chaîne de réévaluation broker invalide"));
                }
                let (state,backoff,ordinal):(String,Option<i64>,i64)=tx.query_row("SELECT state,retry_not_before_ms,ordinal FROM broker_evaluations WHERE evaluation_id=?1",[prior.evaluation_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(StoreError::from_rusqlite)?;
                if state != "retryable" || backoff.is_none_or(|at| at > now.as_i64()) {
                    return Err(invalid("résultat retryable durable et backoff requis"));
                }
                ordinal
                    .checked_add(1)
                    .ok_or_else(|| invalid("ordinal évaluation débordé"))?
            } else {
                if request.evaluation.previous_evaluation_id.is_some() {
                    return Err(invalid("prédécesseur inattendu pour première évaluation"));
                }
                tx.execute("INSERT INTO actions (action_id,origin_kind,control_principal_id,action_type,payload_hash,policy_revision,authority_generation,daemon_epoch,fence,state,reconcile_strategy,revision,created_at_ms,project_id,scope_key) VALUES (?1,'control_plane',?2,?3,?4,?5,?6,?7,0,'authorizing',?8,?9,?10,?11,?12)",params![action.to_string(),intent.principal_id.as_str(),intent.manifest.get("verb").and_then(Value::as_str),intent.payload_digest.as_prefixed_str(),integer(intent.policy_revision.as_u64())?,daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?,intent.reconcile_strategy.to_string(),integer(revision.as_u64())?,now.as_i64(),intent.project_id.map(|p|p.to_string()),intent.project_id.map_or_else(||"global".to_owned(),|p|p.to_string())]).map_err(StoreError::from_rusqlite)?;
                tx.execute("INSERT INTO broker_intents (action_id,intent_digest,intent_json,created_at_ms) VALUES (?1,?2,?3,?4)",params![action.to_string(),intent.intent_digest.as_prefixed_str(),serde_json::to_string(&intent)?,now.as_i64()]).map_err(StoreError::from_rusqlite)?;
                1
            };
            tx.execute("INSERT INTO broker_evaluations (evaluation_id,action_id,previous_evaluation_id,ordinal,policy_snapshot_json,policy_snapshot_hash,state) VALUES (?1,?2,?3,?4,?5,?6,'authorizing')",params![request.evaluation.evaluation_id.to_string(),action.to_string(),request.evaluation.previous_evaluation_id.map(|p|p.to_string()),ordinal,snapshot,policy_hash.as_prefixed_str()]).map_err(StoreError::from_rusqlite)?;
            (
                ActionState::Authorizing,
                "ActionAuthorizationRequested",
                json!({}),
            )
        }
        BrokerPhase::Authorization { outcome, cause, .. } => {
            let current = current
                .as_ref()
                .ok_or_else(|| invalid("action broker absente"))?;
            require_current_evaluation(tx, current, request.evaluation)?;
            validate_live_intent_with_connect(tx, &daemon, now, &intent, connect_authorized)?;
            require_policy_unchanged(tx, &intent, request.evaluation)?;
            if !matches!(cause, BrokerAuthorizationCause::Evaluation) {
                return Err(invalid(
                    "disposition humaine non encore qualifiée par reçu dédié",
                ));
            }
            let evaluation_state: String = tx
                .query_row(
                    "SELECT state FROM broker_evaluations WHERE evaluation_id=?1",
                    [request.evaluation.evaluation_id.to_string()],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            if current.state != ActionState::Authorizing || evaluation_state != "authorizing" {
                return Err(invalid("évaluation broker déjà terminée"));
            }
            let (state, eval_state, backoff, event_name) = match outcome {
                BrokerAuthorizationOutcome::Authorized { decision } => {
                    validate_decision(&intent, request.evaluation, now, decision)?;
                    tx.execute(
                        "UPDATE actions SET state='authorized',authorization_ref=?2 WHERE action_id=?1",
                        params![action.to_string(), decision.decision_ref],
                    )
                    .map_err(StoreError::from_rusqlite)?;
                    (
                        ActionState::Authorized,
                        "authorized",
                        None,
                        "ActionAuthorized",
                    )
                }
                BrokerAuthorizationOutcome::ApprovalRequired {
                    action_id,
                    decision_ref,
                    ..
                } => {
                    if *action_id != action || decision_ref.is_empty() {
                        return Err(invalid("attente approbation divergente"));
                    }
                    (
                        ActionState::ApprovalRequired,
                        "approval_required",
                        None,
                        "ActionApprovalRequired",
                    )
                }
                BrokerAuthorizationOutcome::Failed { failure } => {
                    if failure.retry_class == RetryClass::Never {
                        (
                            ActionState::Denied,
                            "denied",
                            None,
                            "ActionAuthorizationDenied",
                        )
                    } else {
                        let retryable = failure.retry_class == RetryClass::AfterBackoff;
                        if retryable != failure.retry_not_before_ms.is_some() {
                            return Err(invalid("classe de reprise/backoff broker incohérents"));
                        }
                        (
                            ActionState::Authorizing,
                            if retryable {
                                "retryable"
                            } else {
                                "authorizing"
                            },
                            failure.retry_not_before_ms.map(|at| at.as_i64()),
                            "ActionAuthorizationDeferred",
                        )
                    }
                }
            };
            tx.execute("UPDATE broker_evaluations SET state=?2,retry_not_before_ms=?3 WHERE evaluation_id=?1",params![request.evaluation.evaluation_id.to_string(),eval_state,backoff]).map_err(StoreError::from_rusqlite)?;
            (
                state,
                event_name,
                json!({"outcome_digest":canonical_sha256(&serde_json::to_value(outcome)?)?,"cause_kind":match cause {BrokerAuthorizationCause::Evaluation=>"evaluation",BrokerAuthorizationCause::HumanApproval {..}=>"human_approval"}}),
            )
        }
        BrokerPhase::Dispatch {
            decision,
            dispatch_id,
            fence,
            ..
        } => {
            crate::provider_probes::reject_unqualified_dispatch(tx, action)?;
            let current = current
                .as_ref()
                .ok_or_else(|| invalid("action broker absente"))?;
            require_current_evaluation(tx, current, request.evaluation)?;
            if current.state != ActionState::Authorized
                || *fence != current.fence
                || *fence != intent.lineage_fence
            {
                return Err(invalid("dispatch sans état autorisé/fence exact"));
            }
            validate_live_intent_with_connect(tx, &daemon, now, &intent, connect_authorized)?;
            require_policy_unchanged(tx, &intent, request.evaluation)?;
            validate_decision(&intent, request.evaluation, now, decision)?;
            let encoded:String=tx.query_row("SELECT phase_json FROM broker_phase_records WHERE action_id=?1 AND evaluation_id=?2 AND phase_key='authorization'",params![action.to_string(),request.evaluation.evaluation_id.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
            let persisted: Value = serde_json::from_str(&encoded)?;
            if persisted.pointer("/phase/outcome/decision")
                != Some(&serde_json::to_value(decision)?)
            {
                return Err(invalid("décision dispatch différente du résultat durable"));
            }
            validate_executor_profile(tx, &intent, decision)?;
            let request_hash = normalized_dispatch_request_hash(
                &intent,
                request.evaluation,
                decision,
                *dispatch_id,
                *fence,
            )?;
            crate::recovery_ops::record_action_dispatch_tx(
                tx,
                now,
                &crate::ActionDispatchInput {
                    dispatch_id: dispatch_id.to_string(),
                    action_id: action,
                    ordinal: 1,
                    adapter_id: match decision.executor {
                        BrokerExecutor::Unleash => "unleash",
                        BrokerExecutor::Guard => "guard",
                    }
                    .to_owned(),
                    request_hash,
                    provider_request_id: None,
                    transport_state: "dispatch_committed".to_owned(),
                    receipt_hash: None,
                },
            )?;
            tx.execute("INSERT INTO broker_dispatches (dispatch_id,action_id,evaluation_id,decision_json,authority_generation,daemon_epoch,fence) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![dispatch_id.to_string(),action.to_string(),request.evaluation.evaluation_id.to_string(),serde_json::to_string(decision)?,fence.authority_generation.to_string(),integer(fence.epoch.as_u64())?,integer(fence.fence.as_u64())?]).map_err(StoreError::from_rusqlite)?;
            (
                ActionState::Dispatching,
                "ActionDispatchCommitted",
                json!({"dispatch_id":dispatch_id,"decision_ref":decision.decision_ref,"fence":fence}),
            )
        }
        BrokerPhase::Observation {
            receipt,
            observation_revision,
            cause,
            ..
        } => {
            let current = current
                .as_ref()
                .ok_or_else(|| invalid("action broker absente"))?;
            require_current_evaluation(tx, current, request.evaluation)?;
            if receipt.action_id != action
                || receipt.evaluation_id != request.evaluation.evaluation_id
                || Some(receipt.dispatch_id) != current.dispatch_id
            {
                return Err(invalid("observation non corrélée au dispatch"));
            }
            let (decision,authority,epoch,fence):(String,String,i64,i64)=tx.query_row("SELECT decision_json,authority_generation,daemon_epoch,fence FROM broker_dispatches WHERE dispatch_id=?1 AND action_id=?2 AND evaluation_id=?3",params![receipt.dispatch_id.to_string(),action.to_string(),request.evaluation.evaluation_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(StoreError::from_rusqlite)?;
            let decision: BrokerAuthorizationDecision = serde_json::from_str(&decision)?;
            if receipt.decision_ref != decision.decision_ref
                || authority != current.fence.authority_generation.to_string()
                || epoch != integer(current.fence.epoch.as_u64())?
                || fence != integer(current.fence.fence.as_u64())?
            {
                return Err(invalid("reçu observation/fence divergent"));
            }
            if !matches!(cause, BrokerObservationCause::Execution) {
                return Err(StoreError::EvidenceUnavailable(
                    "preuve d'observation externe/réconciliation non qualifiée".to_owned(),
                ));
            }
            if !matches!(
                current.state,
                ActionState::Dispatching | ActionState::InFlight | ActionState::CancelRequested
            ) || !matches!(
                receipt.state,
                ActionState::ObservedSuccess
                    | ActionState::ObservedFailure
                    | ActionState::OutcomeUnknown
            ) {
                return Err(invalid("transition observation broker refusée"));
            }
            if current.fence.epoch != daemon.daemon_epoch {
                return Err(StoreError::StaleFence {
                    message: "observation exécuteur après redémarrage non qualifiée".to_owned(),
                });
            }
            let max:i64=tx.query_row("SELECT COUNT(*) FROM broker_phase_records WHERE action_id=?1 AND phase_key LIKE 'observation:%'",[action.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
            if integer(observation_revision.as_u64())? != max + 1 {
                return Err(invalid("ordinal observation broker non contigu"));
            }
            let started: i64 = tx
                .query_row(
                    "SELECT started_at_ms FROM action_dispatches WHERE dispatch_id=?1",
                    [receipt.dispatch_id.to_string()],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            let unknown_clock = transport_observation.is_some()
                && receipt.state == ActionState::OutcomeUnknown
                && config
                    .clock_uncertain
                    .load(std::sync::atomic::Ordering::Acquire);
            if !unknown_clock
                && (receipt.observed_at_ms.as_i64() < started
                    || receipt.observed_at_ms.as_i64() > now.as_i64())
            {
                return Err(invalid("horodatage observation broker invalide"));
            }
            transport_completion = match transport_observation {
                Some(TransportObservation::Connect(proof)) => Some(
                    crate::provider_probe_connect_observation::validate_completion(
                        tx, config, &daemon, now, request, proof,
                    )?,
                ),
                Some(TransportObservation::Inventory(proof)) => Some(
                    crate::provider_probe_inventory_observation::validate_completion(
                        tx, config, &daemon, now, request, proof,
                    )?,
                ),
                None => None,
            };
            // Empreinte de la preuve LOCALE complète conservée dans phase_json, aucune attribution fournisseur.
            let local_hash = canonical_sha256(&exact)?;
            crate::backup_requests::validate_backup_observation_tx(tx, &daemon, receipt)?;
            tx.execute("UPDATE action_dispatches SET ended_at_ms=?2,transport_state=?3,receipt_hash=?4 WHERE dispatch_id=?1",params![receipt.dispatch_id.to_string(),receipt.observed_at_ms.as_i64(),receipt.state.to_string(),local_hash.as_prefixed_str()]).map_err(StoreError::from_rusqlite)?;
            (
                receipt.state,
                "ActionObserved",
                json!({"dispatch_id":receipt.dispatch_id,"observation_revision":observation_revision,"state":receipt.state,"output_digest":receipt.output_digest,"cause":cause,"local_observation_digest":local_hash,"proof_provenance":"local_broker_execution_receipt"}),
            )
        }
    };
    tx.execute(
        "UPDATE actions SET state=?2,revision=?3 WHERE action_id=?1",
        params![
            action.to_string(),
            state.to_string(),
            integer(revision.as_u64())?
        ],
    )
    .map_err(StoreError::from_rusqlite)?;
    let payload = json!({"schema_version":1,"action_id":action,"evaluation_id":request.evaluation.evaluation_id,"intent_digest":intent.intent_digest,"state":state,"details":extra});
    let mut payload = payload
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("payload interne broker invalide"))?;
    if let Some(fields) = extra.as_object() {
        payload.extend(fields.clone());
    }
    if let Some(result) = terminal_proof.and_then(|proof| proof.resize_result.as_ref()) {
        payload.insert("terminal_resize".into(), serde_json::to_value(result)?);
    }
    if state == ActionState::ObservedSuccess
        && let Some(result) =
            crate::terminal_allocations::terminal_release_observation::observed_result(tx, action)?
    {
        payload.insert("terminal_release".into(), serde_json::to_value(result)?);
    }
    let (event_id, sequence) = append_phase_event_tx(
        tx,
        config,
        &daemon,
        now,
        &intent,
        revision,
        event_name,
        &Value::Object(payload),
    )?;
    if matches!(&request.phase, BrokerPhase::Authorizing { .. }) {
        crate::provider_probes::materialize(tx, &intent, &event_id, now)?;
    }
    tx.execute("INSERT INTO broker_phase_records (action_id,evaluation_id,phase_key,phase_json,phase_digest,action_revision,event_id,event_sequence) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",params![action.to_string(),request.evaluation.evaluation_id.to_string(),key,serde_json::to_string(&exact)?,digest.as_prefixed_str(),integer(revision.as_u64())?,event_id,sequence]).map_err(StoreError::from_rusqlite)?;
    if let Some(completion) = transport_completion {
        crate::provider_probe_connect_observation::finish_completion(
            tx,
            config,
            now,
            &completion,
            &event_id,
        )?;
    }
    config.cutpoints.hit(StoreCutpoint::C1BeforeCommandCommit)?;
    Ok(BrokerPhaseCommit {
        action_id: action,
        revision,
        event_cursor: EventCursor::new(
            daemon.stream_id,
            Sequence::new(u64::try_from(sequence).unwrap_or(0)),
        ),
        replayed: false,
    })
}

fn require_current_evaluation(
    tx: &Transaction<'_>,
    current: &BrokerActionSnapshot,
    evaluation: BrokerEvaluation,
) -> Result<(), StoreError> {
    let previous:Option<String>=tx.query_row("SELECT previous_evaluation_id FROM broker_evaluations WHERE evaluation_id=?1 AND action_id=?2",params![evaluation.evaluation_id.to_string(),current.action_id.to_string()],|r|r.get(0)).optional().map_err(StoreError::from_rusqlite)?.ok_or_else(||invalid("évaluation broker absente"))?;
    if current.evaluation_id != evaluation.evaluation_id
        || previous != evaluation.previous_evaluation_id.map(|p| p.to_string())
    {
        return Err(invalid(
            "évaluation broker différente de la chaîne courante",
        ));
    }
    Ok(())
}

fn validate_executor_profile(
    tx: &Transaction<'_>,
    intent: &BrokerIntent,
    decision: &BrokerAuthorizationDecision,
) -> Result<(), StoreError> {
    let policy = read_policy_summary_conn(tx, intent.project_id)?;
    if !decision.applied_obligations.is_empty()
        || decision.executor != BrokerExecutor::Unleash
        || policy.effective_isolation_mode == PolicyIsolationMode::Strict
    {
        return Err(invalid("profil exécuteur/obligations non qualifié"));
    }
    let isolation = intent
        .manifest
        .get("isolation_requirements")
        .ok_or_else(|| invalid("exigences isolation absentes"))?;
    if isolation.get("mode") != Some(&json!("standard"))
        || isolation.get("require_process_confinement") != Some(&json!(false))
        || isolation.get("required_capabilities") != Some(&json!([]))
    {
        return Err(invalid("confinement demandé non qualifié"));
    }
    if intent.manifest.get("effects_class") != Some(&json!("local_read"))
        && policy
            .guard_config
            .as_ref()
            .is_some_and(|g| g.guard_mode != GuardPolicyMode::BrokerOnly)
    {
        return Err(invalid("évaluation Guard externe non qualifiée"));
    }
    if intent.manifest.get("secret_refs") != Some(&json!([])) {
        return Err(invalid(
            "résolution références secrètes non qualifiée pour exécuteur",
        ));
    }
    Ok(())
}

pub(crate) fn validate_execution_dispatch_tx(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    action: ActionId,
    dispatch: unleash_domain::DispatchId,
) -> Result<(), StoreError> {
    let current =
        inspect_action_conn(tx, action)?.ok_or_else(|| invalid("action exécuteur absente"))?;
    if current.state != ActionState::Dispatching
        || current.dispatch_id != Some(dispatch)
        || current.fence.authority_generation != daemon.authority_generation
        || current.fence.epoch != daemon.daemon_epoch
    {
        return Err(invalid("marqueur exécuteur ou lignée non courants"));
    }
    let encoded: String = tx
        .query_row(
            "SELECT intent_json FROM broker_intents WHERE action_id=?1",
            [action.to_string()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let intent: BrokerIntent =
        serde_json::from_str(&encoded).map_err(|_| invalid("intention exécuteur illisible"))?;
    validate_manifest(&intent)?;
    validate_live_intent(tx, daemon, now, &intent)?;
    let (decision,auth,epoch,fence):(String,String,i64,i64)=tx.query_row("SELECT decision_json,authority_generation,daemon_epoch,fence FROM broker_dispatches WHERE dispatch_id=?1 AND action_id=?2 AND evaluation_id=?3",params![dispatch.to_string(),action.to_string(),current.evaluation_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(StoreError::from_rusqlite)?;
    if auth != daemon.authority_generation.to_string()
        || epoch != integer(daemon.daemon_epoch.as_u64())?
        || fence != integer(current.fence.fence.as_u64())?
    {
        return Err(invalid("barrière exécuteur divergente du marqueur"));
    }
    let decision: BrokerAuthorizationDecision =
        serde_json::from_str(&decision).map_err(|_| invalid("décision exécuteur illisible"))?;
    let previous: Option<String> = tx
        .query_row(
            "SELECT previous_evaluation_id FROM broker_evaluations WHERE evaluation_id=?1",
            [current.evaluation_id.to_string()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let evaluation = BrokerEvaluation {
        evaluation_id: current.evaluation_id,
        previous_evaluation_id: previous.map(|v| v.parse()).transpose()?,
    };
    require_current_evaluation(tx, &current, evaluation)?;
    require_policy_unchanged(tx, &intent, evaluation)?;
    validate_decision(&intent, evaluation, now, &decision)?;
    validate_executor_profile(tx, &intent, &decision)?;
    let expected =
        normalized_dispatch_request_hash(&intent, evaluation, &decision, dispatch, current.fence)?;
    require_normalized_dispatch_hash(tx, action, dispatch, expected)
}

fn normalized_dispatch_request_hash(
    intent: &BrokerIntent,
    evaluation: BrokerEvaluation,
    decision: &BrokerAuthorizationDecision,
    dispatch: DispatchId,
    fence: LineageFence,
) -> Result<Sha256Digest, StoreError> {
    Ok(canonical_sha256(
        &json!({"schema_version":1,"action_id":intent.action_id,
        "evaluation":evaluation,"intent_digest":intent.intent_digest,"payload_digest":intent.payload_digest,
        "decision":decision,"dispatch_id":dispatch,"fence":fence}),
    )?)
}

fn require_normalized_dispatch_hash(
    tx: &Transaction<'_>,
    action: ActionId,
    dispatch: DispatchId,
    expected: Sha256Digest,
) -> Result<(), StoreError> {
    let valid:i64=tx.query_row("SELECT COUNT(*) FROM action_dispatches WHERE dispatch_id=?1 AND action_id=?2 AND request_hash=?3 AND transport_state='dispatch_committed' AND ended_at_ms IS NULL",params![dispatch.to_string(),action.to_string(),expected.as_prefixed_str()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
    if valid != 1 {
        return Err(invalid("requête normalisée du dispatch divergente"));
    }
    Ok(())
}

struct RecordedDispatchRow {
    decision: String,
    evaluation: String,
    previous_evaluation: Option<String>,
    authority: String,
    epoch: i64,
    fence: i64,
}

/// Checks immutable dispatch attribution after I/O, without granting a new effect.
/// Current policy, decision expiry and active subject checks belong before dispatch.
pub(crate) fn validate_recorded_dispatch_binding_tx(
    tx: &Transaction<'_>,
    intent: &BrokerIntent,
    dispatch: DispatchId,
) -> Result<(), StoreError> {
    let recorded=tx.query_row(
        "SELECT d.decision_json,d.evaluation_id,e.previous_evaluation_id,d.authority_generation,d.daemon_epoch,d.fence FROM broker_dispatches d JOIN broker_evaluations e ON e.evaluation_id=d.evaluation_id AND e.action_id=d.action_id WHERE d.dispatch_id=?1 AND d.action_id=?2",
        params![dispatch.to_string(),intent.action_id.to_string()],|row|Ok(RecordedDispatchRow {
            decision:row.get(0)?,evaluation:row.get(1)?,previous_evaluation:row.get(2)?,authority:row.get(3)?,epoch:row.get(4)?,fence:row.get(5)?,
        }),
    )?;
    let evaluation = BrokerEvaluation {
        evaluation_id: recorded.evaluation.parse()?,
        previous_evaluation_id: recorded
            .previous_evaluation
            .map(|id| id.parse())
            .transpose()?,
    };
    let fence = LineageFence::new(
        recorded.authority.parse()?,
        unleash_domain::DaemonEpoch::new(
            u64::try_from(recorded.epoch).map_err(|_| invalid("époque du marqueur négative"))?,
        ),
        Fence::new(
            u64::try_from(recorded.fence).map_err(|_| invalid("fence du marqueur négatif"))?,
        ),
    );
    let decision: BrokerAuthorizationDecision = serde_json::from_str(&recorded.decision)?;
    if fence != intent.lineage_fence
        || decision.action_id != intent.action_id
        || decision.evaluation_id != evaluation.evaluation_id
        || decision.payload_digest != intent.payload_digest
        || decision.scope_revision != intent.scope_revision
        || decision.policy_revision != intent.policy_revision
    {
        return Err(invalid("faisceau du dispatch enregistré divergent"));
    }
    let (encoded,digest):(String,String)=tx.query_row(
        "SELECT phase_json,phase_digest FROM broker_phase_records WHERE action_id=?1 AND evaluation_id=?2 AND phase_key='authorization'",
        params![intent.action_id.to_string(),evaluation.evaluation_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)),
    )?;
    let authorized: Value = serde_json::from_str(&encoded)?;
    if canonical_sha256(&authorized)?.as_prefixed_str() != digest
        || authorized.get("evaluation") != Some(&serde_json::to_value(evaluation)?)
        || authorized.pointer("/phase/intent_digest")
            != Some(&serde_json::to_value(intent.intent_digest)?)
        || authorized.pointer("/phase/outcome/decision") != Some(&serde_json::to_value(&decision)?)
    {
        return Err(invalid(
            "décision du dispatch différente de l'autorisation durable",
        ));
    }
    let expected =
        normalized_dispatch_request_hash(intent, evaluation, &decision, dispatch, fence)?;
    require_normalized_dispatch_hash(tx, intent.action_id, dispatch, expected)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Contexte explicite de l’événement interne atomique, sans commande fictive"
)]
pub(crate) fn append_phase_event_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    intent: &BrokerIntent,
    revision: Revision,
    event_name: &str,
    payload: &Value,
) -> Result<(String, i64), StoreError> {
    let event = config.id_generator.new_uuid().to_string();
    tx.execute("INSERT INTO events (event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,command_actor_id,command_id,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES (?1,?2,'action',?3,?4,(SELECT COALESCE(MAX(event_ordinal)+1,0) FROM events WHERE aggregate_type='action' AND aggregate_id=?3 AND aggregate_revision=?4),?5,1,?6,?7,?7,?8,?8,?9,'full',?10,'restricted',NULL,NULL,'action',?3,NULL,?3)",params![event,daemon.stream_id.to_string(),intent.action_id.to_string(),integer(revision.as_u64())?,event_name,intent.project_id.map(|p|p.to_string()),intent.principal_id.as_str(),now.as_i64(),serde_json::to_string(payload)?,canonical_sha256(payload)?.as_prefixed_str()]).map_err(StoreError::from_rusqlite)?;
    let sequence = tx.last_insert_rowid();
    tx.execute("INSERT INTO outbox (event_id,destination,state,delivery_attempt,next_attempt_at_ms,authority_generation,consumer_version,delivery_key,rearm_count) VALUES (?1,'clients','pending',0,?2,?3,?4,?5,0)",params![event,now.as_i64(),daemon.authority_generation.to_string(),config.consumer_version,format!("{event}:clients")]).map_err(StoreError::from_rusqlite)?;
    Ok((event, sequence))
}

impl BrokerPhaseStore for crate::UnleashStore {
    fn commit_broker_phase(
        &self,
        request: BrokerPhaseRequest,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        Self::commit_broker_phase(self, request)
    }
    fn inspect_broker_action(
        &self,
        action_id: ActionId,
    ) -> Result<Option<BrokerActionSnapshot>, StoreError> {
        Self::inspect_broker_action(self, action_id)
    }
}
