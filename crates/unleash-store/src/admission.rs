//! Autorité transactionnelle d'admission F003 ; aucune politique de choix parallèle.
//!
//! La composition injecte la politique pure dans le writer. Les observations
//! d'éligibilité et de neutralisation viennent d'un observateur interne enregistré,
//! jamais d'une commande client. Ce module vérifie leurs liens durables ; il ne
//! prétend pas observer lui-même un processus ou un canal modèle.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use unleash_domain::{
    AdmissionClass, AttemptId, AuthorityGeneration, DaemonEpoch, DaemonMode, Fence, InvocationId,
    LeaseId, PoolId, ProjectId, ProviderProbeId, ReplayWorkerId, ReservationId, Revision, Sequence,
    Sha256Digest, TimestampMs,
};
use unleash_protocol::{EventCursor, canonical_sha256};
use uuid::Uuid;

use crate::budgets::{
    BudgetReservationRequest, CapacityPoolState, CapacityReservationRequest, ExecutionOwner,
    ExecutionPoolParams, tx_read_execution_pool, tx_release_capacity_reservation_verified,
    tx_reserve_budgets, tx_reserve_capacity,
};
use crate::commands::{
    DaemonStateRow, ensure_scope_not_frozen_tx, read_daemon_state_conn, read_daemon_state_tx,
};
use crate::leases::{
    AcquireLeaseRequest, FenceAssertion, LeaseLineageCtx, ResourceLeaseRecord, tx_acquire_lease,
    tx_release_lease, tx_validate_lease_fence,
};
use crate::{StoreConfig, StoreError};

/// Unité réelle projetée de sa table d'origine, sans agent fictif pour un probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AdmissionUnit {
    /// Invocation logique ; la classe vient de sa ligne et de sa cause.
    Invocation(InvocationId),
    /// Travail mécanique de replay.
    Replay(ReplayWorkerId),
    /// Sonde fournisseur.
    Probe(ProviderProbeId),
}

impl AdmissionUnit {
    /// Famille fermée de propriétaire de capacité et de lease.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Invocation(_) => "agent_invocation",
            Self::Replay(_) => "replay_worker",
            Self::Probe(_) => "provider_probe",
        }
    }

    /// Identité de la ligne d'origine.
    #[must_use]
    pub const fn id(self) -> Uuid {
        match self {
            Self::Invocation(id) => id.as_uuid(),
            Self::Replay(id) => id.as_uuid(),
            Self::Probe(id) => id.as_uuid(),
        }
    }

    fn table(self) -> (&'static str, &'static str) {
        match self {
            Self::Invocation(_) => ("invocations", "invocation_id"),
            Self::Replay(_) => ("replay_workers", "replay_worker_id"),
            Self::Probe(_) => ("provider_probes", "probe_id"),
        }
    }
}

/// Ligne de file ; l'absence de preuve laisse l'unité inéligible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionUrgency {
    /// Priorité ordinaire de sa classe.
    Ordinary,
    /// Contrôle urgent attesté par l'observateur de sa cause durable.
    UrgentControl,
}

/// Ligne de file ; l'absence de preuve laisse l'unité inéligible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionCandidate {
    /// Identité réelle.
    pub unit: AdmissionUnit,
    /// Révision de la ligne.
    pub revision: Revision,
    /// Projet résolu par les parents, pas fourni par le décideur.
    pub project_id: Option<ProjectId>,
    /// Classe persistée.
    pub class: AdmissionClass,
    /// Date connue de mise en file ; les anciennes lignes sans date restent bloquées.
    pub queued_at: Option<TimestampMs>,
    /// Observation d'éligibilité pour ces entrées exactes.
    pub eligibility_evidence: Option<Uuid>,
    /// Échéance effective de cette observation, sans prolongation au commit.
    pub eligibility_valid_until: Option<TimestampMs>,
    /// Urgence liée à l'observation courante, jamais à un champ client.
    pub urgency: AdmissionUrgency,
}

/// Réservation encore comptée, y compris après crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAdmission {
    /// Identité historique de réservation.
    pub reservation_id: ReservationId,
    /// Propriétaire réel.
    pub unit: AdmissionUnit,
    /// Projet courant du propriétaire.
    pub project_id: Option<ProjectId>,
    /// Classe recopiée à l'admission.
    pub class: AdmissionClass,
    /// Horodatage d'admission.
    pub admitted_at: TimestampMs,
}

/// Snapshot consulté par la politique pure sous le writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionSnapshotToken {
    /// Autorité exacte du snapshot ; les compteurs ne traversent jamais une restauration.
    pub authority_generation: AuthorityGeneration,
    /// Incarnation exacte de l'autorité.
    pub daemon_epoch: DaemonEpoch,
    /// Révision des faits durables.
    pub inputs: Revision,
    /// Révision des observations admissibles, séparée pour permettre leur publication.
    pub observations: Revision,
    /// Première échéance qui peut changer l'ensemble admissible sans écriture.
    pub valid_until: Option<TimestampMs>,
}

/// Snapshot consulté par la politique pure sous le writer.
#[derive(Debug, Clone)]
pub struct ExecutionAdmissionSnapshot {
    /// Lignée active.
    pub authority_generation: AuthorityGeneration,
    /// Époque active.
    pub daemon_epoch: DaemonEpoch,
    /// CAS conservateur couvrant les entrées réelles, pas seulement la configuration.
    pub source_revision: Revision,
    /// Jeton CAS des faits ET de l'ensemble des observations d'éligibilité.
    pub token: AdmissionSnapshotToken,
    /// Pool unique et occupation réelle.
    pub pool: CapacityPoolState,
    /// Réglages Q-15.
    pub settings: ExecutionPoolParams,
    /// Réservations held/reconciling.
    pub active: Vec<ActiveAdmission>,
    /// Projection des trois tables d'unités ; filtrer les preuves absentes avant decide.
    pub queued: Vec<AdmissionCandidate>,
    /// Poids et déficit par projet ; None pour la portée utilisateur.
    pub fairness: BTreeMap<Option<ProjectId>, (u8, i64)>,
    /// Rafale durable courante.
    pub consecutive_controls: u64,
    /// Demandes durables ; aucune durée monotone n'est inventée à partir d'UTC.
    pub checkpoints: Vec<ExecutionCheckpointRequestRecord>,
}

/// Demande de checkpoint actuellement persistée.
#[derive(Debug, Clone)]
pub struct ExecutionCheckpointRequestRecord {
    /// Cible historique exacte.
    pub reservation_id: ReservationId,
    /// Contrôle dont la demande est issue.
    pub waiting_control: InvocationId,
    /// Horodatage de journal, pas preuve d'un délai monotone.
    pub requested_at: TimestampMs,
}

