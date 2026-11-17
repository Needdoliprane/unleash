//! Algorithme transactionnel de commande (`CommitCommand`), reçus, tombstones et purge (`REQ-PERSIST-001`, `REQ-PERSIST-004`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unleash_domain::{
    ActionId, ActorId, AgentId, AgentLevel, AgentState, AttemptId, AuthorityGeneration, CommandId,
    CompletionPolicy, DaemonEpoch, DaemonMode, GoalId, GoalState, InvocationId, InvocationState,
    MissionId, MissionState, OperationId, ProjectId, ReceiptId, ReplayWorkerId, Revision, RunId,
    Sha256Digest, StepId, StreamId, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, CommandReceiptInspection, EventCausationKind, ReceiptState,
    Sensitivity,
};
use uuid::Uuid;

use crate::budgets::{BudgetReservationRequest, CapacityReservationRequest};
use crate::leases::FenceAssertion;

/// Nature d'agrégat vérifié lors d'une commande.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateKind {
    /// État global du daemon (`daemon_state`).
    Daemon,
    /// Projet (`projects`).
    Project,
    /// Objectif (`goals`).
    Goal,
    /// Mission (`missions`).
    Mission,
    /// Exécution de workflow (`workflow_runs`).
    WorkflowRun,
    /// Agent logique (`agents`).
    Agent,
    /// Invocation durable (`invocations`).
    Invocation,
    /// Action broker (`actions`).
    Action,
    /// Entité générique persistée dans une table clé/révision dédiée ou `workflow_runs`/`missions`.
    GenericRun,
}

impl AggregateKind {
    /// Nom canonique stocké dans `events.aggregate_type`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::Project => "project",
            Self::Goal => "goal",
            Self::Mission => "mission",
            Self::WorkflowRun | Self::GenericRun => "workflow_run",
            Self::Agent => "agent",
            Self::Invocation => "invocation",
            Self::Action => "action",
        }
    }
}

/// Mutation pure décidée par le domaine et exécutée atomiquement par [`CommitCommandRequest`].
#[derive(Debug, Clone)]
pub enum CommandMutation {
    /// Crée un nouveau projet.
    CreateProject {
        /// Identifiant du projet.
        project_id: ProjectId,
        /// Nom du projet.
        name: String,
        /// Poids d'équité (1..=10).
        weight: i64,
    },
    /// Crée un nouvel agent logique.
    CreateAgent {
        /// Identifiant de l'agent.
        agent_id: AgentId,
        /// Niveau hiérarchique (`N`, `N+1`, `N+2`).
        level: AgentLevel,
        /// Parent organisationnel optionnel.
        organization_parent_id: Option<AgentId>,
        /// Créateur immuable optionnel.
        created_by_agent_id: Option<AgentId>,
        /// Projet rattaché optionnel.
        project_id: Option<ProjectId>,
        /// État initial.
        state: AgentState,
    },
    /// Crée un objectif.
    CreateGoal {
        /// Identifiant de l'objectif.
        goal_id: GoalId,
        /// Projet propriétaire.
        project_id: ProjectId,
        /// Propriétaire humain (`principal_id`) ou agent (`agent_id`).
        owner_principal_id: Option<String>,
        /// Agent propriétaire N+2.
        owner_agent_id: Option<AgentId>,
        /// Politique de complétion.
        completion_policy: CompletionPolicy,
        /// Référence de définition.
        definition_ref: String,
    },
    /// Crée une mission.
    CreateMission {
        /// Identifiant de mission.
        mission_id: MissionId,
        /// Objectif parent optionnel.
        goal_id: Option<GoalId>,
        /// Projet propriétaire.
        project_id: ProjectId,
        /// Propriétaire utilisateur optionnel.
        owner_principal_id: Option<String>,
        /// Agent propriétaire optionnel.
        owner_agent_id: Option<AgentId>,
        /// Politique de complétion.
        completion_policy: CompletionPolicy,
        /// État initial (`proposed` ou `ready`).
        state: MissionState,
    },
    /// Met à jour l'état d'une mission (ex: `PauseMission`, `ResumeMission`, `CancelMission`).
    TransitionMission {
        /// Identifiant de mission.
        mission_id: MissionId,
        /// Nouvel état cible.
        target_state: MissionState,
    },
    /// Crée un `WorkflowRun` (avec contrôle `predecessor_run_id` terminal et unicité de successeur).
    CreateWorkflowRun {
        /// Identifiant du run.
        run_id: RunId,
        /// Identifiant de mission.
        mission_id: MissionId,
        /// Version de workflow épinglée.
        workflow_version_id: String,
        /// Finalité (`operational` ou `evaluation`).
        purpose: String,
        /// Candidat d'évaluation éventuel.
        candidate_ref: Option<String>,
        /// Prédécesseur optionnel.
        predecessor_run_id: Option<RunId>,
        /// Motif de sélection.
        selection_reason: String,
        /// Hash du bundle effectif.
        effective_bundle_hash: Sha256Digest,
    },
    /// Fait transiter un `WorkflowRun` (`StartRun`, `PauseRun`, `ResumeRun`, `CancelRun`, etc.).
    TransitionWorkflowRun {
        /// Identifiant du run.
        run_id: RunId,
        /// Nouvel état cible (après validation pure).
        target_state: unleash_domain::RunState,
        /// Horodatage de fin si terminal.
        ended_at_ms: Option<TimestampMs>,
    },
    /// Enregistre une nouvelle action durable préparée.
    CreateAction {
        /// Identifiant d'action.
        action_id: ActionId,
        /// Origine (`invocation`, `replay` ou `control_plane`).
        origin_kind: String,
        /// Principal de contrôle si `control_plane`.
        control_principal_id: Option<String>,
        /// Run éventuel.
        run_id: Option<RunId>,
        /// Step éventuel.
        step_id: Option<StepId>,
        /// Invocation éventuelle.
        invocation_id: Option<InvocationId>,
        /// Tentative éventuelle.
        attempt_id: Option<AttemptId>,
        /// Worker de replay éventuel.
        replay_worker_id: Option<ReplayWorkerId>,
        /// Type d'action.
        action_type: String,
        /// Hash immuable du payload.
        payload_hash: Sha256Digest,
        /// Stratégie de réconciliation.
        reconcile_strategy: String,
        /// Révision de politique.
        policy_revision: i64,
        /// Lease optionnelle associée.
        resource_lease_id: Option<String>,
    },
    /// Met à jour le mode du daemon (`StopDaemon`, etc.).
    TransitionDaemonMode {
        /// Mode cible.
        target_mode: DaemonMode,
    },
    /// Met à jour l'état d'un objectif.
    TransitionGoal {
        /// Identifiant d'objectif.
        goal_id: GoalId,
        /// État cible.
        target_state: GoalState,
    },
    /// Met à jour l'état d'une invocation.
    TransitionInvocation {
        /// Identifiant d'invocation.
        invocation_id: InvocationId,
        /// État cible.
        target_state: InvocationState,
        /// Raison d'attente optionnelle.
        wait_reason: Option<unleash_domain::WaitReason>,
    },
    /// Mutation neutre sur `daemon_state` (pour tests de bout en bout / charge WAL / receipts).
    NoopDaemonBump,
}

