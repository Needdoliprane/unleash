//! Réservations causales de sauvegarde ; aucun effet filesystem dans le writer.
use crate::commands::{
    AggregateKind, CommandMutation, CommitCommandRequest, DaemonStateRow, EventAppendSpec,
    ensure_scope_not_frozen_tx, read_daemon_state_tx,
};
use crate::policies_secrets::{
    SubjectKind, ensure_subject_not_blocked_tx, read_policy_summary_conn,
};
use crate::{BackupJobState, BackupManifest, StoreConfig, StoreError};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use unleash_domain::{
    ActionId, ActorId, AuthorityGeneration, BackupId, DaemonEpoch, DaemonMode, DispatchId, Fence,
    LineageFence, ReceiptId, Revision, Sequence, Sha256Digest, TimestampMs,
};
use unleash_protocol::{EventCausationKind, EventCursor, Sensitivity, canonical_sha256};

/// Verbe du producteur interne `RequestBackup` ; sa classe est toujours `local_write`.
pub const REQUEST_BACKUP_VERB: &str = "backup.prepare";

/// État durable du droit d'exécuter une demande de sauvegarde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupRequestState {
    /// Reçu et job réservés, sans appel Guard ni écriture de sauvegarde.
    Reserved,
    /// Un orchestrateur a pris la demande une fois ; aucune reprise implicite.
    Claimed,
    /// Marqueur vérifié et pin durable engagés avant l'I/O.
    Executing,
    /// Manifeste vérifié et complétion SQL engagés.
    Completed,
    /// Issue nécessitant une inspection ou une décision explicite.
    Blocked,
}

/// Motif fermé d'un blocage prouvé avant tout marqueur d'effet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupBlockReason {
    /// Profil Guard demandé et non qualifié pour le producteur M1.
    GuardProfileUnavailable,
    /// Confinement strict demandé et non qualifié pour le producteur M1.
    IsolationProfileUnavailable,
    /// Refus terminal déjà engagé dans le journal broker.
    AuthorizationDenied,
}

impl BackupBlockReason {
    /// Valeur SQL/wire fermée, sans texte externe.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GuardProfileUnavailable => "guard_profile_unavailable",
            Self::IsolationProfileUnavailable => "isolation_profile_unavailable",
            Self::AuthorizationDenied => "authorization_denied",
        }
    }
}

/// Projection interne authentique ; aucun payload client ne fournit ces paramètres.
#[derive(Clone, PartialEq, Eq)]
pub struct BackupRequestRecord {
    /// Identité stable de la sauvegarde.
    pub backup_id: BackupId,
    /// Action allouée une seule fois dans la réservation.
    pub action_id: ActionId,
    /// Acteur de la session authentifiée ayant engagé la commande.
    pub actor_id: ActorId,
    /// Principal résolu en base, jamais créé depuis le DTO.
    pub principal_id: ActorId,
    /// Vrai reçu `RequestBackup`.
    pub receipt_id: ReceiptId,
    /// Lignée immuable de la réservation.
    pub authority_generation: AuthorityGeneration,
    /// État du claim/exécution.
    pub state: BackupRequestState,
    /// CAS de la demande interne.
    pub revision: Revision,
    /// Destination choisie par le serveur sous sa racine privée.
    pub destination_dir: PathBuf,
    /// Digest canonique du document retourné par `action_payload`.
    pub payload_digest: Sha256Digest,
    /// Scope engagé lors du claim.
    pub scope_revision: Revision,
    /// Politique engagée lors du claim.
    pub policy_revision: Revision,
    /// Lignée/epoch de création ou du claim, fence contrôle nul.
    pub lineage_fence: LineageFence,
    /// État réel du job, None si son historique a été retiré.
    pub job_state: Option<BackupJobState>,
    /// État broker réel, distinct de la complétion locale du job.
    pub action_state: Option<unleash_domain::ActionState>,
    /// Date de la réservation initiale.
    pub created_at_ms: TimestampMs,
    /// Dernier événement réellement associé au reçu de réservation.
    pub last_event_cursor: EventCursor,
    /// Dispatch consommé, si l'exécution a commencé.
    pub dispatch_id: Option<DispatchId>,
    /// Digest du manifeste complet réellement engagé, si disponible.
    pub manifest_digest: Option<Sha256Digest>,
    /// Manifeste exact attesté à la complétion SQL, sans prétendre une nouvelle vérification FS.
    pub manifest: Option<BackupManifest>,
    /// Barrière de sauvegarde effectivement held dans cette lecture.
    pub barrier_pin_held: bool,
    /// Nombre réel de pins d'objets held appartenant au job.
    pub held_object_pins: u64,
    /// Motif attesté avant dispatch, sans classifier un effet inconnu.
    pub block_reason: Option<BackupBlockReason>,
}