/// Observation privilégiée de toutes les préconditions hors de la politique Q-15.
///
/// L'observateur est responsable des dépendances, droits, politique modèle,
/// quotas et sous-limites. Son objet est une preuve d'observation réellement
/// produite ; son contenu n'est jamais déduit du seul état queued. Le banc peut
/// être cet observateur avec une provenance explicitement synthétique.
#[derive(Debug, Clone)]
pub struct AdmissionEligibilityObservation {
    /// ID stable de l'observation (rejeu exact seulement).
    pub evidence_id: Uuid,
    /// Principal interne actif service/system.
    pub observer_principal_id: String,
    /// Objet durable de l'observation.
    pub object_hash: Sha256Digest,
    /// Ligne observée.
    pub unit: AdmissionUnit,
    /// Révision observée de la ligne.
    pub unit_revision: Revision,
    /// Snapshot intégral des préconditions observées.
    pub source_revision: Revision,
    /// Lignée observée.
    pub authority_generation: AuthorityGeneration,
    /// Époque observée.
    pub daemon_epoch: DaemonEpoch,
    /// Échéance effective la plus proche des autorisations/quotas observés.
    pub valid_until: TimestampMs,
    /// Urgence observée dans la cause, seulement pour une invocation contrôle.
    pub urgency: AdmissionUrgency,
}

/// Choix retenu par la politique pure ; le Store valide identité, CAS et deltas.
#[derive(Debug, Clone)]
pub enum AdmissionDecision {
    /// Aucun travail admissible.
    Idle,
    /// Attente sans réservation ni débit d'équité.
    Wait,
    /// Admission proposée, à engager entièrement ou pas du tout.
    Admit {
        /// Candidate choisie dans le snapshot courant.
        unit: AdmissionUnit,
        /// Deltas du déficit pondéré de la classe choisie.
        deficit_deltas: BTreeMap<Option<ProjectId>, i64>,
        /// Compteur après ce choix.
        consecutive_controls: u64,
        /// Coûts déjà attribués à des actions réelles et à leurs comptes applicables.
        budgets: Vec<BudgetReservationRequest>,
    },
}

/// Engagement d'une unité et de sa capacité de continuation.
#[derive(Debug, Clone)]
pub struct AdmissionCommit {
    /// Unité engagée.
    pub unit: AdmissionUnit,
    /// Nouvelle révision de l'unité.
    pub revision: Revision,
    /// Nouvelle réservation ; jamais réutilisée à la réadmission.
    pub reservation_id: ReservationId,
    /// Nouvelle tentative pour une invocation.
    pub attempt_id: Option<AttemptId>,
    /// Faisceau lignée/epoch/fence/ressource.
    pub lease: ResourceLeaseRecord,
    /// Vrai curseur de l'événement engagé avec la réservation.
    pub event_cursor: EventCursor,
}

/// Résultat sans ambiguïté entre attente et admission engagée.
#[derive(Debug, Clone)]
pub enum AdmissionOutcome {
    /// Pas de changement durable.
    Unchanged,
    /// Admission durable.
    Admitted(Box<AdmissionCommit>),
}

/// Preuves distinctes publiées par les observateurs internes ; aucun booléen de sûreté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionContinuation {
    /// Réservation exacte.
    pub reservation_id: ReservationId,
    /// Unité réelle.
    pub unit: AdmissionUnit,
    /// Révision à la frontière observée.
    pub unit_revision: Revision,
    /// Lease de continuation.
    pub lease_id: LeaseId,
    /// Lignée de la continuation.
    pub authority_generation: AuthorityGeneration,
    /// Époque de la continuation.
    pub daemon_epoch: DaemonEpoch,
    /// Fence de la continuation.
    pub fence: Fence,
    /// Boot du host propriétaire.
    pub boot_identity: String,
}

/// Preuves distinctes publiées par les observateurs internes ; aucun booléen de sûreté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCheckpointObservation {
    /// Réservation exacte que l'observation peut libérer.
    pub reservation_id: ReservationId,
    /// Unité et révision exactes.
    pub unit: AdmissionUnit,
    /// Révision de l'unité à la frontière sûre.
    pub unit_revision: Revision,
    /// Lease de continuation observée.
    pub lease_id: LeaseId,
    /// Lignée observée.
    pub authority_generation: AuthorityGeneration,
    /// Époque observée.
    pub daemon_epoch: DaemonEpoch,
    /// Fence observé.
    pub fence: Fence,
    /// Identité de boot du faisceau.
    pub boot_identity: String,
    /// Checkpoint complet publié (contexte/curseur, décisions, entrées et actions).
    pub checkpoint_published: Uuid,
    /// Fin/annulation locale observée du canal modèle.
    pub local_model_quiescent: Uuid,
    /// Continuation et descendants neutralisés sous l'identité observée.
    pub continuation_neutralized: Uuid,
    /// Disposition attestée des ressources conservées ou libérées.
    pub resources_disposed: Uuid,
}

impl ExecutionCheckpointObservation {
    /// Identité à demander en checkpoint, sans inventer de preuve future.
    #[must_use]
    pub fn continuation(&self) -> ExecutionContinuation {
        ExecutionContinuation {
            reservation_id: self.reservation_id,
            unit: self.unit,
            unit_revision: self.unit_revision,
            lease_id: self.lease_id,
            authority_generation: self.authority_generation,
            daemon_epoch: self.daemon_epoch,
            fence: self.fence,
            boot_identity: self.boot_identity.clone(),
        }
    }
    /// Clé de corrélation que les preuves d'observation doivent porter dans `snapshot_ref`.
    #[must_use]
    pub fn evidence_binding(&self, role: &str) -> String {
        format!(
            "execution-checkpoint:v1:{role}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.reservation_id,
            self.unit.id(),
            self.unit_revision,
            self.lease_id,
            self.authority_generation,
            self.daemon_epoch,
            self.fence,
            self.boot_identity
        )
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("entier admission hors plage SQLite"))
}

fn unsigned(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| invalid("entier admission négatif"))
}

fn uuid(raw: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(raw)
        .map_err(|_| StoreError::InvariantViolation("identité admission invalide".to_owned()))
}

fn class(raw: &str) -> Result<AdmissionClass, StoreError> {
    match raw {
        "control" => Ok(AdmissionClass::Control),
        "work" => Ok(AdmissionClass::Work),
        _ => Err(invalid("classe admission inconnue")),
    }
}

fn revision(conn: &Connection) -> Result<Revision, StoreError> {
    let value: i64 = conn.query_row(
        "SELECT revision FROM admission_source_clock WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    Ok(Revision::new(unsigned(value)?))
}

fn eligibility_binding(
    candidate: &AdmissionCandidate,
    daemon: &DaemonStateRow,
    source: Revision,
) -> String {
    format!(
        "execution-eligibility:v1:{}:{}:{}:{}:{}:{}",
        candidate.unit.kind(),
        candidate.unit.id(),
        candidate.revision,
        daemon.authority_generation,
        daemon.daemon_epoch,
        source
    )
}

fn verify_evidence(conn: &Connection, evidence: Uuid, binding: &str) -> Result<(), StoreError> {
    let valid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM evidence e JOIN principals p ON p.principal_id=e.producer_id JOIN object_records o ON o.object_hash=e.object_hash WHERE e.evidence_id=?1 AND e.kind='observation' AND e.snapshot_ref=?2 AND e.schema_version=1 AND e.availability='available' AND p.state='active' AND p.principal_kind IN ('service','system') AND o.state='live')", params![evidence.to_string(), binding], |row| row.get(0))?;
    if !valid {
        return Err(invalid("observation absente, périmée ou non attribuable"));
    }
    Ok(())
}