/// Événement à journaliser dans la même transaction que la mutation.
#[derive(Debug, Clone)]
pub struct EventAppendSpec {
    /// Type d'événement `PascalCase`.
    pub event_type: String,
    /// Agrégat modifié.
    pub aggregate_type: AggregateKind,
    /// Identifiant de l'agrégat modifié.
    pub aggregate_id: Uuid,
    /// Révision résultante (>= 1).
    pub aggregate_revision: Revision,
    /// Projet associé.
    pub project_id: Option<ProjectId>,
    /// Identifiant de corrélation.
    pub correlation_id: Uuid,
    /// Cause (par défaut `Command`, ce qui lie `causation_receipt_id` au reçu créé).
    pub causation_kind: EventCausationKind,
    /// ID de cause si différent de `Command`.
    pub causation_id: Option<String>,
    /// Payload JSON borné.
    pub payload: serde_json::Map<String, Value>,
    /// Sensibilité.
    pub sensitivity: Sensitivity,
    /// Consommateurs internes à notifier via `outbox`.
    pub outbox_destinations: Vec<String>,
}

/// Requête complète soumise à `CommitCommand`.
#[derive(Debug, Clone)]
pub struct CommitCommandRequest {
    /// Acteur authentifié côté serveur (clé d'idempotence avec `command_id`).
    pub actor_id: ActorId,
    /// Enveloppe v2 validée.
    pub envelope: CommandEnvelope,
    /// Agrégat ciblé pour vérification d'`expected_revision`.
    pub target_aggregate: AggregateKind,
    /// Opération asynchrone longue éventuelle à lier au reçu.
    pub operation_id: Option<OperationId>,
    /// État final du reçu (par défaut `Accepted`).
    pub receipt_state: ReceiptState,
    /// Référence opaque de résultat.
    pub result_ref: Option<String>,
    /// Fenêtre de conservation complète (1..=90 jours, `None` = défaut config 7 jours).
    pub retention_days_override: Option<u32>,
    /// Vérification optionnelle de fence/lease dans la lignée active.
    pub fence_assertion: Option<FenceAssertion>,
    /// Mutation de domaine.
    pub mutation: CommandMutation,
    /// Événements à écrire atomiquement (le dernier donne son curseur au reçu).
    pub events: Vec<EventAppendSpec>,
    /// Réservations de capacité atomiques associées.
    pub capacity_reservations: Vec<CapacityReservationRequest>,
    /// Réservations budgétaires atomiques associées.
    pub budget_reservations: Vec<BudgetReservationRequest>,
}

/// Issue d'un appel à `CommitCommand`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    /// Nouvelle décision engagée durablement pour la première fois.
    Committed(CommandReceipt),
    /// Rejeu idempotent exact dans la même lignée et fenêtre active.
    IdempotentReplay(CommandReceipt),
}