impl core::fmt::Debug for BackupRequestRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BackupRequestRecord")
            .field("backup_id", &self.backup_id)
            .field("action_id", &self.action_id)
            .field("receipt_id", &self.receipt_id)
            .field("state", &self.state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl BackupRequestRecord {
    /// Document interne exact de l'effet, distinct du payload wire fermé `{}`.
    #[must_use]
    pub fn action_payload(&self) -> Value {
        action_payload(
            self.backup_id,
            self.action_id,
            self.receipt_id,
            &self.destination_dir,
        )
    }
}

/// Exécution exclusivement liée à une réservation et un marqueur déjà durables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecuteReservedBackupRequest {
    /// Job réservé.
    pub backup_id: BackupId,
    /// Action réservée.
    pub action_id: ActionId,
    /// Marqueur exact, jamais substitué par un second dispatch.
    pub dispatch_id: DispatchId,
    /// Autorité actuelle.
    pub authority_generation: AuthorityGeneration,
}

/// Résultat local vérifié, à observer via le journal broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedBackup {
    /// Manifeste complet réellement écrit et vérifié.
    pub manifest: BackupManifest,
    /// SHA-256 canonique RFC 8785 du manifeste complet, engagé avec le job.
    pub output_digest: Sha256Digest,
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}
fn as_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("révision sauvegarde hors plage"))
}
fn action_payload(
    backup: BackupId,
    action: ActionId,
    receipt: ReceiptId,
    destination: &std::path::Path,
) -> Value {
    json!({"schema_version":1,"backup_id":backup,"action_id":action,"receipt_id":receipt,"destination_dir":destination,"coverage":"db_and_objects","retention_tier":"manual"})
}