/// Projection interne réutilisée dans une lecture et dans le writer.
#[expect(
    clippy::too_many_lines,
    reason = "Projection cohérente des trois files, des propriétaires et de l'état d'équité"
)]
pub(crate) fn read_snapshot(
    conn: &Connection,
    pool_id: PoolId,
    now: TimestampMs,
) -> Result<ExecutionAdmissionSnapshot, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    let pool = tx_read_execution_pool(conn, pool_id)?;
    let source_revision = revision(conn)?;
    let settings = conn.query_row("SELECT control_reserve_mode,control_reserved_units,control_burst_limit,checkpoint_escalation_s FROM capacity_pools WHERE pool_id=?1 AND pool_kind='execution' AND scope_key='global' AND unit='slot'", [pool_id.to_string()], |row| {
        Ok(ExecutionPoolParams { pool_id, requested_limit: pool.requested_limit, control_reserve_mode: if row.get::<_,String>(0)? == "hard" { crate::ControlReserveMode::Hard } else { crate::ControlReserveMode::Soft }, control_reserved_units: row.get(1)?, control_burst_limit: row.get(2)?, checkpoint_escalation_s: row.get(3)? })
    })?;
    let mut queued = Vec::new();
    let sql = "SELECT 'invocation',i.invocation_id,i.revision,a.project_id,i.admission_class,COALESCE(i.queued_at_ms,i.created_at_ms) FROM invocations i JOIN agents a ON a.agent_id=i.agent_id WHERE i.state='queued' AND i.wait_reason IS NULL UNION ALL SELECT 'replay',w.replay_worker_id,w.revision,m.project_id,w.admission_class,w.queued_at_ms FROM replay_workers w JOIN workflow_runs r ON r.run_id=w.run_id JOIN missions m ON m.mission_id=r.mission_id WHERE w.state='queued' AND w.wait_reason IS NULL UNION ALL SELECT 'probe',p.probe_id,p.revision,m.project_id,p.admission_class,p.queued_at_ms FROM provider_probes p LEFT JOIN workflow_runs r ON r.run_id=p.source_run_id LEFT JOIN missions m ON m.mission_id=r.mission_id WHERE p.state='queued' AND p.wait_reason IS NULL ORDER BY 1,2";
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if queued.len() >= 4096 {
            return Err(invalid("snapshot admission dépasse 4096 candidats"));
        }
        let kind: String = row.get(0)?;
        let id = uuid(&row.get::<_, String>(1)?)?;
        let unit = match kind.as_str() {
            "invocation" => AdmissionUnit::Invocation(InvocationId::from_uuid(id)),
            "replay" => AdmissionUnit::Replay(ReplayWorkerId::from_uuid(id)),
            _ => AdmissionUnit::Probe(ProviderProbeId::from_uuid(id)),
        };
        let mut candidate = AdmissionCandidate {
            unit,
            revision: Revision::new(unsigned(row.get(2)?)?),
            project_id: row
                .get::<_, Option<String>>(3)?
                .map(|id| uuid(&id).map(ProjectId::from_uuid))
                .transpose()?,
            class: class(&row.get::<_, String>(4)?)?,
            queued_at: row.get::<_, Option<i64>>(5)?.map(TimestampMs::new),
            eligibility_evidence: None,
            eligibility_valid_until: None,
            urgency: AdmissionUrgency::Ordinary,
        };
        let evidence: Option<(String,String,i64)> = conn.query_row("SELECT e.evidence_id,a.urgency,a.valid_until_ms FROM evidence e JOIN admission_eligibility_observations a ON a.evidence_id=e.evidence_id WHERE e.snapshot_ref=?1 ORDER BY a.observation_seq DESC LIMIT 1", [eligibility_binding(&candidate,&daemon,source_revision)], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        if let Some((raw, urgency, valid_until)) =
            evidence.filter(|(_, _, until)| *until > now.as_i64())
        {
            let id = uuid(&raw)?;
            match verify_evidence(
                conn,
                id,
                &eligibility_binding(&candidate, &daemon, source_revision),
            ) {
                Ok(()) => {}
                Err(StoreError::InvalidArgument { .. }) => {
                    queued.push(candidate);
                    continue;
                }
                Err(error) => return Err(error),
            }
            candidate.eligibility_evidence = Some(id);
            candidate.eligibility_valid_until = Some(TimestampMs::new(valid_until));
            candidate.urgency = if urgency == "urgent_control" {
                AdmissionUrgency::UrgentControl
            } else {
                AdmissionUrgency::Ordinary
            };
        }
        queued.push(candidate);
    }
    let mut active = Vec::new();
    let mut stmt = conn.prepare("SELECT c.reservation_id,c.owner_type,c.owner_id,c.admission_class,c.created_at_ms,c.amount,o.invocation_id,o.replay_worker_id,o.provider_probe_id FROM capacity_reservations c LEFT JOIN execution_reservation_owners o ON o.reservation_id=c.reservation_id WHERE c.pool_id=?1 AND c.state IN ('held','reconciling') ORDER BY c.reservation_id")?;
    let mut rows = stmt.query([pool_id.to_string()])?;
    while let Some(row) = rows.next()? {
        let kind: String = row.get(1)?;
        let id_raw: String = row.get(2)?;
        let id = uuid(&id_raw)?;
        let unit = match kind.as_str() {
            "agent_invocation"
                if row.get::<_, Option<String>>(6)?.as_deref() == Some(id_raw.as_str()) =>
            {
                AdmissionUnit::Invocation(InvocationId::from_uuid(id))
            }
            "replay_worker"
                if row.get::<_, Option<String>>(7)?.as_deref() == Some(id_raw.as_str()) =>
            {
                AdmissionUnit::Replay(ReplayWorkerId::from_uuid(id))
            }
            "provider_probe"
                if row.get::<_, Option<String>>(8)?.as_deref() == Some(id_raw.as_str()) =>
            {
                AdmissionUnit::Probe(ProviderProbeId::from_uuid(id))
            }
            _ => return Err(invalid("propriétaire execution non prouvé")),
        };
        if row.get::<_, i64>(5)? != 1 {
            return Err(invalid("une exécution doit réserver exactement une unité"));
        }
        active.push(ActiveAdmission {
            reservation_id: ReservationId::from_uuid(uuid(&row.get::<_, String>(0)?)?),
            unit,
            project_id: project_for(conn, unit)?,
            class: class(&row.get::<_, String>(3)?)?,
            admitted_at: TimestampMs::new(row.get(4)?),
        });
    }
    let mut fairness = BTreeMap::new();
    fairness.insert(None, (1, 0));
    let mut stmt=conn.prepare("SELECT p.project_id,p.weight,COALESCE(f.deficit,0) FROM projects p LEFT JOIN execution_fairness f ON f.project_key=p.project_id AND f.pool_id=?1 UNION ALL SELECT '',1,deficit FROM execution_fairness WHERE project_key='' AND pool_id=?1")?;
    for row in stmt.query_map([pool_id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u8>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })? {
        let (key, weight, deficit) = row?;
        fairness.insert(
            if key.is_empty() {
                None
            } else {
                Some(ProjectId::from_uuid(uuid(&key)?))
            },
            (weight, deficit),
        );
    }
    let consecutive_controls = unsigned(
        conn.query_row(
            "SELECT consecutive_controls FROM execution_scheduler_state WHERE pool_id=?1",
            [pool_id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0),
    )?;
    let observation_revision: i64 = conn.query_row(
        "SELECT revision FROM admission_observation_clock WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    let mut checkpoints = Vec::new();
    let mut statement=conn.prepare("SELECT r.reservation_id,r.waiting_control,r.requested_at_ms FROM execution_checkpoint_requests r JOIN capacity_reservations c ON c.reservation_id=r.reservation_id WHERE c.pool_id=?1 AND c.state IN ('held','reconciling') ORDER BY r.requested_at_ms,r.reservation_id")?;
    for row in statement.query_map([pool_id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })? {
        let (reservation, waiting, time) = row?;
        checkpoints.push(ExecutionCheckpointRequestRecord {
            reservation_id: ReservationId::from_uuid(uuid(&reservation)?),
            waiting_control: InvocationId::from_uuid(uuid(&waiting)?),
            requested_at: TimestampMs::new(time),
        });
    }
    Ok(ExecutionAdmissionSnapshot {
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        source_revision,
        token: AdmissionSnapshotToken {
            authority_generation: daemon.authority_generation,
            daemon_epoch: daemon.daemon_epoch,
            inputs: source_revision,
            observations: Revision::new(unsigned(observation_revision)?),
            valid_until: queued
                .iter()
                .filter_map(|candidate| candidate.eligibility_valid_until)
                .min(),
        },
        pool,
        settings,
        active,
        queued,
        fairness,
        consecutive_controls,
        checkpoints,
    })
}

fn project_for(conn: &Connection, unit: AdmissionUnit) -> Result<Option<ProjectId>, StoreError> {
    let sql = match unit {
        AdmissionUnit::Invocation(_) => {
            "SELECT a.project_id FROM invocations i JOIN agents a ON a.agent_id=i.agent_id WHERE i.invocation_id=?1"
        }
        AdmissionUnit::Replay(_) => {
            "SELECT m.project_id FROM replay_workers w JOIN workflow_runs r ON r.run_id=w.run_id JOIN missions m ON m.mission_id=r.mission_id WHERE w.replay_worker_id=?1"
        }
        AdmissionUnit::Probe(_) => {
            "SELECT m.project_id FROM provider_probes p LEFT JOIN workflow_runs r ON r.run_id=p.source_run_id LEFT JOIN missions m ON m.mission_id=r.mission_id WHERE p.probe_id=?1"
        }
    };
    conn.query_row(sql, [unit.id().to_string()], |row| {
        row.get::<_, Option<String>>(0)
    })?
    .map(|raw| uuid(&raw).map(ProjectId::from_uuid))
    .transpose()
}

pub(crate) fn record_eligibility(
    tx: &Transaction<'_>,
    now: TimestampMs,
    observation: &AdmissionEligibilityObservation,
) -> Result<(), StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    check_lineage(
        &daemon,
        observation.authority_generation,
        observation.daemon_epoch,
    )?;
    if observation.valid_until.as_i64() <= now.as_i64() {
        return Err(invalid("observation éligibilité expirée"));
    }
    if revision(tx)? != observation.source_revision {
        return Err(invalid("préconditions admission modifiées"));
    }
    let candidate = AdmissionCandidate {
        unit: observation.unit,
        revision: observation.unit_revision,
        project_id: None,
        class: AdmissionClass::Work,
        queued_at: None,
        eligibility_evidence: None,
        eligibility_valid_until: None,
        urgency: observation.urgency,
    };
    let (table, key) = observation.unit.table();
    let current: i64 = tx.query_row(
        &format!(
            "SELECT revision FROM {table} WHERE {key}=?1 AND state='queued' AND wait_reason IS NULL"
        ),
        [observation.unit.id().to_string()],
        |row| row.get(0),
    )?;
    if unsigned(current)? != observation.unit_revision.as_u64() {
        return Err(invalid("unité observée périmée"));
    }
    let binding = eligibility_binding(&candidate, &daemon, observation.source_revision);
    let prior: Option<(String, String, String)> = tx
        .query_row(
            "SELECT producer_id,object_hash,snapshot_ref FROM evidence WHERE evidence_id=?1",
            [observation.evidence_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some(prior) = prior {
        if prior
            != (
                observation.observer_principal_id.clone(),
                observation.object_hash.to_string(),
                binding,
            )
        {
            return Err(invalid("rejeu divergent observation admission"));
        }
    } else {
        tx.execute("INSERT INTO evidence(evidence_id,kind,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES(?1,'observation',?2,?3,?4,?5,1,'available')",params![observation.evidence_id.to_string(),binding,observation.object_hash.to_string(),observation.observer_principal_id,now.as_i64()])?;
    }
    verify_evidence(
        tx,
        observation.evidence_id,
        &eligibility_binding(&candidate, &daemon, observation.source_revision),
    )?;
    let urgency = if observation.urgency == AdmissionUrgency::UrgentControl {
        let (table, key) = observation.unit.table();
        let is_control: bool = tx.query_row(
            &format!("SELECT admission_class='control' FROM {table} WHERE {key}=?1"),
            [observation.unit.id().to_string()],
            |row| row.get(0),
        )?;
        if !is_control {
            return Err(invalid("urgence réservée à une invocation contrôle"));
        }
        "urgent_control"
    } else {
        "ordinary"
    };
    tx.execute("INSERT INTO admission_eligibility_observations(evidence_id,valid_until_ms,urgency) VALUES(?1,?2,?3) ON CONFLICT(evidence_id) DO NOTHING",params![observation.evidence_id.to_string(),observation.valid_until.as_i64(),urgency])?;
    let (valid_until,stored_urgency): (i64,String) = tx.query_row(
        "SELECT valid_until_ms,urgency FROM admission_eligibility_observations WHERE evidence_id=?1",
        [observation.evidence_id.to_string()],
        |row| Ok((row.get(0)?,row.get(1)?)),
    )?;
    if valid_until != observation.valid_until.as_i64() || stored_urgency != urgency {
        return Err(invalid("échéance observation divergente"));
    }
    Ok(())
}

fn check_lineage(
    daemon: &DaemonStateRow,
    authority: AuthorityGeneration,
    epoch: DaemonEpoch,
) -> Result<(), StoreError> {
    if daemon.authority_generation != authority {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: authority,
            command_id: None,
        });
    }
    if daemon.daemon_epoch != epoch {
        return Err(StoreError::StaleFence {
            message: "époque admission obsolète".to_owned(),
        });
    }
    if daemon.mode != DaemonMode::Ready {
        return Err(invalid("admission interdite dans ce mode daemon"));
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "Séquence indivisible CAS, tentative, lease, réservations, équité et journal"
)]
pub(crate) fn commit<F>(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    pool_id: PoolId,
    expected: AdmissionSnapshotToken,
    decide: F,
) -> Result<AdmissionOutcome, StoreError>
where
    F: FnOnce(&ExecutionAdmissionSnapshot) -> Result<AdmissionDecision, StoreError>,
{
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid(
            "échéance d'éligibilité inconnue : horloge incertaine",
        ));
    }
    let daemon = read_daemon_state_tx(tx)?;
    check_lineage(
        &daemon,
        expected.authority_generation,
        expected.daemon_epoch,
    )?;
    let snapshot = read_snapshot(tx, pool_id, now)?;
    if snapshot.token != expected {
        return Err(StoreError::RevisionConflict {
            current_revision: snapshot.source_revision,
            expected_revision: expected.inputs,
            command_id: None,
        });
    }
    let AdmissionDecision::Admit {
        unit,
        deficit_deltas,
        consecutive_controls,
        budgets,
    } = decide(&snapshot)?
    else {
        return Ok(AdmissionOutcome::Unchanged);
    };
    if expected
        .valid_until
        .is_some_and(|deadline| deadline.as_i64() <= config.clock.now_ms().as_i64())
    {
        return Err(invalid("snapshot admission expiré pendant la décision"));
    }
    let candidate = snapshot
        .queued
        .iter()
        .find(|c| c.unit == unit)
        .ok_or_else(|| invalid("unité absente de la file courante"))?;
    let evidence = candidate
        .eligibility_evidence
        .ok_or_else(|| invalid("éligibilité non observée"))?;
    verify_evidence(
        tx,
        evidence,
        &eligibility_binding(candidate, &daemon, expected.inputs),
    )?;
    if candidate.queued_at.is_none() {
        return Err(invalid("date de mise en file non attestée"));
    }
    validate_candidate(tx, candidate, &daemon)?;
    validate_scope_and_subjects(tx, candidate)?;
    validate_deltas(&snapshot, candidate, &deficit_deltas, consecutive_controls)?;
    let lease = tx_acquire_lease(
        tx,
        &LeaseLineageCtx {
            authority: daemon.authority_generation,
            daemon_epoch: daemon.daemon_epoch,
            boot_identity: &daemon.boot_identity,
            now_ms: now,
        },
        LeaseId::from_uuid(config.id_generator.new_uuid()),
        &AcquireLeaseRequest {
            resource_kind: "execution".to_owned(),
            resource_key: format!("{}:{}", unit.kind(), unit.id()),
            owner_type: unit.kind().to_owned(),
            owner_id: unit.id().to_string(),
            ttl_ms: 60_000,
        },
    )?;
    let attempt_id = if let AdmissionUnit::Invocation(id) = unit {
        let attempt = AttemptId::from_uuid(config.id_generator.new_uuid());
        tx.execute("INSERT INTO invocation_attempts(attempt_id,invocation_id,ordinal,authority_generation,daemon_epoch,fence,state,started_at_ms) VALUES(?1,?2,(SELECT COALESCE(MAX(ordinal),0)+1 FROM invocation_attempts WHERE invocation_id=?2),?3,?4,?5,'admitted',?6)",params![attempt.to_string(),id.to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?,integer(lease.fence.as_u64())?,now.as_i64()])?;
        Some(attempt)
    } else {
        None
    };
    let (table, key) = unit.table();
    let next = Revision::new(
        candidate
            .revision
            .as_u64()
            .checked_add(1)
            .ok_or_else(|| invalid("révision admission épuisée"))?,
    );
    let extra = if matches!(unit, AdmissionUnit::Invocation(_)) {
        String::new()
    } else {
        format!(
            ",authority_generation='{}',daemon_epoch={},fence={}",
            daemon.authority_generation,
            integer(daemon.daemon_epoch.as_u64())?,
            integer(lease.fence.as_u64())?
        )
    };
    let changed=tx.execute(&format!("UPDATE {table} SET state='admitted',revision=?1{extra} WHERE {key}=?2 AND revision=?3 AND state='queued'"),params![integer(next.as_u64())?,unit.id().to_string(),integer(candidate.revision.as_u64())?])?;
    if changed != 1 {
        return Err(invalid("statut admission modifié"));
    }
    let reservation_id = ReservationId::from_uuid(config.id_generator.new_uuid());
    let owner = match unit {
        AdmissionUnit::Invocation(invocation_id) => ExecutionOwner::AgentInvocation {
            invocation_id,
            attempt_id,
        },
        AdmissionUnit::Replay(replay_worker_id) => {
            ExecutionOwner::ReplayWorker { replay_worker_id }
        }
        AdmissionUnit::Probe(provider_probe_id) => {
            ExecutionOwner::ProviderProbe { provider_probe_id }
        }
    };
    tx_reserve_capacity(
        tx,
        daemon.authority_generation,
        daemon.daemon_epoch,
        now,
        &CapacityReservationRequest {
            reservation_id,
            pool_id,
            execution_owner: Some(owner),
            owner_type: unit.kind().to_owned(),
            owner_id: unit.id().to_string(),
            admission_class: Some(candidate.class),
            amount: 1,
        },
    )?;
    for budget in &budgets {
        validate_budget_owner(tx, unit, budget)?;
        tx_reserve_budgets(tx, config.id_generator.as_ref(), budget)?;
    }
    for (key, delta) in deficit_deltas {
        let current = snapshot.fairness.get(&key).map_or(0, |value| value.1);
        let next = current
            .checked_add(delta)
            .ok_or_else(|| invalid("déficit hors plage"))?;
        tx.execute("INSERT INTO execution_fairness(pool_id,project_key,deficit) VALUES(?1,?2,?3) ON CONFLICT(pool_id,project_key) DO UPDATE SET deficit=excluded.deficit",params![pool_id.to_string(),key.map_or_else(String::new,|id|id.to_string()),next])?;
    }
    tx.execute("INSERT INTO execution_scheduler_state(pool_id,consecutive_controls) VALUES(?1,?2) ON CONFLICT(pool_id) DO UPDATE SET consecutive_controls=excluded.consecutive_controls",params![pool_id.to_string(),integer(consecutive_controls)?])?;
    let event_name = match unit {
        AdmissionUnit::Invocation(_) => "InvocationAdmitted",
        AdmissionUnit::Replay(_) => "ReplayWorkerAdmitted",
        AdmissionUnit::Probe(_) => "ProviderProbeAdmitted",
    };
    let cursor = append_event(
        tx,
        config,
        &daemon,
        now,
        unit,
        next,
        candidate.project_id,
        event_name,
        &json!({"reservation_id":reservation_id,"attempt_id":attempt_id,"lease_id":lease.lease_id,"fence":lease.fence,"eligibility_evidence_id":evidence}),
    )?;
    Ok(AdmissionOutcome::Admitted(Box::new(AdmissionCommit {
        unit,
        revision: next,
        reservation_id,
        attempt_id,
        lease,
        event_cursor: cursor,
    })))
}