impl CommitOutcome {
    /// Renvoie le reçu quel que soit le chemin (`Committed` ou `IdempotentReplay`).
    #[must_use]
    pub const fn receipt(&self) -> &CommandReceipt {
        match self {
            Self::Committed(r) | Self::IdempotentReplay(r) => r,
        }
    }
}

/// Résultat d'inspection d'un reçu (`GetCommandReceipt`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptInspectionOutcome {
    /// Reçu complet toujours dans sa fenêtre de conservation.
    Full(CommandReceiptInspection),
    /// Réponse purgée en tombstone minimal ; clé et causalité conservées.
    Tombstone {
        /// Identifiant serveur unique du reçu.
        receipt_id: ReceiptId,
        /// Identifiant de commande client.
        command_id: CommandId,
        /// Lignée d'origine.
        authority_generation: AuthorityGeneration,
        /// Statut de lignée (`Current` ou `Historical`).
        lineage_status: unleash_protocol::LineageStatus,
        /// Digest canonique enregistré.
        command_digest: Sha256Digest,
        /// État historique du reçu.
        state: ReceiptState,
        /// Horodatage d'engagement.
        committed_at_ms: TimestampMs,
    },
}

/// Rapport de purge des réponses de commandes expirées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptPurgeReport {
    /// Purge exécutée avec nombre de réponses converties en tombstones.
    Purged {
        /// Nombre de reçus purgés en tombstones.
        tombstoned_count: usize,
    },
    /// Purge suspendue car l'horloge est déclarée incertaine.
    SuspendedClockUncertain,
}

/// État courant d'un `daemon_state` lu en base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStateRow {
    /// UID propriétaire.
    pub owner_uid: u32,
    /// Lignée d'autorité active.
    pub authority_generation: AuthorityGeneration,
    /// Époque dans la lignée.
    pub daemon_epoch: DaemonEpoch,
    /// Identifiant d'instance.
    pub daemon_instance_id: unleash_domain::DaemonInstanceId,
    /// Identité de boot OS.
    pub boot_identity: String,
    /// Identifiant du flux d'événements.
    pub stream_id: StreamId,
    /// Version de schéma active.
    pub schema_version: i64,
    /// Mode opérationnel courant.
    pub mode: DaemonMode,
    /// Révision du singleton daemon.
    pub revision: Revision,
    /// Horodatage de démarrage.
    pub started_at_ms: TimestampMs,
}

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction};
use unleash_domain::RunState;
use unleash_protocol::{
    EventCursor, LineageStatus, canonical_sha256, compute_command_digest_v2, from_json_str_strict,
};

use crate::budgets::{tx_reserve_budgets, tx_reserve_capacity};
use crate::config::{CutpointObserver, StoreConfig, StoreCutpoint};
use crate::error::StoreError;
use crate::leases::tx_validate_lease_fence;

pub(crate) fn decode_command_envelope_json(raw_json: &str) -> Result<CommandEnvelope, StoreError> {
    match from_json_str_strict::<CommandEnvelope>(raw_json) {
        Ok(env) => Ok(env),
        Err(unleash_protocol::ProtocolError::ProtocolIncompatible { version }) => {
            Err(StoreError::ProtocolIncompatible { version })
        }
        Err(unleash_protocol::ProtocolError::Json(err)) => {
            let msg = err.to_string();
            if msg.contains("[protocol_incompatible:") || msg.contains("unleash.ipc.v1") {
                Err(StoreError::ProtocolIncompatible { version: msg })
            } else {
                Err(StoreError::Protocol(unleash_protocol::ProtocolError::Json(
                    err,
                )))
            }
        }
        Err(other) => Err(StoreError::Protocol(other)),
    }
}

pub(crate) fn read_daemon_state_conn(conn: &Connection) -> Result<DaemonStateRow, StoreError> {
    conn.query_row(
        "SELECT owner_uid, authority_generation, daemon_epoch, daemon_instance_id, boot_identity,
                stream_id, schema_version, mode, revision, started_at_ms
         FROM daemon_state WHERE singleton = 1",
        [],
        map_daemon_state_row,
    )
    .map_err(StoreError::from_rusqlite)
}

pub(crate) fn read_daemon_state_tx(tx: &Transaction<'_>) -> Result<DaemonStateRow, StoreError> {
    tx.query_row(
        "SELECT owner_uid, authority_generation, daemon_epoch, daemon_instance_id, boot_identity,
                stream_id, schema_version, mode, revision, started_at_ms
         FROM daemon_state WHERE singleton = 1",
        [],
        map_daemon_state_row,
    )
    .map_err(StoreError::from_rusqlite)
}