pub(crate) fn validate_request_backup_envelope(
    request: &CommitCommandRequest,
) -> Result<(), StoreError> {
    match &request.mutation {
        CommandMutation::RequestBackup { backup_id } => {
            if request.envelope.command_type != "RequestBackup"
                || request.envelope.aggregate_id != backup_id.as_uuid()
                || backup_id.as_uuid().is_nil()
                || request.envelope.expected_revision != Revision::INITIAL
                || request.envelope.project_id.is_some()
                || !request.envelope.payload.is_empty()
                || request.target_aggregate != AggregateKind::BackupJob
                || request
                    .envelope
                    .actor_ref
                    .as_ref()
                    .is_some_and(|a| a != request.actor_id.as_str())
                || !request.events.is_empty()
                || !request.capacity_reservations.is_empty()
                || !request.budget_reservations.is_empty()
            {
                return Err(invalid(
                    "enveloppe RequestBackup divergente du contrat fermé",
                ));
            }
        }
        _ if request.envelope.command_type == "RequestBackup" => {
            return Err(invalid(
                "mutation RequestBackup divergente de son enveloppe",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn current_revisions(tx: &Transaction<'_>) -> Result<(Revision, Revision), StoreError> {
    let scope:i64=tx.query_row("SELECT COALESCE((SELECT revision FROM settings WHERE scope_kind='user' AND scope_id='default'),0)",[],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
    let policy = read_policy_summary_conn(tx, None)?
        .global_policy
        .map_or(Revision::INITIAL, |p| p.revision);
    Ok((
        Revision::new(u64::try_from(scope).map_err(|_| invalid("scope sauvegarde corrompu"))?),
        policy,
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Identités de commande et événement explicites dans la transaction atomique"
)]
pub(crate) fn reserve_backup_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    backup: BackupId,
    receipt: ReceiptId,
    actor: &ActorId,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "RequestBackup exige le daemon ready".to_owned(),
        ));
    }
    ensure_scope_not_frozen_tx(tx, None, Some("global"), None)?;
    let count:i64=tx.query_row("SELECT (SELECT COUNT(*) FROM backup_jobs WHERE backup_id=?1)+(SELECT COUNT(*) FROM backup_requests WHERE backup_id=?1)+(SELECT COUNT(*) FROM command_receipts WHERE aggregate_id=?1)",[backup.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
    if count != 0 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(1),
            expected_revision: Revision::INITIAL,
            command_id: None,
        });
    }
    let principal:Option<String>=tx.query_row("SELECT principal_id FROM principals WHERE actor_id=?1 AND state='active' AND principal_kind IN ('user','system','service')",[actor.as_str()],|r|r.get(0)).optional().map_err(StoreError::from_rusqlite)?;
    let principal =
        principal.ok_or_else(|| invalid("principal authentifié de sauvegarde absent"))?;
    ensure_principal_available(tx, &principal, actor.as_str())?;
    let action = ActionId::from_uuid(config.id_generator.new_uuid());
    let destination = config.backups_dir.join(backup.to_string());
    let payload_digest = canonical_sha256(&action_payload(backup, action, receipt, &destination))?;
    let (scope, policy) = current_revisions(tx)?;
    tx.execute("INSERT INTO backup_jobs (backup_id,state,source_authority_generation,coverage,destination_path,retention_tier,created_at_ms,revision) VALUES (?1,'preparing',?2,'db_and_objects',?3,'manual',?4,1)",params![backup.to_string(),daemon.authority_generation.to_string(),destination.to_string_lossy(),now.as_i64()]).map_err(StoreError::from_rusqlite)?;
    tx.execute("INSERT INTO backup_requests (backup_id,receipt_id,action_id,actor_id,principal_id,authority_generation,daemon_epoch,scope_revision,policy_revision,destination_path,payload_digest,state,revision,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'reserved',1,?12)",params![backup.to_string(),receipt.to_string(),action.to_string(),actor.as_str(),principal,daemon.authority_generation.to_string(),as_i64(daemon.daemon_epoch.as_u64())?,as_i64(scope.as_u64())?,as_i64(policy.as_u64())?,destination.to_string_lossy(),payload_digest.as_prefixed_str(),now.as_i64()]).map_err(StoreError::from_rusqlite)?;
    let payload = json!({"backup_id":backup,"action_id":action,"receipt_id":receipt,"state":"preparing","request_state":"reserved","payload_digest":payload_digest});
    events.push(EventAppendSpec {
        event_type: "BackupRequested".to_owned(),
        aggregate_type: AggregateKind::BackupJob,
        aggregate_id: backup.as_uuid(),
        aggregate_revision: Revision::new(1),
        project_id: None,
        correlation_id: receipt.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: payload
            .as_object()
            .cloned()
            .ok_or_else(|| invalid("payload sauvegarde interne invalide"))?,
        sensitivity: Sensitivity::Restricted,
        outbox_destinations: vec!["clients".to_owned()],
    });
    Ok(Revision::new(1))
}

fn ensure_principal_available(
    tx: &Transaction<'_>,
    principal: &str,
    actor: &str,
) -> Result<(), StoreError> {
    for subject in [principal, actor] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, subject, None).map_err(|error| {
            match error {
                StoreError::CapabilityDenied { reason, .. } => StoreError::CapabilityDenied {
                    reason,
                    message: "principal de sauvegarde suspendu ou révoqué".to_owned(),
                },
                other => other,
            }
        })?;
    }
    Ok(())
}