fn validate_candidate(
    tx: &Transaction<'_>,
    candidate: &AdmissionCandidate,
    daemon: &DaemonStateRow,
) -> Result<(), StoreError> {
    if let Some(project) = candidate.project_id {
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1 AND state='active')",
            [project.to_string()],
            |row| row.get(0),
        )?;
        if !active {
            return Err(invalid("projet admission inactif"));
        }
    }
    match candidate.unit {
        AdmissionUnit::Invocation(id) => {
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM invocations i JOIN agents a ON a.agent_id=i.agent_id JOIN sessions s ON s.session_id=i.session_id JOIN effective_bundles b ON b.bundle_hash=i.effective_bundle_hash JOIN context_manifests m ON m.manifest_id=i.context_manifest_id JOIN object_records o ON o.object_hash=m.content_hash JOIN object_records bo ON bo.object_hash=b.content_object_hash JOIN events e ON e.event_id=i.trigger_event_id WHERE i.invocation_id=?1 AND a.state IN ('eligible','active') AND s.agent_id=i.agent_id AND s.state='open' AND m.session_id=i.session_id AND o.state='live' AND bo.state='live' AND e.stream_id=?2 AND (i.admission_class='work' OR a.level IN ('N+1','N+2')) AND NOT EXISTS(SELECT 1 FROM invocation_attempts t WHERE t.invocation_id=i.invocation_id AND t.state IN ('admitted','running','checkpointing','reconciling')))",params![id.to_string(),daemon.stream_id.to_string()],|row|row.get(0))?;
            if !valid {
                return Err(invalid(
                    "parents, contexte, cause ou ancienne tentative non valides",
                ));
            }
        }
        AdmissionUnit::Replay(id) => {
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM replay_workers w JOIN step_instances s ON s.step_id=w.step_id JOIN workflow_runs r ON r.run_id=w.run_id WHERE w.replay_worker_id=?1 AND s.run_id=w.run_id AND s.state IN ('ready','queued') AND r.state IN ('queued','running') AND w.admission_class='work' AND w.authority_generation=?2 AND w.daemon_epoch=?3)",params![id.to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?],|row|row.get(0))?;
            if !valid {
                return Err(invalid("parents ou incarnation replay non valides"));
            }
        }
        AdmissionUnit::Probe(id) => {
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_probes p JOIN principals s ON s.principal_id=p.principal_id JOIN command_receipts c ON c.receipt_id=p.command_receipt_id JOIN provider_profiles f ON f.profile_version_ref=p.profile_version_ref WHERE p.probe_id=?1 AND s.state='active' AND c.actor_id=s.actor_id AND c.authority_generation=?2 AND c.state IN ('accepted','completed') AND p.authority_generation=?2 AND p.daemon_epoch=?3 AND p.admission_class='work' AND p.context_hash!='')",params![id.to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?],|row|row.get(0))?;
            if !valid {
                return Err(invalid("cause, sujet ou incarnation probe non valides"));
            }
        }
    }
    Ok(())
}