fn map_daemon_state_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DaemonStateRow> {
    let owner_uid_i64: i64 = r.get(0)?;
    let auth_str: String = r.get(1)?;
    let epoch_i64: i64 = r.get(2)?;
    let inst_str: String = r.get(3)?;
    let boot_id: String = r.get(4)?;
    let stream_str: String = r.get(5)?;
    let schema_ver: i64 = r.get(6)?;
    let mode_str: String = r.get(7)?;
    let rev_i64: i64 = r.get(8)?;
    let started_i64: i64 = r.get(9)?;

    let auth = auth_str
        .parse::<AuthorityGeneration>()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let inst = inst_str
        .parse::<unleash_domain::DaemonInstanceId>()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let stream_id = stream_str
        .parse::<StreamId>()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let mode = parse_daemon_mode(&mode_str).ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(7, "mode".to_owned(), rusqlite::types::Type::Text)
    })?;

    Ok(DaemonStateRow {
        owner_uid: u32::try_from(owner_uid_i64).unwrap_or(0),
        authority_generation: auth,
        daemon_epoch: DaemonEpoch::new(u64::try_from(epoch_i64).unwrap_or(0)),
        daemon_instance_id: inst,
        boot_identity: boot_id,
        stream_id,
        schema_version: schema_ver,
        mode,
        revision: Revision::new(u64::try_from(rev_i64).unwrap_or(0)),
        started_at_ms: TimestampMs::new(started_i64),
    })
}

fn parse_daemon_mode(s: &str) -> Option<DaemonMode> {
    match s {
        "starting" => Some(DaemonMode::Starting),
        "recovering" => Some(DaemonMode::Recovering),
        "ready" => Some(DaemonMode::Ready),
        "degraded" => Some(DaemonMode::Degraded),
        "read_only_recovery" => Some(DaemonMode::ReadOnlyRecovery),
        "draining" => Some(DaemonMode::Draining),
        "stopped" => Some(DaemonMode::Stopped),
        _ => None,
    }
}

pub(crate) fn compute_validated_command_params(
    config: &StoreConfig,
    req: &CommitCommandRequest,
) -> Result<(Sha256Digest, u32), StoreError> {
    let retention_days = req
        .retention_days_override
        .unwrap_or(config.default_receipt_retention_days);
    if !(1..=90).contains(&retention_days) {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("fenêtre de rétention '{retention_days}' hors [1..=90] jours"),
        });
    }
    let digest = compute_command_digest_v2(&req.envelope)?;
    Ok((digest, retention_days))
}