pub(crate) fn validate_backup_intent_tx(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    intent: &crate::BrokerIntent,
) -> Result<(), StoreError> {
    let backup: Option<String> = tx
        .query_row(
            "SELECT backup_id FROM backup_requests WHERE action_id=?1",
            [intent.action_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let receipt = if let crate::BrokerCause::CommandReceipt { receipt_id } = intent.cause {
        Some(receipt_id)
    } else {
        None
    };
    let source_type: Option<String> = if let Some(receipt) = receipt {
        tx.query_row(
            "SELECT command_type FROM command_receipts WHERE receipt_id=?1",
            [receipt.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?
    } else {
        None
    };
    if backup.is_none()
        && source_type.as_deref() != Some("RequestBackup")
        && intent.manifest.get("verb").and_then(Value::as_str) != Some(REQUEST_BACKUP_VERB)
    {
        return Ok(());
    }
    let backup: BackupId = backup
        .ok_or_else(|| invalid("réservation RequestBackup absente pour action"))?
        .parse()?;
    let record = read_backup_request_conn(tx, backup)?
        .ok_or_else(|| invalid("réservation RequestBackup illisible"))?;
    if record.authority_generation != daemon.authority_generation
        || record.lineage_fence != intent.lineage_fence
        || record.principal_id != intent.principal_id
        || record.receipt_id
            != receipt.ok_or_else(|| invalid("RequestBackup exige sa vraie cause receipt"))?
        || record.state != BackupRequestState::Claimed
        || record.job_state != Some(BackupJobState::Preparing)
        || record.payload_digest != intent.payload_digest
        || record.scope_revision != intent.scope_revision
        || record.policy_revision != intent.policy_revision
        || intent.project_id.is_some()
        || intent.manifest.get("verb") != Some(&json!(REQUEST_BACKUP_VERB))
        || intent.manifest.get("effects_class") != Some(&json!("local_write"))
        || intent.manifest.get("resource_refs")
            != Some(&json!([{"kind":"path","value":record.destination_dir}]))
        || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
        || canonical_sha256(&record.action_payload())? != intent.payload_digest
    {
        return Err(invalid(
            "intention RequestBackup divergente de sa réservation authentique",
        ));
    }
    let job:i64=tx.query_row("SELECT COUNT(*) FROM backup_jobs WHERE backup_id=?1 AND state='preparing' AND source_authority_generation=?2 AND destination_path=?3 AND coverage='db_and_objects' AND retention_tier='manual'",params![backup.to_string(),daemon.authority_generation.to_string(),record.destination_dir.to_string_lossy()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
    if job != 1 {
        return Err(invalid("job RequestBackup divergent"));
    }
    Ok(())
}

pub(crate) fn validate_backup_observation_tx(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    receipt: &crate::BrokerActionReceipt,
) -> Result<(), StoreError> {
    let backup: Option<String> = tx
        .query_row(
            "SELECT backup_id FROM backup_requests WHERE action_id=?1",
            [receipt.action_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some(backup) = backup else {
        return Ok(());
    };
    let record = read_backup_request_conn(tx, backup.parse()?)?
        .ok_or_else(|| invalid("demande observation sauvegarde absente"))?;
    if receipt.state == unleash_domain::ActionState::ObservedSuccess {
        if record.authority_generation != daemon.authority_generation
            || record.dispatch_id != Some(receipt.dispatch_id)
            || record.state != BackupRequestState::Completed
            || record.job_state != Some(BackupJobState::Complete)
            || record.manifest_digest.is_none()
            || record.manifest_digest != receipt.output_digest
        {
            return Err(invalid(
                "succès sauvegarde sans job/manifeste complets corrélés",
            ));
        }
    } else if receipt.state != unleash_domain::ActionState::OutcomeUnknown {
        return Err(invalid("issue sauvegarde sans preuve terminale qualifiée"));
    }
    Ok(())
}

type RequestRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    i64,
    String,
    i64,
    Option<String>,
);
pub(crate) fn read_backup_request_conn(
    conn: &Connection,
    backup: BackupId,
) -> Result<Option<BackupRequestRecord>, StoreError> {
    let row:Option<RequestRow>=conn.query_row(
        "SELECT r.action_id,r.actor_id,r.principal_id,r.receipt_id,r.authority_generation,r.daemon_epoch,r.scope_revision,r.policy_revision,r.destination_path,r.payload_digest,r.state,r.revision,r.dispatch_id,r.manifest_digest,r.created_at_ms,e.stream_id,e.sequence,j.state
         FROM backup_requests r JOIN command_receipts c ON c.receipt_id=r.receipt_id
         JOIN events e ON e.sequence=(SELECT MAX(sequence) FROM events WHERE aggregate_type='backup_job' AND aggregate_id=r.backup_id) LEFT JOIN backup_jobs j USING(backup_id) WHERE r.backup_id=?1",
        [backup.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?,r.get(15)?,r.get(16)?,r.get(17)?))
    ).optional().map_err(StoreError::from_rusqlite)?;
    row.map(
        |(
            action,
            actor,
            principal,
            receipt,
            authority,
            epoch,
            scope,
            policy,
            destination,
            digest,
            state,
            revision,
            dispatch,
            stored_manifest_digest,
            created,
            stream,
            sequence,
            job,
        )| {
            let authority = authority.parse()?;
            let state = match state.as_str() {
                "reserved" => BackupRequestState::Reserved,
                "claimed" => BackupRequestState::Claimed,
                "executing" => BackupRequestState::Executing,
                "completed" => BackupRequestState::Completed,
                "blocked" => BackupRequestState::Blocked,
                _ => return Err(invalid("état requête sauvegarde corrompu")),
            };
            let job = job
                .map(|state| match state.as_str() {
                    "preparing" => Ok(BackupJobState::Preparing),
                    "copying" => Ok(BackupJobState::Copying),
                    "verifying" => Ok(BackupJobState::Verifying),
                    "complete" => Ok(BackupJobState::Complete),
                    "aborted" => Ok(BackupJobState::Aborted),
                    _ => Err(invalid("état job sauvegarde corrompu")),
                })
                .transpose()?;
            let unsigned =
                |v| u64::try_from(v).map_err(|_| invalid("entier requête sauvegarde corrompu"));
        let manifest_json:Option<String>=conn.query_row("SELECT manifest_json FROM backup_requests WHERE backup_id=?1",[backup.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        let manifest:Option<BackupManifest>=manifest_json.map(|m|serde_json::from_str(&m)).transpose().map_err(|_|invalid("manifeste sauvegarde SQL illisible"))?;
        let manifest_digest:Option<Sha256Digest>=stored_manifest_digest.map(|v|v.parse()).transpose()?;
        if (state==BackupRequestState::Completed)!=manifest.is_some() || manifest.is_some()!=manifest_digest.is_some() {return Err(invalid("manifeste absent ou prématuré pour état de demande"));}
        if let Some(manifest)=&manifest
            && (manifest.backup_id!=backup || manifest.state!=BackupJobState::Complete || manifest.source_authority_generation!=authority || Some(canonical_sha256(&serde_json::to_value(manifest)?)?)!=manifest_digest) {return Err(invalid("manifeste sauvegarde SQL divergent"));}
        let (barrier,objects):(i64,i64)=conn.query_row("SELECT COALESCE(SUM(CASE WHEN pin_kind='backup_barrier' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN pin_kind='backup' THEN 1 ELSE 0 END),0) FROM object_pins WHERE owner_kind='backup_job' AND owner_id=?1 AND state='held'",[backup.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).map_err(StoreError::from_rusqlite)?;
        let has_reason:i64=conn.query_row("SELECT COUNT(*) FROM pragma_table_info('backup_requests') WHERE name='block_reason'",[],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        let reason:Option<String>=if has_reason==0 {None} else {conn.query_row("SELECT block_reason FROM backup_requests WHERE backup_id=?1",[backup.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?};
        let block_reason=reason.map(|v|match v.as_str() {"guard_profile_unavailable"=>Ok(BackupBlockReason::GuardProfileUnavailable),"isolation_profile_unavailable"=>Ok(BackupBlockReason::IsolationProfileUnavailable),"authorization_denied"=>Ok(BackupBlockReason::AuthorizationDenied),_=>Err(invalid("motif sauvegarde SQL inconnu"))}).transpose()?;
        let action_state:Option<String>=conn.query_row("SELECT state FROM actions WHERE action_id=?1",[action.as_str()],|r|r.get(0)).optional().map_err(StoreError::from_rusqlite)?;
        let action_state=action_state.map(|v|crate::recovery_ops::parse_action_state(&v)).transpose()?;
        Ok(BackupRequestRecord {
                backup_id: backup,
                action_id: action.parse()?,
                actor_id: ActorId::new(actor)?,
                principal_id: ActorId::new(principal)?,
                receipt_id: receipt.parse()?,
                authority_generation: authority,
                state,
                revision: Revision::new(unsigned(revision)?),
                destination_dir: PathBuf::from(destination),
                payload_digest: digest.parse()?,
                scope_revision: Revision::new(unsigned(scope)?),
                policy_revision: Revision::new(unsigned(policy)?),
                lineage_fence: LineageFence::new(
                    authority,
                    DaemonEpoch::new(unsigned(epoch)?),
                    Fence::new(0),
                ),
                job_state: job,
                action_state,
                created_at_ms: TimestampMs::new(created),
                last_event_cursor: EventCursor::new(
                    stream.parse()?,
                    Sequence::new(unsigned(sequence)?),
                ),
                dispatch_id: dispatch.map(|v| v.parse()).transpose()?,
            manifest_digest,
            manifest,
            barrier_pin_held:barrier>0,
            held_object_pins:unsigned(objects)?,
            block_reason,
            })
        },
    )
    .transpose()
}

pub(crate) fn claim_backup_request_tx(
    tx: &Transaction<'_>,
    backup: BackupId,
    authority: AuthorityGeneration,
) -> Result<BackupRequestRecord, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    if daemon.authority_generation != authority {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: authority,
            command_id: None,
        });
    }
    let record = read_backup_request_conn(tx, backup)?
        .ok_or_else(|| StoreError::NotFound("demande de sauvegarde".to_owned()))?;
    if record.authority_generation != authority {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: authority,
            actual: record.authority_generation,
            command_id: None,
        });
    }
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "claim sauvegarde exige ready".to_owned(),
        ));
    }
    if record.state != BackupRequestState::Reserved
        || record.job_state != Some(BackupJobState::Preparing)
    {
        return Err(StoreError::RecoveryInProgress(
            "demande déjà réclamée : inspection requise".to_owned(),
        ));
    }
    let valid:i64=tx.query_row("SELECT COUNT(*) FROM principals p JOIN command_receipts c ON c.actor_id=p.actor_id WHERE p.principal_id=?1 AND p.actor_id=?2 AND p.state='active' AND c.receipt_id=?3 AND c.command_type='RequestBackup' AND c.aggregate_id=?4 AND c.authority_generation=?5 AND c.state='accepted'",params![record.principal_id.as_str(),record.actor_id.as_str(),record.receipt_id.to_string(),backup.to_string(),authority.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
    if valid != 1 {
        return Err(invalid("cause/principal de sauvegarde non attestés"));
    }
    ensure_principal_available(tx, record.principal_id.as_str(), record.actor_id.as_str())?;
    ensure_scope_not_frozen_tx(tx, None, Some("global"), None)?;
    let (scope, policy) = current_revisions(tx)?;
    let changed=tx.execute("UPDATE backup_requests SET state='claimed',revision=revision+1,daemon_epoch=?2,scope_revision=?3,policy_revision=?4 WHERE backup_id=?1 AND state='reserved' AND revision=?5",params![backup.to_string(),as_i64(daemon.daemon_epoch.as_u64())?,as_i64(scope.as_u64())?,as_i64(policy.as_u64())?,as_i64(record.revision.as_u64())?]).map_err(StoreError::from_rusqlite)?;
    if changed != 1 {
        return Err(StoreError::RecoveryInProgress(
            "claim concurrent de sauvegarde".to_owned(),
        ));
    }
    read_backup_request_conn(tx, backup)?
        .ok_or_else(|| invalid("demande sauvegarde perdue après claim"))
}

pub(crate) struct ConsumedBackupRequest {
    pub record: BackupRequestRecord,
    pub barrier_pin_id: String,
}

pub(crate) fn consume_backup_request_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: ExecuteReservedBackupRequest,
) -> Result<ConsumedBackupRequest, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    if daemon.authority_generation != request.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: request.authority_generation,
            command_id: None,
        });
    }
    let record = read_backup_request_conn(tx, request.backup_id)?
        .ok_or_else(|| invalid("réservation exécuteur sauvegarde absente"))?;
    if record.authority_generation != daemon.authority_generation
        || record.lineage_fence.epoch != daemon.daemon_epoch
        || record.action_id != request.action_id
        || record.state != BackupRequestState::Claimed
        || record.dispatch_id.is_some()
        || record.job_state != Some(BackupJobState::Preparing)
    {
        return Err(invalid(
            "demande sauvegarde déjà consommée ou identité divergente",
        ));
    }
    crate::broker_writer::validate_execution_dispatch_tx(
        tx,
        &daemon,
        now,
        request.action_id,
        request.dispatch_id,
    )?;
    let deleting: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM object_records WHERE state='deleting'",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if deleting != 0 {
        return Err(StoreError::RecoveryInProgress(
            "suppression objet en cours avant sauvegarde".to_owned(),
        ));
    }
    let barrier = config.id_generator.new_uuid().to_string();
    tx.execute("INSERT INTO object_pins (pin_id,pin_kind,object_hash,object_generation,owner_kind,owner_id,authority_generation,state,created_at_ms,released_at_ms) VALUES (?1,'backup_barrier',NULL,NULL,'backup_job',?2,?3,'held',?4,NULL)",params![barrier,request.backup_id.to_string(),daemon.authority_generation.to_string(),now.as_i64()]).map_err(StoreError::from_rusqlite)?;
    let changed=tx.execute("UPDATE backup_requests SET state='executing',revision=revision+1,dispatch_id=?2,barrier_pin_id=?3 WHERE backup_id=?1 AND action_id=?4 AND state='claimed' AND revision=?5 AND dispatch_id IS NULL",params![request.backup_id.to_string(),request.dispatch_id.to_string(),barrier,request.action_id.to_string(),as_i64(record.revision.as_u64())?]).map_err(StoreError::from_rusqlite)?;
    if changed != 1 {
        return Err(invalid("consommation concurrente de sauvegarde"));
    }
    let record = read_backup_request_conn(tx, request.backup_id)?
        .ok_or_else(|| invalid("demande perdue après consommation"))?;
    Ok(ConsumedBackupRequest {
        record,
        barrier_pin_id: barrier,
    })
}