fn validate_scope_and_subjects(
    tx: &Transaction<'_>,
    candidate: &AdmissionCandidate,
) -> Result<(), StoreError> {
    use crate::policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx};
    let (run, scope): (Option<String>, Option<String>) = match candidate.unit {
        AdmissionUnit::Invocation(id) => {
            let (agent,run,scope):(String,Option<String>,String)=tx.query_row("SELECT i.agent_id,i.run_id,b.scope_id FROM invocations i JOIN effective_bundles b ON b.bundle_hash=i.effective_bundle_hash WHERE i.invocation_id=?1",[id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
            ensure_subject_not_blocked_tx(tx, SubjectKind::Agent, &agent, candidate.project_id)?;
            (run, Some(scope))
        }
        AdmissionUnit::Replay(id) => (
            Some(tx.query_row(
                "SELECT run_id FROM replay_workers WHERE replay_worker_id=?1",
                [id.to_string()],
                |row| row.get(0),
            )?),
            None,
        ),
        AdmissionUnit::Probe(id) => {
            let (actor,run):(String,Option<String>)=tx.query_row("SELECT s.actor_id,p.source_run_id FROM provider_probes p JOIN principals s ON s.principal_id=p.principal_id WHERE p.probe_id=?1",[id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?;
            ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, &actor, candidate.project_id)?;
            (run, None)
        }
    };
    if let Some(run) = run.as_deref() {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Workflow, run, candidate.project_id)?;
    }
    let run = run
        .map(|raw| uuid(&raw).map(unleash_domain::RunId::from_uuid))
        .transpose()?;
    ensure_scope_not_frozen_tx(tx, candidate.project_id, scope.as_deref(), run)
}

fn validate_budget_owner(
    tx: &Transaction<'_>,
    unit: AdmissionUnit,
    budget: &BudgetReservationRequest,
) -> Result<(), StoreError> {
    if budget.charges.is_empty() {
        return Err(invalid("comptes budgétaires absents"));
    }
    let sql = match unit {
        AdmissionUnit::Invocation(_) => {
            "SELECT EXISTS(SELECT 1 FROM actions WHERE action_id=?1 AND invocation_id=?2)"
        }
        AdmissionUnit::Replay(_) => {
            "SELECT EXISTS(SELECT 1 FROM actions WHERE action_id=?1 AND replay_worker_id=?2)"
        }
        AdmissionUnit::Probe(_) => {
            "SELECT EXISTS(SELECT 1 FROM model_calls WHERE action_id=?1 AND provider_probe_id=?2)"
        }
    };
    if !tx.query_row(
        sql,
        params![budget.action_id.to_string(), unit.id().to_string()],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(invalid("action budgétaire d'une autre unité"));
    }
    Ok(())
}

fn validate_deltas(
    snapshot: &ExecutionAdmissionSnapshot,
    selected: &AdmissionCandidate,
    deltas: &BTreeMap<Option<ProjectId>, i64>,
    burst: u64,
) -> Result<(), StoreError> {
    let mut expected = BTreeMap::new();
    for candidate in snapshot.queued.iter().filter(|c| {
        c.class == selected.class && c.eligibility_evidence.is_some() && c.queued_at.is_some()
    }) {
        let key = candidate.project_id;
        expected.insert(
            key,
            i64::from(snapshot.fairness.get(&key).map_or(1, |v| v.0)),
        );
    }
    let sum: i64 = expected.values().sum();
    let key = selected.project_id;
    *expected.entry(key).or_default() -= sum;
    let expected_burst = if selected.class == AdmissionClass::Control
        && snapshot
            .queued
            .iter()
            .any(|c| c.class == AdmissionClass::Work && c.eligibility_evidence.is_some())
    {
        snapshot.consecutive_controls.saturating_add(1)
    } else {
        0
    };
    if &expected != deltas || burst != expected_burst {
        return Err(invalid("deltas ou rafale non liés au snapshot choisi"));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Enveloppe interne d'événement atomique liée à l'unité"
)]
fn append_event(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    unit: AdmissionUnit,
    rev: Revision,
    project: Option<ProjectId>,
    name: &str,
    payload: &serde_json::Value,
) -> Result<EventCursor, StoreError> {
    let event = config.id_generator.new_uuid();
    let aggregate = match unit {
        AdmissionUnit::Invocation(_) => "invocation",
        AdmissionUnit::Replay(_) => "replay_worker",
        AdmissionUnit::Probe(_) => "provider_probe",
    };
    tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,correlation_id) VALUES(?1,?2,?3,?4,?5,(SELECT COALESCE(MAX(event_ordinal)+1,0) FROM events WHERE aggregate_type=?3 AND aggregate_id=?4 AND aggregate_revision=?5),?6,1,?7,'system:scheduler','system:scheduler',?8,?8,?9,'full',?10,'restricted','external',?4,?1)",params![event.to_string(),daemon.stream_id.to_string(),aggregate,unit.id().to_string(),integer(rev.as_u64())?,name,project.map(|id|id.to_string()),now.as_i64(),serde_json::to_string(payload)?,canonical_sha256(payload)?.to_string()])?;
    let sequence = Sequence::new(
        u64::try_from(tx.last_insert_rowid()).map_err(|_| invalid("séquence négative"))?,
    );
    for destination in ["clients", "harness"] {
        tx.execute("INSERT INTO outbox(event_id,destination,state,next_attempt_at_ms,authority_generation,consumer_version,delivery_key) VALUES(?1,?2,'pending',?3,?4,?5,?6)",params![event.to_string(),destination,now.as_i64(),daemon.authority_generation.to_string(),config.consumer_version,format!("{event}:{destination}")])?;
    }
    Ok(EventCursor {
        stream_id: daemon.stream_id,
        sequence,
    })
}