/// Exécute l'algorithme transactionnel complet `CommitCommand` dans `BEGIN IMMEDIATE` (REQ-PERSIST-001/004).
#[expect(
    clippy::too_many_lines,
    reason = "Algorithme atomique unifié : contrôle lignée, lookup idempotence, mutation, événements, outbox, réserves et receipt"
)]
pub(crate) fn tx_commit_command(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now_ms: TimestampMs,
    req: &CommitCommandRequest,
    command_digest: Sha256Digest,
    retention_days: u32,
    cutpoints: &dyn CutpointObserver,
) -> Result<CommitOutcome, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;

    // Étape 1 : Contrôler `authority_generation` AVANT tout lookup de reçu (REQ-PERSIST-004)
    if req.envelope.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: req.envelope.authority_generation,
            command_id: Some(req.envelope.command_id),
        });
    }

    // Étape 2 : Rechercher `(actor_id, command_id)` dans `command_receipts`
    let actor_str = req.actor_id.as_str();
    let cmd_str = req.envelope.command_id.to_string();
    let existing: Option<(String, String, String, Option<String>)> = tx
        .query_row(
            "SELECT receipt_id, command_digest, response_state, response_json
             FROM command_receipts WHERE actor_id = ?1 AND command_id = ?2",
            rusqlite::params![actor_str, cmd_str],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    if let Some((rcpt_id_str, stored_digest, resp_state, resp_json)) = existing {
        if stored_digest != command_digest.as_prefixed_str() {
            return Err(StoreError::IdempotencyConflict {
                command_id: req.envelope.command_id,
            });
        }
        if resp_state == "tombstone" {
            let receipt_id = rcpt_id_str.parse::<ReceiptId>()?;
            return Err(StoreError::IdempotencyWindowExpired {
                command_id: req.envelope.command_id,
                receipt_id,
            });
        }
        let json_str = resp_json.ok_or_else(|| {
            StoreError::InvariantViolation("reçu full sans response_json".to_owned())
        })?;
        let receipt: CommandReceipt = serde_json::from_str(&json_str)?;
        return Ok(CommitOutcome::IdempotentReplay(receipt));
    }

    // Mode `read_only_recovery` refuse les nouvelles écritures métier ordinaires
    if daemon.mode == DaemonMode::ReadOnlyRecovery
        && !matches!(req.mutation, CommandMutation::TransitionDaemonMode { .. })
    {
        return Err(StoreError::CapacityBlocked(
            "daemon en mode read_only_recovery".to_owned(),
        ));
    }

    // Étape 3 : Vérifier fence, expected_revision et appliquer la décision pure
    if let Some(ref assertion) = req.fence_assertion {
        tx_validate_lease_fence(
            tx,
            daemon.authority_generation,
            daemon.daemon_epoch,
            assertion,
        )?;
    }

    let expected_rev = req.envelope.expected_revision;
    let new_rev = apply_command_mutation(
        tx,
        &daemon,
        now_ms,
        expected_rev,
        &req.mutation,
        req.envelope.command_id,
    )?;

    // Réservations atomiques de capacité et de budgets
    for cap_req in &req.capacity_reservations {
        tx_reserve_capacity(
            tx,
            daemon.authority_generation,
            daemon.daemon_epoch,
            now_ms,
            cap_req,
        )?;
    }
    for budget_req in &req.budget_reservations {
        tx_reserve_budgets(tx, config.id_generator.as_ref(), budget_req)?;
    }

    // Étape 4 : Écrire events + outbox + command_receipts dans la MÊME transaction
    let receipt_id = ReceiptId::from_uuid(config.id_generator.new_uuid());
    let receipt_id_str = receipt_id.to_string();

    let mut first_seq: Option<i64> = None;
    let mut last_seq: Option<i64> = None;

    for (ordinal, ev) in req.events.iter().enumerate() {
        let event_id = config.id_generator.new_uuid().to_string();
        let payload_json = serde_json::to_string(&ev.payload)?;
        let payload_hash = canonical_sha256(&ev.payload)?;
        let (causation_kind_str, causation_id_str, causation_rcpt_opt) = match ev.causation_kind {
            EventCausationKind::Command => (
                "command",
                receipt_id_str.clone(),
                Some(receipt_id_str.clone()),
            ),
            EventCausationKind::Event => (
                "event",
                ev.causation_id.clone().unwrap_or_else(|| event_id.clone()),
                None,
            ),
            EventCausationKind::Action => {
                ("action", ev.causation_id.clone().unwrap_or_default(), None)
            }
            EventCausationKind::Timer => {
                ("timer", ev.causation_id.clone().unwrap_or_default(), None)
            }
            EventCausationKind::External => (
                "external",
                ev.causation_id.clone().unwrap_or_default(),
                None,
            ),
            other => {
                return Err(StoreError::InvariantViolation(format!(
                    "type de causalité non supporté : {other:?}"
                )));
            }
        };
        let sensitivity_str = match ev.sensitivity {
            Sensitivity::Public => "public",
            Sensitivity::Project => "project",
            Sensitivity::Confidential => "confidential",
            Sensitivity::Restricted => "restricted",
            other => {
                return Err(StoreError::InvariantViolation(format!(
                    "niveau de sensibilité non supporté : {other:?}"
                )));
            }
        };
        let ord_i64 = i64::try_from(ordinal).unwrap_or(0);
        let agg_rev_i64 = i64::try_from(ev.aggregate_revision.as_u64()).unwrap_or(1);

        tx.execute(
            "INSERT INTO events
             (event_id, stream_id, aggregate_type, aggregate_id, aggregate_revision, event_ordinal,
              event_type, schema_version, project_id, actor_ref, actor_id, occurred_at_ms, recorded_at_ms,
              payload_json, payload_state, payload_hash, sensitivity, command_actor_id, command_id,
              causation_kind, causation_id, causation_receipt_id, correlation_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?11, ?11, ?12, 'full', ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                event_id,
                daemon.stream_id.to_string(),
                ev.aggregate_type.as_str(),
                ev.aggregate_id.to_string(),
                agg_rev_i64,
                ord_i64,
                ev.event_type,
                ev.project_id.map(|p| p.to_string()),
                actor_str,
                actor_str,
                now_ms.as_i64(),
                payload_json,
                payload_hash.as_prefixed_str(),
                sensitivity_str,
                actor_str,
                cmd_str,
                causation_kind_str,
                causation_id_str,
                causation_rcpt_opt,
                ev.correlation_id.to_string()
            ],
        )
        .map_err(StoreError::from_rusqlite)?;

        let seq = tx.last_insert_rowid();
        if first_seq.is_none() {
            first_seq = Some(seq);
        }
        last_seq = Some(seq);

        for dest in &ev.outbox_destinations {
            let delivery_key = format!("{event_id}:{dest}");
            tx.execute(
                "INSERT INTO outbox
                 (event_id, destination, state, delivery_attempt, next_attempt_at_ms,
                  authority_generation, lease_epoch, consumer_version, delivery_key, acked_at_ms, last_error, rearm_count)
                 VALUES (?1, ?2, 'pending', 0, ?3, ?4, NULL, ?5, ?6, NULL, NULL, 0)",
                rusqlite::params![
                    event_id,
                    dest,
                    now_ms.as_i64(),
                    daemon.authority_generation.to_string(),
                    config.consumer_version,
                    delivery_key
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
    }

    let cursor_seq_u64 = if let Some(s) = last_seq {
        u64::try_from(s).unwrap_or(0)
    } else {
        let max_s: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE stream_id = ?1",
                [daemon.stream_id.to_string()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        u64::try_from(max_s).unwrap_or(0)
    };

    let accepted_at =
        DateTime::<Utc>::from_timestamp_millis(now_ms.as_i64()).unwrap_or(DateTime::UNIX_EPOCH);
    let receipt = CommandReceipt {
        receipt_id,
        command_id: req.envelope.command_id,
        accepted_at,
        aggregate_id: req.envelope.aggregate_id,
        new_revision: new_rev,
        operation_id: req.operation_id,
        state: req.receipt_state,
        event_cursor: EventCursor::new(
            daemon.stream_id,
            unleash_domain::Sequence::new(cursor_seq_u64),
        ),
        result_ref: req.result_ref.clone(),
    };
    let receipt_json = serde_json::to_string(&receipt)?;
    let state_str = match req.receipt_state {
        ReceiptState::Accepted => "accepted",
        ReceiptState::Completed => "completed",
        ReceiptState::Blocked => "blocked",
        ReceiptState::Rejected => "rejected",
        other => {
            return Err(StoreError::InvariantViolation(format!(
                "état de reçu non supporté : {other:?}"
            )));
        }
    };
    let retained_until_ms = now_ms
        .as_i64()
        .saturating_add(i64::from(retention_days).saturating_mul(86_400_000));

    tx.execute(
        "INSERT INTO command_receipts
         (actor_id, command_id, authority_generation, receipt_id, request_id, command_type,
          command_digest, aggregate_id, state, response_state, response_json,
          response_retained_until_ms, first_event_seq, last_event_seq, committed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'full', ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            actor_str,
            cmd_str,
            daemon.authority_generation.to_string(),
            receipt_id_str,
            req.envelope.request_id.to_string(),
            req.envelope.command_type,
            command_digest.as_prefixed_str(),
            req.envelope.aggregate_id.to_string(),
            state_str,
            receipt_json,
            retained_until_ms,
            first_seq,
            last_seq,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    cutpoints.hit(StoreCutpoint::C1BeforeCommandCommit)?;

    Ok(CommitOutcome::Committed(receipt))
}

#[expect(
    clippy::too_many_lines,
    reason = "Dispatch exhaustif des variantes CommandMutation dans la transaction atomique"
)]
fn apply_command_mutation(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now_ms: TimestampMs,
    expected_rev: Revision,
    mutation: &CommandMutation,
    cmd_id: CommandId,
) -> Result<Revision, StoreError> {
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(0);
    match mutation {
        CommandMutation::CreateProject {
            project_id,
            name,
            weight,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            let exists: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM projects WHERE project_id = ?1",
                    [project_id.to_string()],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            if exists > 0 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(1),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            tx.execute(
                "INSERT INTO projects (project_id, name, weight, state, revision, created_at_ms)
                 VALUES (?1, ?2, ?3, 'active', 1, ?4)",
                rusqlite::params![project_id.to_string(), name, weight, now_ms.as_i64()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::CreateAgent {
            agent_id,
            level,
            organization_parent_id,
            created_by_agent_id,
            project_id,
            state,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            tx.execute(
                "INSERT INTO agents (agent_id, level, organization_parent_id, created_by_agent_id, project_id, state, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                rusqlite::params![
                    agent_id.to_string(),
                    level.to_string(),
                    organization_parent_id.map(|a| a.to_string()),
                    created_by_agent_id.map(|a| a.to_string()),
                    project_id.map(|p| p.to_string()),
                    state.to_string()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::CreateGoal {
            goal_id,
            project_id,
            owner_principal_id,
            owner_agent_id,
            completion_policy,
            definition_ref,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            let owner_kind = if owner_principal_id.is_some() {
                "user"
            } else {
                "agent"
            };
            tx.execute(
                "INSERT INTO goals (goal_id, project_id, owner_kind, owner_user_id, owner_agent_id, owner_epoch, completion_policy, state, definition_ref, revision, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, 'draft', ?7, 1, ?8)",
                rusqlite::params![
                    goal_id.to_string(),
                    project_id.to_string(),
                    owner_kind,
                    owner_principal_id,
                    owner_agent_id.map(|a| a.to_string()),
                    completion_policy.to_string(),
                    definition_ref,
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::CreateMission {
            mission_id,
            goal_id,
            project_id,
            owner_principal_id,
            owner_agent_id,
            completion_policy,
            state,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            let exists: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM missions WHERE mission_id = ?1",
                    [mission_id.to_string()],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            if exists > 0 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(1),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let owner_kind = if owner_principal_id.is_some() {
                "user"
            } else {
                "agent"
            };
            tx.execute(
                "INSERT INTO missions (mission_id, goal_id, project_id, owner_kind, owner_user_id, owner_agent_id, completion_policy, leader_agent_id, worktree_id, state, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, 1)",
                rusqlite::params![
                    mission_id.to_string(),
                    goal_id.map(|g| g.to_string()),
                    project_id.to_string(),
                    owner_kind,
                    owner_principal_id,
                    owner_agent_id.map(|a| a.to_string()),
                    completion_policy.to_string(),
                    state.to_string()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::TransitionMission {
            mission_id,
            target_state,
        } => {
            let cur_rev: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM missions WHERE mission_id = ?1",
                    [mission_id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?;
            let Some(rev) = cur_rev else {
                return Err(StoreError::NotFound(format!("mission {mission_id}")));
            };
            if rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = rev.saturating_add(1);
            tx.execute(
                "UPDATE missions SET state = ?1, revision = ?2 WHERE mission_id = ?3",
                rusqlite::params![target_state.to_string(), next_rev, mission_id.to_string()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::CreateWorkflowRun {
            run_id,
            mission_id,
            workflow_version_id,
            purpose,
            candidate_ref,
            predecessor_run_id,
            selection_reason,
            effective_bundle_hash,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            if let Some(pred) = predecessor_run_id {
                let pred_state_str: Option<String> = tx
                    .query_row(
                        "SELECT state FROM workflow_runs WHERE run_id = ?1",
                        [pred.to_string()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(StoreError::from_rusqlite)?;
                let Some(ps) = pred_state_str else {
                    return Err(StoreError::NotFound(format!("run prédécesseur {pred}")));
                };
                let pred_state = parse_run_state(&ps)?;
                if !pred_state.is_terminal() {
                    return Err(StoreError::PredecessorNotTerminal { state: ps });
                }
                let succ_cnt: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM workflow_runs WHERE predecessor_run_id = ?1",
                        [pred.to_string()],
                        |r| r.get(0),
                    )
                    .map_err(StoreError::from_rusqlite)?;
                if succ_cnt > 0 {
                    return Err(StoreError::SuccessorExists);
                }
            }
            tx.execute(
                "INSERT INTO workflow_runs
                 (run_id, mission_id, workflow_version_id, purpose, candidate_ref, predecessor_run_id,
                  selection_reason, selected_via_alias, effective_bundle_hash, state, revision, started_at_ms, ended_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, 'prepared', 1, ?9, NULL)",
                rusqlite::params![
                    run_id.to_string(),
                    mission_id.to_string(),
                    workflow_version_id,
                    purpose,
                    candidate_ref,
                    predecessor_run_id.map(|r| r.to_string()),
                    selection_reason,
                    effective_bundle_hash.as_prefixed_str(),
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::TransitionWorkflowRun {
            run_id,
            target_state,
            ended_at_ms,
        } => {
            let cur_rev: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM workflow_runs WHERE run_id = ?1",
                    [run_id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?;
            let Some(rev) = cur_rev else {
                return Err(StoreError::NotFound(format!("run {run_id}")));
            };
            if rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = rev.saturating_add(1);
            tx.execute(
                "UPDATE workflow_runs SET state = ?1, revision = ?2, ended_at_ms = COALESCE(?3, ended_at_ms)
                 WHERE run_id = ?4",
                rusqlite::params![
                    target_state.to_string(),
                    next_rev,
                    ended_at_ms.map(|t| t.as_i64()),
                    run_id.to_string()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::CreateAction {
            action_id,
            origin_kind,
            control_principal_id,
            run_id,
            step_id,
            invocation_id,
            attempt_id,
            replay_worker_id,
            action_type,
            payload_hash,
            reconcile_strategy,
            policy_revision,
            resource_lease_id,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            let epoch_i64 = i64::try_from(daemon.daemon_epoch.as_u64()).unwrap_or(1);
            tx.execute(
                "INSERT INTO actions
                 (action_id, origin_kind, control_principal_id, run_id, step_id, invocation_id, attempt_id,
                  replay_worker_id, action_type, payload_hash, authorization_ref, policy_revision, resource_lease_id,
                  authority_generation, daemon_epoch, fence, state, reconcile_strategy, provider_idempotency_key, revision, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?12, ?13, ?14, 0, 'prepared', ?15, NULL, 1, ?16)",
                rusqlite::params![
                    action_id.to_string(),
                    origin_kind,
                    control_principal_id,
                    run_id.map(|r| r.to_string()),
                    step_id.map(|s| s.to_string()),
                    invocation_id.map(|i| i.to_string()),
                    attempt_id.map(|a| a.to_string()),
                    replay_worker_id.map(|w| w.to_string()),
                    action_type,
                    payload_hash.as_prefixed_str(),
                    policy_revision,
                    resource_lease_id,
                    daemon.authority_generation.to_string(),
                    epoch_i64,
                    reconcile_strategy,
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::TransitionDaemonMode { target_mode } => {
            let cur_rev = i64::try_from(daemon.revision.as_u64()).unwrap_or(0);
            if cur_rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: daemon.revision,
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_mode = unleash_domain::transition_daemon_mode(daemon.mode, *target_mode)?;
            let next_rev = cur_rev.saturating_add(1);
            tx.execute(
                "UPDATE daemon_state SET mode = ?1, revision = ?2 WHERE singleton = 1",
                rusqlite::params![next_mode.to_string(), next_rev],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::TransitionGoal {
            goal_id,
            target_state,
        } => {
            let cur_rev: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM goals WHERE goal_id = ?1",
                    [goal_id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?;
            let Some(rev) = cur_rev else {
                return Err(StoreError::NotFound(format!("goal {goal_id}")));
            };
            if rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = rev.saturating_add(1);
            tx.execute(
                "UPDATE goals SET state = ?1, revision = ?2 WHERE goal_id = ?3",
                rusqlite::params![target_state.to_string(), next_rev, goal_id.to_string()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::TransitionInvocation {
            invocation_id,
            target_state,
            wait_reason,
        } => {
            let cur_rev: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM invocations WHERE invocation_id = ?1",
                    [invocation_id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?;
            let Some(rev) = cur_rev else {
                return Err(StoreError::NotFound(format!("invocation {invocation_id}")));
            };
            if rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = rev.saturating_add(1);
            let wr = wait_reason.map(|w| w.to_string());
            tx.execute(
                "UPDATE invocations SET state = ?1, wait_reason = ?2, revision = ?3 WHERE invocation_id = ?4",
                rusqlite::params![target_state.to_string(), wr, next_rev, invocation_id.to_string()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::NoopDaemonBump => {
            let cur_rev = i64::try_from(daemon.revision.as_u64()).unwrap_or(0);
            if cur_rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: daemon.revision,
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = cur_rev.saturating_add(1);
            tx.execute(
                "UPDATE daemon_state SET revision = ?1 WHERE singleton = 1",
                [next_rev],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
    }
}

fn ensure_creation_rev(expected_rev: Revision, cmd_id: CommandId) -> Result<(), StoreError> {
    if expected_rev != Revision::INITIAL {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::INITIAL,
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }
    Ok(())
}

fn parse_run_state(s: &str) -> Result<RunState, StoreError> {
    match s {
        "prepared" => Ok(RunState::Prepared),
        "running" => Ok(RunState::Running),
        "paused" => Ok(RunState::Paused),
        "blocked" => Ok(RunState::Blocked),
        "cancelling" => Ok(RunState::Cancelling),
        "succeeded" => Ok(RunState::Succeeded),
        "failed" => Ok(RunState::Failed),
        "cancelled" => Ok(RunState::Cancelled),
        other => Err(StoreError::InvariantViolation(format!(
            "état de run inconnu '{other}'"
        ))),
    }
}

pub(crate) fn read_command_receipt_inspection(
    conn: &Connection,
    actor_id: &ActorId,
    command_id: CommandId,
) -> Result<ReceiptInspectionOutcome, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    let row: Option<(String, String, String, String, String, Option<String>, i64)> = conn
        .query_row(
            "SELECT authority_generation, receipt_id, command_digest, state, response_state, response_json, committed_at_ms
             FROM command_receipts WHERE actor_id = ?1 AND command_id = ?2",
            rusqlite::params![actor_id.as_str(), command_id.to_string()],
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

    let Some((auth_str, rcpt_id_str, digest_str, state_str, resp_state, resp_json, committed_ms)) =
        row
    else {
        return Err(StoreError::NotFound(format!(
            "reçu absent pour ({actor_id}, {command_id})"
        )));
    };

    let authority_generation = auth_str.parse::<AuthorityGeneration>()?;
    let lineage_status = if authority_generation == daemon.authority_generation {
        LineageStatus::Current
    } else {
        LineageStatus::Historical
    };

    if resp_state == "tombstone" {
        let state = match state_str.as_str() {
            "accepted" => ReceiptState::Accepted,
            "completed" => ReceiptState::Completed,
            "blocked" => ReceiptState::Blocked,
            _ => ReceiptState::Rejected,
        };
        return Ok(ReceiptInspectionOutcome::Tombstone {
            receipt_id: rcpt_id_str.parse::<ReceiptId>()?,
            command_id,
            authority_generation,
            lineage_status,
            command_digest: digest_str.parse::<Sha256Digest>()?,
            state,
            committed_at_ms: TimestampMs::new(committed_ms),
        });
    }

    let json_str = resp_json
        .ok_or_else(|| StoreError::InvariantViolation("reçu full sans response_json".to_owned()))?;
    let receipt: CommandReceipt = serde_json::from_str(&json_str)?;
    Ok(ReceiptInspectionOutcome::Full(CommandReceiptInspection {
        receipt,
        authority_generation,
        lineage_status,
        provenance_ref: Some(format!("sqlite:command_receipts:{rcpt_id_str}")),
    }))
}

pub(crate) fn tx_purge_expired_command_receipts(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    clock_uncertain: bool,
) -> Result<ReceiptPurgeReport, StoreError> {
    if clock_uncertain {
        return Ok(ReceiptPurgeReport::SuspendedClockUncertain);
    }

    let count = tx
        .execute(
            "UPDATE command_receipts
             SET response_state = 'tombstone', response_json = NULL
             WHERE response_state = 'full' AND ?1 >= response_retained_until_ms",
            [now_ms.as_i64()],
        )
        .map_err(StoreError::from_rusqlite)?;

    Ok(ReceiptPurgeReport::Purged {
        tombstoned_count: count,
    })
}