pub(crate) fn ensure_consumed_backup_tx(
    tx: &Transaction<'_>,
    request: ExecuteReservedBackupRequest,
) -> Result<(), StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    let count:i64=tx.query_row("SELECT COUNT(*) FROM backup_requests r JOIN broker_dispatches d ON d.dispatch_id=r.dispatch_id JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1 AND r.action_id=?2 AND r.dispatch_id=?3 AND r.state='executing' AND r.authority_generation=?4 AND r.daemon_epoch=?5 AND d.authority_generation=?4 AND d.daemon_epoch=?5 AND d.fence=0 AND a.authority_generation=?4 AND a.daemon_epoch=?5 AND a.fence=0 AND a.state IN ('dispatching','in_flight')",params![request.backup_id.to_string(),request.action_id.to_string(),request.dispatch_id.to_string(),daemon.authority_generation.to_string(),as_i64(daemon.daemon_epoch.as_u64())?],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
    if request.authority_generation != daemon.authority_generation || count != 1 {
        return Err(invalid(
            "exécution sauvegarde annulée, périmée ou non attribuée",
        ));
    }
    Ok(())
}

pub(crate) fn block_claimed_backup_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    backup: BackupId,
    authority: AuthorityGeneration,
    reason: BackupBlockReason,
) -> Result<BackupRequestRecord, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    if authority != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: authority,
            command_id: None,
        });
    }
    let record = read_backup_request_conn(tx, backup)?
        .ok_or_else(|| StoreError::NotFound("demande de sauvegarde".to_owned()))?;
    if record.authority_generation != authority {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: authority,
            actual: record.authority_generation,
            command_id: None,
        });
    }
    if record.state == BackupRequestState::Blocked && record.block_reason == Some(reason) {
        return Ok(record);
    }
    let markers: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM action_dispatches WHERE action_id=?1",
            [record.action_id.to_string()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let unsafe_state:i64=tx.query_row("SELECT COUNT(*) FROM actions WHERE action_id=?1 AND state NOT IN ('prepared','authorizing','approval_required','authorized','denied')",[record.action_id.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
    if record.state != BackupRequestState::Claimed
        || record.job_state != Some(BackupJobState::Preparing)
        || record.dispatch_id.is_some()
        || markers != 0
        || unsafe_state != 0
    {
        return Err(StoreError::RecoveryInProgress(
            "blocage pré-dispatch impossible : inspection de l'issue requise".to_owned(),
        ));
    }
    let policy = read_policy_summary_conn(tx, None)?;
    let verified = match reason {
        BackupBlockReason::GuardProfileUnavailable => policy
            .guard_config
            .as_ref()
            .is_some_and(|g| g.guard_mode != crate::GuardPolicyMode::BrokerOnly),
        BackupBlockReason::IsolationProfileUnavailable => {
            policy.effective_isolation_mode == crate::PolicyIsolationMode::Strict
        }
        BackupBlockReason::AuthorizationDenied => {
            tx.query_row(
                "SELECT COUNT(*) FROM actions WHERE action_id=?1 AND state='denied'",
                [record.action_id.to_string()],
                |r| r.get::<_, i64>(0),
            )
            .map_err(StoreError::from_rusqlite)?
                == 1
        }
    };
    if !verified {
        return Err(invalid("motif de blocage sauvegarde non attesté"));
    }
    let changed=tx.execute("UPDATE backup_requests SET state='blocked',revision=revision+1,block_reason=?2 WHERE backup_id=?1 AND state='claimed' AND revision=?3 AND dispatch_id IS NULL",params![backup.to_string(),reason.as_str(),as_i64(record.revision.as_u64())?]).map_err(StoreError::from_rusqlite)?;
    if changed != 1 {
        return Err(StoreError::RecoveryInProgress(
            "blocage sauvegarde concurrent".to_owned(),
        ));
    }
    let changed=tx.execute("UPDATE backup_jobs SET state='aborted',revision=revision+1 WHERE backup_id=?1 AND state='preparing' AND source_authority_generation=?2",params![backup.to_string(),authority.to_string()]).map_err(StoreError::from_rusqlite)?;
    if changed != 1 {
        return Err(invalid("job sauvegarde divergent au blocage"));
    }
    let revision: i64 = tx
        .query_row(
            "SELECT revision FROM backup_jobs WHERE backup_id=?1",
            [backup.to_string()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let event = config.id_generator.new_uuid().to_string();
    let policy_digest = canonical_sha256(&serde_json::to_value(&policy)?)?;
    let payload = json!({"backup_id":backup,"action_id":record.action_id,"receipt_id":record.receipt_id,"state":"blocked","reason":reason,"execution_state":"not_dispatched","policy_snapshot_digest":policy_digest});
    tx.execute("INSERT INTO events (event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,command_actor_id,command_id,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES (?1,?2,'backup_job',?3,?4,(SELECT COALESCE(MAX(event_ordinal)+1,0) FROM events WHERE aggregate_type='backup_job' AND aggregate_id=?3 AND aggregate_revision=?4),'BackupRequestBlocked',1,NULL,?5,?5,?6,?6,?7,'full',?8,'restricted',NULL,NULL,'command',?9,?9,?9)",params![event,daemon.stream_id.to_string(),backup.to_string(),revision,record.actor_id.as_str(),now.as_i64(),serde_json::to_string(&payload)?,canonical_sha256(&payload)?.as_prefixed_str(),record.receipt_id.to_string()]).map_err(StoreError::from_rusqlite)?;
    tx.execute("INSERT INTO outbox (event_id,destination,state,delivery_attempt,next_attempt_at_ms,authority_generation,consumer_version,delivery_key,rearm_count) VALUES (?1,'clients','pending',0,?2,?3,?4,?5,0)",params![event,now.as_i64(),authority.to_string(),config.consumer_version,format!("{event}:clients")]).map_err(StoreError::from_rusqlite)?;
    read_backup_request_conn(tx, backup)?.ok_or_else(|| invalid("demande perdue après blocage"))
}