fn checkpoint_lease(
    tx: &Transaction<'_>,
    observation: &ExecutionContinuation,
) -> Result<DaemonStateRow, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    // Drain permits checkpoint completion but never a new admission.
    if daemon.authority_generation != observation.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: observation.authority_generation,
            command_id: None,
        });
    }
    let assertion = FenceAssertion {
        lease_id: observation.lease_id,
        authority_generation: observation.authority_generation,
        daemon_epoch: observation.daemon_epoch,
        fence: observation.fence,
        owner_id: observation.unit.id().to_string(),
    };
    tx_validate_lease_fence(
        tx,
        daemon.authority_generation,
        daemon.daemon_epoch,
        &assertion,
    )?;
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM resource_leases l JOIN capacity_reservations c ON c.owner_type=l.owner_type AND c.owner_id=l.owner_id JOIN execution_reservation_owners o ON o.reservation_id=c.reservation_id WHERE l.lease_id=?1 AND l.resource_kind='execution' AND l.resource_key=?2 AND l.boot_identity=?3 AND l.boot_identity=?4 AND c.reservation_id=?5 AND c.state IN ('held','reconciling') AND c.authority_generation=?6 AND l.authority_generation=?6 AND c.daemon_epoch=?7 AND ((c.owner_type='agent_invocation' AND o.invocation_id=c.owner_id AND o.attempt_id IS NOT NULL) OR (c.owner_type='replay_worker' AND o.replay_worker_id=c.owner_id) OR (c.owner_type='provider_probe' AND o.provider_probe_id=c.owner_id)))",params![observation.lease_id.to_string(),format!("{}:{}",observation.unit.kind(),observation.unit.id()),observation.boot_identity,daemon.boot_identity,observation.reservation_id.to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?],|row|row.get(0))?;
    if !valid {
        return Err(StoreError::StaleFence {
            message: "checkpoint d'une autre incarnation ou réservation".to_owned(),
        });
    }
    let incarnation_sql = match observation.unit {
        AdmissionUnit::Invocation(_) => {
            "SELECT EXISTS(SELECT 1 FROM invocation_attempts t JOIN execution_reservation_owners o ON o.attempt_id=t.attempt_id WHERE o.reservation_id=?1 AND t.invocation_id=?2 AND t.authority_generation=?3 AND t.daemon_epoch=?4 AND t.fence=?5 AND t.state IN ('admitted','running','checkpointing','reconciling'))"
        }
        AdmissionUnit::Replay(_) => {
            "SELECT EXISTS(SELECT 1 FROM replay_workers w WHERE EXISTS(SELECT 1 FROM execution_reservation_owners o WHERE o.reservation_id=?1 AND o.replay_worker_id=w.replay_worker_id) AND w.replay_worker_id=?2 AND w.authority_generation=?3 AND w.daemon_epoch=?4 AND w.fence=?5 AND w.state IN ('admitted','running','checkpointing','reconciling'))"
        }
        AdmissionUnit::Probe(_) => {
            "SELECT EXISTS(SELECT 1 FROM provider_probes p WHERE EXISTS(SELECT 1 FROM execution_reservation_owners o WHERE o.reservation_id=?1 AND o.provider_probe_id=p.probe_id) AND p.probe_id=?2 AND p.authority_generation=?3 AND p.daemon_epoch=?4 AND p.fence=?5 AND p.state IN ('admitted','running','reconciling'))"
        }
    };
    let incarnation: bool = tx.query_row(
        incarnation_sql,
        params![
            observation.reservation_id.to_string(),
            observation.unit.id().to_string(),
            observation.authority_generation.to_string(),
            integer(observation.daemon_epoch.as_u64())?,
            integer(observation.fence.as_u64())?
        ],
        |row| row.get(0),
    )?;
    if !incarnation {
        return Err(StoreError::StaleFence {
            message: "lease sans tentative ou activation correspondante".to_owned(),
        });
    }
    let (table, key) = observation.unit.table();
    let current: i64 = tx.query_row(
        &format!("SELECT revision FROM {table} WHERE {key}=?1"),
        [observation.unit.id().to_string()],
        |row| row.get(0),
    )?;
    if unsigned(current)? != observation.unit_revision.as_u64() {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(unsigned(current)?),
            expected_revision: observation.unit_revision,
            command_id: None,
        });
    }
    Ok(daemon)
}

fn checkpoint_proofs(
    tx: &Transaction<'_>,
    observation: &ExecutionCheckpointObservation,
) -> Result<(), StoreError> {
    let proofs = [
        ("checkpoint-published", observation.checkpoint_published),
        ("local-model-quiescent", observation.local_model_quiescent),
        (
            "continuation-neutralized",
            observation.continuation_neutralized,
        ),
        ("resources-disposed", observation.resources_disposed),
    ];
    let distinct: std::collections::BTreeSet<_> = proofs.iter().map(|(_, id)| *id).collect();
    if distinct.len() != proofs.len() {
        return Err(invalid(
            "les observations de checkpoint doivent être distinctes",
        ));
    }
    for (role, id) in proofs {
        verify_evidence(tx, id, &observation.evidence_binding(role))?;
    }
    Ok(())
}

pub(crate) fn request_checkpoint(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    waiting: InvocationId,
    observation: &ExecutionContinuation,
) -> Result<EventCursor, StoreError> {
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid(
            "observation urgente non qualifiée : horloge incertaine",
        ));
    }
    let daemon = checkpoint_lease(tx, observation)?;
    let control:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM invocations i JOIN agents a ON a.agent_id=i.agent_id WHERE i.invocation_id=?1 AND i.state='queued' AND i.admission_class='control' AND a.level IN ('N+1','N+2'))",[waiting.to_string()],|row|row.get(0))?;
    if !control {
        return Err(invalid("contrôle en attente non prouvé"));
    }
    let pool: String = tx.query_row(
        "SELECT pool_id FROM capacity_reservations WHERE reservation_id=?1",
        [observation.reservation_id.to_string()],
        |row| row.get(0),
    )?;
    let snapshot = read_snapshot(tx, PoolId::from_uuid(uuid(&pool)?), now)?;
    if snapshot.pool.current_occupation < snapshot.pool.effective_limit
        || !snapshot.queued.iter().any(|c| {
            c.unit == AdmissionUnit::Invocation(waiting)
                && c.eligibility_evidence.is_some()
                && c.urgency == AdmissionUrgency::UrgentControl
        })
    {
        return Err(invalid(
            "contrôle non admissible ou absence de pression capacité",
        ));
    }
    let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM execution_checkpoint_requests r JOIN capacity_reservations c ON c.reservation_id=r.reservation_id WHERE r.waiting_control=?1 AND c.state IN ('held','reconciling'))",[waiting.to_string()],|row|row.get(0))?;
    if pending {
        return Err(invalid(
            "checkpoint déjà demandé ; escalade sans observation monotone interdite",
        ));
    }
    let work: bool = tx.query_row(
        "SELECT admission_class='work' FROM capacity_reservations WHERE reservation_id=?1",
        [observation.reservation_id.to_string()],
        |row| row.get(0),
    )?;
    if !work {
        return Err(invalid("un contrôle ne préempte pas un autre contrôle"));
    }
    tx.execute("INSERT INTO execution_checkpoint_requests(reservation_id,waiting_control,requested_at_ms,authority_generation,daemon_epoch) VALUES(?1,?2,?3,?4,?5)",params![observation.reservation_id.to_string(),waiting.to_string(),now.as_i64(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?])?;
    append_event(
        tx,
        config,
        &daemon,
        now,
        observation.unit,
        observation.unit_revision,
        project_for(tx, observation.unit)?,
        match observation.unit {
            AdmissionUnit::Invocation(_) => "InvocationCheckpointRequested",
            AdmissionUnit::Replay(_) => "ReplayWorkerCheckpointRequested",
            AdmissionUnit::Probe(_) => "ProviderProbeCheckpointRequested",
        },
        &json!({"reservation_id":observation.reservation_id,"waiting_control":waiting}),
    )
}

pub(crate) fn record_checkpoint(
    tx: &Transaction<'_>,
    now: TimestampMs,
    observation: &ExecutionCheckpointObservation,
) -> Result<(), StoreError> {
    checkpoint_lease(tx, &observation.continuation())?;
    checkpoint_proofs(tx, observation)?;
    let raw = serde_json::to_string(observation)?;
    let prior:Option<String>=tx.query_row("SELECT observation_json FROM execution_checkpoint_observations WHERE reservation_id=?1",[observation.reservation_id.to_string()],|row|row.get(0)).optional()?;
    if let Some(prior) = prior {
        if prior != raw {
            return Err(invalid("observation checkpoint divergente"));
        }
        return Ok(());
    }
    tx.execute("INSERT INTO execution_checkpoint_observations(reservation_id,observation_json,recorded_at_ms) VALUES(?1,?2,?3)",params![observation.reservation_id.to_string(),raw,now.as_i64()])?;
    Ok(())
}

pub(crate) fn complete_checkpoint(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    reservation: ReservationId,
    reason: unleash_domain::WaitReason,
) -> Result<EventCursor, StoreError> {
    let raw:String=tx.query_row("SELECT observation_json FROM execution_checkpoint_observations WHERE reservation_id=?1",[reservation.to_string()],|row|row.get(0)).optional()?.ok_or_else(||invalid("observation checkpoint non persistée"))?;
    let observation: ExecutionCheckpointObservation = serde_json::from_str(&raw)?;
    let daemon = checkpoint_lease(tx, &observation.continuation())?;
    checkpoint_proofs(tx, &observation)?;
    let (table, key) = observation.unit.table();
    let state: String = tx.query_row(
        &format!("SELECT state FROM {table} WHERE {key}=?1"),
        [observation.unit.id().to_string()],
        |row| row.get(0),
    )?;
    if !matches!(state.as_str(), "running" | "checkpointing" | "reconciling") {
        return Err(invalid("état hors frontière de checkpoint"));
    }
    let next = Revision::new(
        observation
            .unit_revision
            .as_u64()
            .checked_add(1)
            .ok_or_else(|| invalid("révision épuisée"))?,
    );
    let target = if matches!(observation.unit, AdmissionUnit::Probe(_)) {
        "ended"
    } else {
        "waiting_durable"
    };
    tx.execute(
        &format!(
            "UPDATE {table} SET state=?1,wait_reason=?2,revision=?3 WHERE {key}=?4 AND revision=?5"
        ),
        params![
            target,
            if target == "ended" {
                None
            } else {
                Some(reason.to_string())
            },
            integer(next.as_u64())?,
            observation.unit.id().to_string(),
            integer(observation.unit_revision.as_u64())?
        ],
    )?;
    if let AdmissionUnit::Invocation(id) = observation.unit {
        let changed=tx.execute("UPDATE invocation_attempts SET state='ended',ended_at_ms=?1 WHERE attempt_id=(SELECT attempt_id FROM execution_reservation_owners WHERE reservation_id=?2) AND invocation_id=?3 AND authority_generation=?4 AND daemon_epoch=?5 AND fence=?6 AND state IN ('running','checkpointing','reconciling')",params![now.as_i64(),reservation.to_string(),id.to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?,integer(observation.fence.as_u64())?])?;
        if changed != 1 {
            return Err(StoreError::StaleFence {
                message: "ancienne tentative de checkpoint".to_owned(),
            });
        }
    }
    if let AdmissionUnit::Replay(id) = observation.unit {
        tx.execute("UPDATE replay_workers SET checkpoint_hash=(SELECT object_hash FROM evidence WHERE evidence_id=?1) WHERE replay_worker_id=?2",params![observation.checkpoint_published.to_string(),id.to_string()])?;
    }
    tx_release_lease(tx, observation.lease_id)?;
    tx_release_capacity_reservation_verified(tx, now, reservation)?;
    append_event(
        tx,
        config,
        &daemon,
        now,
        observation.unit,
        next,
        project_for(tx, observation.unit)?,
        match observation.unit {
            AdmissionUnit::Invocation(_) => "InvocationCheckpointed",
            AdmissionUnit::Replay(_) => "ReplayWorkerCheckpointed",
            AdmissionUnit::Probe(_) => "ProviderProbeCheckpointed",
        },
        &json!({"reservation_id":reservation,"checkpoint_evidence":observation.checkpoint_published,"neutralization_evidence":observation.continuation_neutralized,"wait_reason":reason}),
    )
}

pub(crate) fn requeue(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    unit: AdmissionUnit,
    expected: Revision,
) -> Result<EventCursor, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    check_lineage(&daemon, daemon.authority_generation, daemon.daemon_epoch)?;
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM capacity_reservations c JOIN execution_checkpoint_observations o ON o.reservation_id=c.reservation_id WHERE c.owner_type=?1 AND c.owner_id=?2 AND c.state='released' AND c.authority_generation=?3 AND c.daemon_epoch=?4) AND NOT EXISTS(SELECT 1 FROM capacity_reservations WHERE owner_type=?1 AND owner_id=?2 AND state IN ('held','reconciling'))",params![unit.kind(),unit.id().to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?],|row|row.get(0))?;
    if !valid {
        return Err(invalid(
            "ancienne continuation non disposée dans cette incarnation",
        ));
    }
    let (table, key) = unit.table();
    let next = expected
        .as_u64()
        .checked_add(1)
        .ok_or_else(|| invalid("révision épuisée"))?;
    let date_column = "queued_at_ms";
    let replay_ordinal = if matches!(unit, AdmissionUnit::Replay(_)) {
        ",attempt_ordinal=attempt_ordinal+1"
    } else {
        ""
    };
    let changed=tx.execute(&format!("UPDATE {table} SET state='queued',wait_reason=NULL,revision=?1,{date_column}=?2{replay_ordinal} WHERE {key}=?3 AND revision=?4 AND state='waiting_durable'"),params![integer(next)?,now.as_i64(),unit.id().to_string(),integer(expected.as_u64())?])?;
    if changed != 1 {
        return Err(invalid(
            "réadmission depuis un état ou une révision interdits",
        ));
    }
    append_event(
        tx,
        config,
        &daemon,
        now,
        unit,
        Revision::new(next),
        project_for(tx, unit)?,
        match unit {
            AdmissionUnit::Invocation(_) => "InvocationQueued",
            AdmissionUnit::Replay(_) => "ReplayWorkerQueued",
            AdmissionUnit::Probe(_) => "ProviderProbeQueued",
        },
        &json!({"resumed_after_checkpoint":true}),
    )
}
