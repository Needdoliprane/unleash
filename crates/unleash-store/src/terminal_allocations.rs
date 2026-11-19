//! Préparation transactionnelle F011 ; aucune identité OS ni opération de host.
//! Le resolver serveur qualifie chemins et environnement. Les fixtures de tests
//! n'attestent ni ce resolver ni un rattachement réel de PTY.

use crate::{CallerAuthority, FenceAssertion, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, DaemonEpoch, DispatchId, Fence, HostId,
    LeaseId, MissionId, PoolId, ProcessId, ProcessRunUuid, ProjectId, ReceiptId, ReservationId,
    Revision, Sha256Digest, TerminalId, TimestampMs,
};

/// Verbe fermé du producteur d'allocation humaine.
pub const TERMINAL_ALLOCATION_VERB: &str = "terminal.acquire";

/// Non-secret environment explicitly configured by an authenticated administrator.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalEnvironmentProfile {
    /// Stable identifier, unique within the active configuration.
    pub profile_id: String,
    /// Immutable version of this profile.
    pub version: u64,
    /// Explicit, closed locale/terminal values; never an inherited environment.
    pub variables: std::collections::BTreeMap<String, String>,
}
impl std::fmt::Debug for TerminalEnvironmentProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalEnvironmentProfile")
            .field("profile_id", &self.profile_id)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl TerminalEnvironmentProfile {
    /// Digest of the complete explicit profile, including its values.
    /// # Errors
    /// Returns a canonical serialization error.
    pub fn digest(&self) -> Result<Sha256Digest, StoreError> {
        Ok(canonical_sha256(self)?)
    }
}

/// Version-one global terminal settings, persisted by the existing SetSettings CAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalRuntimeSettings {
    /// Closed format version.
    pub schema_version: u32,
    /// Explicit non-secret profiles; no implicit default profile exists.
    pub profiles: Vec<TerminalEnvironmentProfile>,
    /// Auxiliary host limit, distinct from execution admission.
    pub max_hosts: u32,
    /// Auxiliary child limit.
    pub max_processes: u32,
    /// Auxiliary PTY limit.
    pub max_ptys: u32,
    /// Explicit lease TTL in milliseconds.
    pub lease_ttl_ms: i64,
}
impl TerminalRuntimeSettings {
    /// Validate the closed initial profile namespace and finite non-secret values.
    /// # Errors
    /// Refuses missing/duplicate profiles, secrets, unknown fields or unsafe bounds.
    pub fn validate(&self) -> Result<(), StoreError> {
        let mut ids = std::collections::BTreeSet::new();
        if self.schema_version != 1
            || !(1..=16).contains(&self.profiles.len())
            || [self.max_hosts, self.max_processes, self.max_ptys]
                .iter()
                .any(|n| !(1..=50).contains(n))
            || !(1000..=300_000).contains(&self.lease_ttl_ms)
        {
            return Err(invalid("invalid terminal runtime settings"));
        }
        for p in &self.profiles {
            if p.profile_id.is_empty()
                || p.profile_id.len() > 64
                || !p.profile_id.bytes().enumerate().all(|(i, b)| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && b"._-".contains(&b))
                })
                || !ids.insert(&p.profile_id)
                || !(1..=9_007_199_254_740_991).contains(&p.version)
                || p.variables.is_empty()
                || p.variables.len() > 4
            {
                return Err(invalid("invalid terminal environment profile"));
            }
            for (key, value) in &p.variables {
                let allowed = match key.as_str() {
                    "LANG" | "LC_ALL" => {
                        ["C", "C.UTF-8", "en_US.UTF-8", "fr_FR.UTF-8"].contains(&value.as_str())
                    }
                    "TERM" => {
                        ["xterm", "xterm-256color", "vt100", "dumb"].contains(&value.as_str())
                    }
                    "TZ" => value == "UTC",
                    _ => false,
                };
                if !allowed {
                    return Err(invalid(
                        "terminal profile contains an unsupported environment value",
                    ));
                }
            }
        }
        Ok(())
    }
    /// Stable identities of the three configured global auxiliary pools.
    #[must_use]
    pub const fn pools(&self) -> TerminalAuxiliaryPools {
        TerminalAuxiliaryPools {
            host: PoolId::from_u128(0xf011_1001),
            child: PoolId::from_u128(0xf011_1002),
            pty: PoolId::from_u128(0xf011_1003),
        }
    }
}

/// Private transport between the authenticated producer and the configuration writer.
/// It is never accepted from the public IPC payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedTerminalEnvironment {
    profile_id: String,
    version: u64,
    digest: Sha256Digest,
    pin_id: String,
    object_generation: i64,
}

pub(crate) fn configure_runtime_tx(
    tx: &Transaction<'_>,
    now: TimestampMs,
    next: &TerminalRuntimeSettings,
    previous: Option<&Value>,
    prepared: &[PreparedTerminalEnvironment],
    command_id: unleash_domain::CommandId,
    ids: &dyn unleash_domain::IdGeneratorPort,
) -> Result<(), StoreError> {
    next.validate()?;
    if let Some(old) = previous {
        let old: TerminalRuntimeSettings = serde_json::from_value(old.clone())?;
        for p in &next.profiles {
            if let Some(prior) = old.profiles.iter().find(|v| v.profile_id == p.profile_id)
                && (p.version < prior.version || (p.version == prior.version && p != prior))
            {
                return Err(invalid("terminal profile version is immutable"));
            }
        }
    }
    if prepared.len() != next.profiles.len() {
        return Err(invalid(
            "terminal environments were not prepared by the service",
        ));
    }
    for p in &next.profiles {
        let digest = p.digest()?.to_string();
        let highest:i64=tx.query_row("SELECT COALESCE(MAX(CAST(source_field AS INTEGER)),0) FROM object_references WHERE owner_kind='terminal_environment' AND owner_id=?1",[&p.profile_id],|r|r.get(0))?;
        if integer(p.version)? < highest {
            return Err(invalid("terminal profile version cannot decrease"));
        }
        let conflict:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM object_references WHERE owner_kind='terminal_environment' AND owner_id=?1 AND source_field=?2 AND object_hash!=?3)",params![p.profile_id,p.version.to_string(),digest],|r|r.get(0))?;
        if conflict {
            return Err(invalid("terminal environment version is conflicting"));
        }
        let pin = prepared
            .iter()
            .find(|v| {
                v.profile_id == p.profile_id
                    && v.version == p.version
                    && v.digest.to_string() == digest
            })
            .ok_or_else(|| invalid("terminal prepared bytes differ from configuration"))?;
        let owner:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM object_pins WHERE pin_id=?1 AND object_hash=?2 AND owner_kind='terminal_configuration_candidate' AND owner_id=?3 AND state='held')",params![pin.pin_id.to_string(),digest,command_id.to_string()],|r|r.get(0))?;
        if !owner {
            return Err(invalid(
                "terminal prepared pin is not owned by this command",
            ));
        }
        crate::objects::tx_publish_object_reference(
            tx,
            ids,
            now,
            &crate::PublishObjectReferenceRequest {
                pin_id: pin.pin_id.clone(),
                object_hash: pin.digest,
                object_generation: pin.object_generation,
                owner_kind: "terminal_environment".into(),
                owner_id: p.profile_id.clone(),
                source_field: p.version.to_string(),
            },
        )?;
    }
    for ((id, kind), limit) in pool_bindings(&next.pools()).into_iter().zip([
        next.max_hosts,
        next.max_processes,
        next.max_ptys,
    ]) {
        let occupation:i64=tx.query_row("SELECT COALESCE(SUM(amount),0) FROM capacity_reservations WHERE pool_id=?1 AND state IN ('held','reconciling')",[id.to_string()],|r|r.get(0))?;
        if occupation > i64::from(limit) {
            return Err(invalid("terminal limit is below held capacity"));
        }
        tx.execute("INSERT INTO capacity_pools(pool_id,pool_kind,scope_key,unit,requested_limit,effective_limit,admissions_frozen,hard_limit,revision) VALUES(?1,?2,'global','slot',?3,?3,0,50,1) ON CONFLICT(pool_id) DO UPDATE SET requested_limit=excluded.requested_limit,effective_limit=excluded.effective_limit,revision=capacity_pools.revision+1",params![id.to_string(),kind,limit])?;
    }
    Ok(())
}

impl crate::UnleashStore {
    /// Read and validate the actual user/default terminal configuration.
    /// # Errors
    /// Missing or invalid configuration is refused, never filled with defaults.
    pub fn terminal_runtime_settings(&self) -> Result<TerminalRuntimeSettings, StoreError> {
        let settings = self.read_settings(crate::SettingsScopeKind::User, "default")?;
        let raw = settings
            .extra_typed
            .get("terminal_runtime")
            .ok_or_else(|| invalid("terminal runtime is not configured"))?;
        let cfg: TerminalRuntimeSettings = serde_json::from_value(raw.clone())?;
        cfg.validate()?;
        Ok(cfg)
    }
    /// Prepare explicit profile bytes without reserving the immutable version.
    /// # Errors
    /// Refuses invalid bytes. Publication belongs exclusively to the accepted SetSettings transaction.
    pub fn prepare_terminal_environment(
        &self,
        p: &TerminalEnvironmentProfile,
        command: unleash_domain::CommandId,
    ) -> Result<PreparedTerminalEnvironment, StoreError> {
        let bytes = unleash_protocol::canonicalize_to_vec(p)?;
        let hash = p.digest()?;
        let handle = self.prepare_object(crate::PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".into(),
            owner_kind: "terminal_configuration_candidate".into(),
            owner_id: command.to_string(),
        })?;
        self.write_prepared_object_bytes(&handle, &bytes)?;
        let record = self.object_record(hash)?;
        crate::objects::verify_object_file_before_publish(
            self.config(),
            &record.relative_path,
            hash,
        )?;
        Ok(PreparedTerminalEnvironment {
            profile_id: p.profile_id.clone(),
            version: p.version,
            digest: hash,
            pin_id: handle.pin_id,
            object_generation: handle.object_generation,
        })
    }
}

/// Source de portée authentifiée du sous-chemin humain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalAllocationScope {
    /// Configuration du projet de la mission, relue dans le writer.
    ProjectConfiguration {
        /// Projet durable.
        project_id: ProjectId,
        /// Révision exacte.
        revision: Revision,
    },
}

/// Référence d'environnement qualifiée par le resolver serveur ; aucun secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalEnvironmentRef {
    /// Identité du profil effectif.
    pub profile_id: String,
    /// Version exacte du profil.
    pub version: u64,
    /// Objet publié représentant ce profil, sans valeur secrète.
    pub digest: Sha256Digest,
}

/// Spécification normalisée de ce premier sous-chemin, sans worktree implicite.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedTerminalProcessSpec {
    /// Programme absolu résolu ; aucun shell implicite.
    pub program: String,
    /// Arguments exacts.
    pub arguments: Vec<String>,
    /// Chemin relatif demandé dans la racine de projet.
    pub requested_cwd: String,
    /// Chemin canonique qualifié par le resolver.
    pub cwd_canonical: String,
    /// Profil d'environnement exact, jamais un environnement vide déduit.
    pub environment: TerminalEnvironmentRef,
    /// Lignes PTY.
    pub rows: u16,
    /// Colonnes PTY.
    pub cols: u16,
    /// Readiness `process_started`, délai borné par le serveur.
    pub readiness_timeout_ms: u64,
}

/// Pools auxiliaires préalablement configurés ; aucun défaut chiffré implicite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalAuxiliaryPools {
    /// Pool `process_host` global, unité `slot`.
    pub host: PoolId,
    /// Pool `process` global, unité `slot`.
    pub child: PoolId,
    /// Pool `pty` global, unité `slot`.
    pub pty: PoolId,
}

/// Paramètres résolus par un service authentifié, distincts du payload client.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalAllocationPreparation {
    /// Mission réelle.
    pub mission_id: MissionId,
    /// Révision de mission à relire.
    pub mission_revision: Revision,
    /// Portée configurée réelle.
    pub scope: TerminalAllocationScope,
    /// Propriété exclusive dans la mission.
    pub purpose_key: String,
    /// Commande et environnement qualifiés par le resolver.
    pub process_spec: NormalizedTerminalProcessSpec,
    /// Réserves auxiliaires séparées du pool d'exécution.
    pub auxiliary: TerminalAuxiliaryPools,
    /// TTL effectif fourni par la configuration serveur.
    pub lease_ttl_ms: i64,
}

impl std::fmt::Debug for TerminalAllocationPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalAllocationPreparation")
            .field("mission_id", &self.mission_id)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}
impl TerminalAllocationPreparation {
    /// Payload demandé correspondant à cette résolution (premier sous-chemin).
    #[must_use]
    pub fn command_payload(&self) -> Value {
        let TerminalAllocationScope::ProjectConfiguration {
            project_id,
            revision,
        } = self.scope;
        json!({"mission_id":self.mission_id,"invocation_id":null,"purpose_key":self.purpose_key,"reuse_policy":"reuse_compatible","scope_id":project_id.to_string(),"scope_revision":revision,"worktree_id":null,"cwd":self.process_spec.requested_cwd,"program":self.process_spec.program,"arguments":self.process_spec.arguments,"environment_profile_id":self.process_spec.environment.profile_id,"pty":{"rows":self.process_spec.rows,"cols":self.process_spec.cols},"readiness":{"kind":"process_started","timeout_ms":self.process_spec.readiness_timeout_ms}})
    }
}

/// Identités et autorité figées ; les IDs OS restent absents.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalAllocationContext {
    /// Version du document.
    pub schema_version: u32,
    /// Terminal logique stable.
    pub terminal_id: TerminalId,
    /// Processus logique stable.
    pub process_id: ProcessId,
    /// Génération physique planifiée, sans preuve de spawn.
    pub process_run_uuid: ProcessRunUuid,
    /// Host planifié, sans preuve de présence.
    pub host_id: HostId,
    /// Action allouée, créée seulement à Authorizing.
    pub action_id: ActionId,
    /// Reçu causal authentique.
    pub receipt_id: ReceiptId,
    /// Acteur authentifié.
    pub actor_id: ActorId,
    /// Principal résolu depuis cet acteur.
    pub principal_id: ActorId,
    /// Lignée de préparation.
    pub authority_generation: AuthorityGeneration,
    /// Incarnation de préparation.
    pub daemon_epoch: DaemonEpoch,
    /// Boot du daemon, jamais une identité de processus observée.
    pub daemon_boot_identity: String,
    /// Bail de ressource distinct du fence broker.
    pub lease_id: LeaseId,
    /// Fence de ressource.
    pub resource_fence: Fence,
    /// Expiration initiale du bail.
    pub lease_expires_at: TimestampMs,
    /// Réservation host.
    pub host_reservation_id: ReservationId,
    /// Réservation enfant.
    pub child_reservation_id: ReservationId,
    /// Réservation PTY.
    pub pty_reservation_id: ReservationId,
    /// Résolution immuable.
    pub preparation: TerminalAllocationPreparation,
    /// Révision de politique de sécurité exacte.
    pub policy_revision: Revision,
    /// Snapshot de sécurité effectivement lu.
    pub security_policy_digest: Sha256Digest,
}

/// État de la préparation, distinct de l'état OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalAllocationState {
    /// IDs et ressources réservés ; aucun lancement.
    Allocated,
    /// Marqueur consommé une fois ; inspection seulement en cas d'ACK perdu.
    LaunchResolved,
    /// Retrait sous observation qualifiée par un futur port interne.
    Retired,
}

/// Lecture interne et autorité figée.
#[derive(Clone, PartialEq, Eq)]
pub struct TerminalAllocationRecord {
    /// Document exact.
    pub context: TerminalAllocationContext,
    /// Digest canonique également utilisé comme payload broker.
    pub context_digest: Sha256Digest,
    /// État de préparation.
    pub state: TerminalAllocationState,
    /// Révision CAS.
    pub revision: Revision,
    /// Marqueur consommé éventuel.
    pub dispatch_id: Option<DispatchId>,
    /// État broker observé dans le même snapshot.
    pub action_state: Option<ActionState>,
    /// Observation liée au retrait, conservée historiquement.
    pub retirement_evidence_id: Option<String>,
}
impl std::fmt::Debug for TerminalAllocationRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalAllocationRecord")
            .field("terminal_id", &self.context.terminal_id)
            .field("context_digest", &self.context_digest)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
impl TerminalAllocationRecord {
    /// Empreinte exacte autorisée.
    #[must_use]
    pub const fn payload_digest(&self) -> Sha256Digest {
        self.context_digest
    }
    /// Référence native du terminal logique.
    #[must_use]
    pub fn resource_refs(&self) -> Value {
        json!([{"kind":"terminal","value":self.context.terminal_id}])
    }
}

/// Résolution interne avant effet ; le client ne produit pas cette autorité.
#[derive(Debug, Clone)]
pub struct ResolveTerminalLaunchRequest {
    /// Terminal exact.
    pub terminal_id: TerminalId,
    /// Génération exacte.
    pub process_run_uuid: ProcessRunUuid,
    /// Action exacte.
    pub action_id: ActionId,
    /// Dispatch exact.
    pub dispatch_id: DispatchId,
    /// Autorité active, vérifiée avant lecture.
    pub authority_generation: AuthorityGeneration,
    /// Révision CAS.
    pub expected_revision: Revision,
    /// Document exact.
    pub context_digest: Sha256Digest,
    /// Payload exact.
    pub payload_digest: Sha256Digest,
    /// Autorité authentifiée du service.
    pub caller: CallerAuthority,
    /// Bail exact porté par la préparation.
    pub lease: FenceAssertion,
}

/// Résolution unique remise au daemon ; aucune opération OS dans le Store.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedTerminalLaunch {
    /// Préparation exacte consommée.
    pub allocation: TerminalAllocationRecord,
}
impl std::fmt::Debug for ResolvedTerminalLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedTerminalLaunch")
            .field("allocation", &self.allocation)
            .finish()
    }
}

use crate::commands::{ensure_scope_not_frozen_tx, read_daemon_state_conn};
use crate::leases::{LeaseLineageCtx, tx_acquire_lease, tx_validate_lease_fence};
use crate::policies_secrets::{
    SubjectKind, ensure_subject_not_blocked_tx, read_policy_summary_conn, require_security_admin,
};
use crate::{
    AggregateKind, BrokerCause, BrokerIntent, CapacityReservationRequest, CommandMutation,
    CommitCommandRequest, DaemonStateRow, EventAppendSpec, StoreConfig,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use unleash_protocol::{EventCausationKind, Sensitivity, canonical_sha256};
fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}
fn stale(message: &'static str) -> StoreError {
    StoreError::StaleFence {
        message: message.to_owned(),
    }
}
fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("terminal integer out of range"))
}
fn key(prep: &TerminalAllocationPreparation) -> Result<String, StoreError> {
    Ok(format!(
        "terminal-purpose:{}",
        canonical_sha256(&json!({"mission_id":prep.mission_id,"purpose_key":prep.purpose_key}))?
    ))
}
fn valid_ref(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

pub(crate) fn validate_command(
    tx: &Transaction<'_>,
    req: &CommitCommandRequest,
) -> Result<(), StoreError> {
    let CommandMutation::PrepareTerminalAllocation {
        terminal_id,
        caller,
        preparation,
    } = &req.mutation
    else {
        if req.envelope.command_type == "AcquireTerminal" {
            return Err(invalid("terminal allocation mutation missing"));
        }
        return Ok(());
    };
    require_security_admin(caller, "AcquireTerminal")?;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = preparation.scope;
    if caller.actor_id != req.actor_id
        || req
            .envelope
            .actor_ref
            .as_ref()
            .is_some_and(|a| a != req.actor_id.as_str())
        || req.envelope.command_type != "AcquireTerminal"
        || req.target_aggregate != AggregateKind::TerminalAllocation
        || req.envelope.aggregate_id != terminal_id.as_uuid()
        || terminal_id.as_uuid().is_nil()
        || req.envelope.expected_revision != Revision::INITIAL
        || req.envelope.project_id != Some(project_id)
        || !req.events.is_empty()
        || !req.capacity_reservations.is_empty()
        || !req.budget_reservations.is_empty()
        || req.fence_assertion.is_some()
        || serde_json::to_value(&req.envelope.payload)? != preparation.command_payload()
    {
        return Err(invalid(
            "terminal allocation envelope differs from resolved request",
        ));
    }
    if read_daemon_state_conn(tx)?.schema_version < 10 {
        return Err(invalid("terminal allocation requires V010"));
    }
    let digest = canonical_sha256(preparation)?.to_string();
    let prior:Option<(String,String)>=tx.query_row("SELECT t.terminal_id,t.preparation_digest FROM terminal_allocations t JOIN command_receipts c ON c.receipt_id=t.receipt_id WHERE c.actor_id=?1 AND c.command_id=?2",params![req.actor_id.as_str(),req.envelope.command_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((id, old)) = prior
        && (id != terminal_id.to_string() || old != digest)
    {
        return Err(StoreError::IdempotencyConflict {
            command_id: req.envelope.command_id,
        });
    }
    Ok(())
}
fn policy(
    tx: &Transaction<'_>,
    project: ProjectId,
) -> Result<(Revision, Sha256Digest), StoreError> {
    let s = read_policy_summary_conn(tx, Some(project))?;
    let rev = s
        .global_policy
        .as_ref()
        .map_or(Revision::INITIAL, |p| p.revision)
        .max(
            s.project_policy
                .as_ref()
                .map_or(Revision::INITIAL, |p| p.revision),
        );
    Ok((
        rev,
        canonical_sha256(
            &json!({"schema_version":1,"global_policy":s.global_policy,"project_policy":s.project_policy,"guard_config":s.guard_config}),
        )?,
    ))
}
fn validate_preparation(
    tx: &Transaction<'_>,
    prep: &TerminalAllocationPreparation,
    actor: &ActorId,
    principal: &ActorId,
) -> Result<(), StoreError> {
    use std::path::{Component, Path};
    let TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = prep.scope;
    let (root,current,owner,mission_revision,state):(Option<String>,i64,String,i64,String)=tx.query_row("SELECT p.primary_root_path,p.revision,m.owner_user_id,m.revision,m.state FROM missions m JOIN projects p ON p.project_id=m.project_id WHERE m.mission_id=?1 AND m.project_id=?2 AND p.state='active' AND m.owner_kind='user'",params![prep.mission_id.to_string(),project_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?.ok_or_else(||invalid("terminal mission/project scope absent"))?;
    if current != integer(revision.as_u64())?
        || mission_revision != integer(prep.mission_revision.as_u64())?
        || owner != principal.as_str()
        || !["ready", "active"].contains(&state.as_str())
    {
        return Err(invalid(
            "terminal mission, owner or project revision changed",
        ));
    }
    ensure_scope_not_frozen_tx(tx, Some(project_id), None, None)?;
    for id in [principal.as_str(), actor.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, id, Some(project_id)).map_err(
            |e| match e {
                StoreError::CapabilityDenied { reason, .. } => StoreError::CapabilityDenied {
                    reason,
                    message: "terminal subject suspended or revoked".to_owned(),
                },
                other => other,
            },
        )?;
    }
    let s = &prep.process_spec;
    let root = root.ok_or_else(|| invalid("terminal primary project root absent"))?;
    let relative = Path::new(&s.requested_cwd);
    if !Path::new(&root).is_absolute()
        || !Path::new(&s.program).is_absolute()
        || !Path::new(&s.cwd_canonical).is_absolute()
        || relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        || Path::new(&root).join(relative) != Path::new(&s.cwd_canonical)
        || [s.program.as_str(), s.cwd_canonical.as_str()]
            .iter()
            .any(|s| s.contains('\0'))
        || s.arguments.iter().any(|a| a.contains('\0'))
        || !(1..=500).contains(&s.rows)
        || !(1..=1000).contains(&s.cols)
        || s.readiness_timeout_ms == 0
        || i64::try_from(s.readiness_timeout_ms).is_err()
        || prep.lease_ttl_ms <= 0
        || i64::try_from(s.readiness_timeout_ms).is_ok_and(|ms| ms > prep.lease_ttl_ms)
        || !valid_ref(&prep.purpose_key)
        || !valid_ref(&s.environment.profile_id)
        || s.environment.version == 0
    {
        return Err(invalid("terminal normalized process specification invalid"));
    }
    let published: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_records WHERE object_hash=?1 AND state='live')",
        [s.environment.digest.to_string()],
        |r| r.get(0),
    )?;
    if !published {
        return Err(invalid(
            "resolved terminal environment object is not published",
        ));
    }
    Ok(())
}
fn principal(tx: &Connection, actor: &ActorId) -> Result<ActorId, StoreError> {
    let id:Option<String>=tx.query_row("SELECT principal_id FROM principals WHERE actor_id=?1 AND principal_kind='user' AND state='active'",[actor.as_str()],|r|r.get(0)).optional()?;
    Ok(ActorId::new(id.ok_or_else(|| {
        invalid("active human terminal principal absent")
    })?)?)
}
fn pool_bindings(p: &TerminalAuxiliaryPools) -> [(PoolId, &'static str); 3] {
    [
        (p.host, "process_host"),
        (p.child, "process"),
        (p.pty, "pty"),
    ]
}
fn validate_pools(tx: &Transaction<'_>, p: &TerminalAuxiliaryPools) -> Result<(), StoreError> {
    if p.host == p.child || p.host == p.pty || p.child == p.pty {
        return Err(invalid("terminal auxiliary roles must use distinct pools"));
    }
    for (id, kind) in pool_bindings(p) {
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM capacity_pools WHERE pool_id=?1 AND pool_kind=?2 AND scope_key='global' AND unit='slot' AND admissions_frozen=0 AND effective_limit<=hard_limit AND (SELECT COALESCE(SUM(r.amount),0) FROM capacity_reservations r WHERE r.pool_id=capacity_pools.pool_id AND r.state IN ('held','reconciling'))<=effective_limit)",params![id.to_string(),kind],|r|r.get(0))?;
        if !valid {
            return Err(invalid(
                "terminal auxiliary pool not configured with the expected role",
            ));
        }
    }
    Ok(())
}

fn available_property_key(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    terminal: TerminalId,
    prep: &TerminalAllocationPreparation,
) -> Result<String, StoreError> {
    let taken:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_allocations WHERE terminal_id=?1 OR (mission_id=?2 AND purpose_key=?3 AND state IN ('allocated','launch_resolved'))) OR EXISTS(SELECT 1 FROM terminal_sessions s WHERE (s.terminal_id=?1 OR (s.mission_id=?2 AND s.purpose_key=?3)) AND NOT EXISTS(SELECT 1 FROM terminal_allocations old JOIN processes p ON p.process_id=s.process_id JOIN actions a ON a.action_id=old.action_id WHERE old.terminal_id=s.terminal_id AND old.state='retired' AND p.process_id=old.process_id AND p.process_run_uuid=old.process_run_uuid AND p.action_id=old.action_id AND p.authority_generation=a.authority_generation AND p.authority_generation=json_extract(old.context_json,'$.authority_generation') AND a.daemon_epoch=json_extract(old.context_json,'$.daemon_epoch') AND a.fence=0 AND a.origin_kind='control_plane' AND a.control_principal_id=old.principal_id AND s.mission_id=old.mission_id AND s.purpose_key=old.purpose_key AND s.lease_id=old.lease_id AND s.owner_agent_id IS NULL AND s.state='exited' AND p.state='exited')) OR EXISTS(SELECT 1 FROM command_receipts WHERE aggregate_id=?1)",params![terminal.to_string(),prep.mission_id.to_string(),prep.purpose_key],|r|r.get(0))?;
    if taken {
        return Err(StoreError::CapacityBlocked(
            "terminal property already reserved; reconciliation or registered acquisition required"
                .to_owned(),
        ));
    }
    let resource_key = key(prep)?;
    let last:Option<i64>=tx.query_row("SELECT last_fence FROM resource_fences WHERE authority_generation=?1 AND resource_kind='terminal' AND resource_key=?2",params![daemon.authority_generation.to_string(),resource_key],|r|r.get(0)).optional()?;
    if last == Some(i64::MAX) {
        return Err(stale("terminal resource fence exhausted"));
    }
    Ok(resource_key)
}
fn ensure_no_registered_resource(
    tx: &Transaction<'_>,
    c: &TerminalAllocationContext,
) -> Result<(), StoreError> {
    let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_sessions s WHERE (s.terminal_id=?1 OR (s.mission_id=?2 AND s.purpose_key=?3)) AND NOT EXISTS(SELECT 1 FROM terminal_allocations old JOIN processes p ON p.process_id=s.process_id JOIN actions a ON a.action_id=old.action_id WHERE old.terminal_id=s.terminal_id AND old.state='retired' AND p.process_id=old.process_id AND p.process_run_uuid=old.process_run_uuid AND p.action_id=old.action_id AND p.authority_generation=a.authority_generation AND p.authority_generation=json_extract(old.context_json,'$.authority_generation') AND a.daemon_epoch=json_extract(old.context_json,'$.daemon_epoch') AND a.fence=0 AND a.origin_kind='control_plane' AND a.control_principal_id=old.principal_id AND s.mission_id=old.mission_id AND s.purpose_key=old.purpose_key AND s.lease_id=old.lease_id AND s.owner_agent_id IS NULL AND s.state='exited' AND p.state='exited')) OR EXISTS(SELECT 1 FROM processes WHERE process_id=?4 OR process_run_uuid=?5)",params![c.terminal_id.to_string(),c.preparation.mission_id.to_string(),c.preparation.purpose_key,c.process_id.to_string(),c.process_run_uuid.to_string()],|r|r.get(0))?;
    if exists {
        return Err(invalid(
            "registered terminal/process requires reconciliation before launch",
        ));
    }
    Ok(())
}
fn reserve_terminal_capacity_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    terminal: TerminalId,
    process_id: ProcessId,
    pools: &TerminalAuxiliaryPools,
) -> Result<[ReservationId; 3], StoreError> {
    let ids = [
        ReservationId::from_uuid(config.id_generator.new_uuid()),
        ReservationId::from_uuid(config.id_generator.new_uuid()),
        ReservationId::from_uuid(config.id_generator.new_uuid()),
    ];
    for ((pool, kind), id) in pool_bindings(pools).into_iter().zip(ids) {
        let (owner_type, owner_id) = if kind == "pty" {
            ("terminal", terminal.to_string())
        } else {
            ("process", process_id.to_string())
        };
        crate::budgets::tx_reserve_capacity(
            tx,
            daemon.authority_generation,
            daemon.daemon_epoch,
            now,
            &CapacityReservationRequest {
                reservation_id: id,
                pool_id: pool,
                execution_owner: None,
                owner_type: owner_type.to_owned(),
                owner_id,
                admission_class: None,
                amount: 1,
            },
        )?;
    }
    Ok(ids)
}
#[expect(
    clippy::too_many_arguments,
    reason = "Explicit identities and authenticated scope of one atomic preparation"
)]
pub(crate) fn prepare_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    terminal: TerminalId,
    receipt: ReceiptId,
    actor: &ActorId,
    project: Option<ProjectId>,
    prep: &TerminalAllocationPreparation,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.mode != unleash_domain::DaemonMode::Ready
        || config
            .clock_uncertain
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid(
            "terminal preparation requires ready daemon and certain clock",
        ));
    }
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = prep.scope;
    if project != Some(project_id) {
        return Err(invalid("terminal project differs from mission scope"));
    }
    let principal = principal(tx, actor)?;
    validate_preparation(tx, prep, actor, &principal)?;
    validate_pools(tx, &prep.auxiliary)?;
    let resource_key = available_property_key(tx, daemon, terminal, prep)?;
    now.as_i64()
        .checked_add(prep.lease_ttl_ms)
        .ok_or_else(|| invalid("terminal lease deadline overflow"))?;
    let action_id = ActionId::from_uuid(config.id_generator.new_uuid());
    let process_id = ProcessId::from_uuid(config.id_generator.new_uuid());
    let process_run_uuid = ProcessRunUuid::from_uuid(config.id_generator.new_uuid());
    let host_id = HostId::from_uuid(config.id_generator.new_uuid());
    let lease_id = LeaseId::from_uuid(config.id_generator.new_uuid());
    let lease = tx_acquire_lease(
        tx,
        &LeaseLineageCtx {
            authority: daemon.authority_generation,
            daemon_epoch: daemon.daemon_epoch,
            boot_identity: &daemon.boot_identity,
            now_ms: now,
        },
        lease_id,
        &crate::AcquireLeaseRequest {
            resource_kind: "terminal".to_owned(),
            resource_key,
            owner_type: "principal".to_owned(),
            owner_id: principal.to_string(),
            ttl_ms: prep.lease_ttl_ms,
        },
    )?;
    let [
        host_reservation_id,
        child_reservation_id,
        pty_reservation_id,
    ] = reserve_terminal_capacity_tx(
        tx,
        config,
        daemon,
        now,
        terminal,
        process_id,
        &prep.auxiliary,
    )?;
    let (policy_revision, security_policy_digest) = policy(tx, project_id)?;
    let context = TerminalAllocationContext {
        schema_version: 1,
        terminal_id: terminal,
        process_id,
        process_run_uuid,
        host_id,
        action_id,
        receipt_id: receipt,
        actor_id: actor.clone(),
        principal_id: principal,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        daemon_boot_identity: daemon.boot_identity.clone(),
        lease_id,
        resource_fence: lease.fence,
        lease_expires_at: lease.expires_at_ms,
        host_reservation_id,
        child_reservation_id,
        pty_reservation_id,
        preparation: prep.clone(),
        policy_revision,
        security_policy_digest,
    };
    let digest = canonical_sha256(&context)?;
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    tx.execute("INSERT INTO terminal_allocations(terminal_id,receipt_id,action_id,process_id,process_run_uuid,principal_id,mission_id,project_id,purpose_key,lease_id,preparation_digest,context_digest,context_json,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'allocated',1)",params![terminal.to_string(),receipt.to_string(),action_id.to_string(),process_id.to_string(),process_run_uuid.to_string(),context.principal_id.as_str(),prep.mission_id.to_string(),project_id.to_string(),prep.purpose_key,lease_id.to_string(),canonical_sha256(prep)?.to_string(),digest.to_string(),serde_json::to_string(&context)?])?;
    events.push(EventAppendSpec{event_type:"TerminalAcquired".to_owned(),aggregate_type:AggregateKind::TerminalAllocation,aggregate_id:terminal.as_uuid(),aggregate_revision:Revision::new(1),project_id:Some(project_id),correlation_id:terminal.as_uuid(),causation_kind:EventCausationKind::Command,causation_id:None,payload:json!({"terminal_id":terminal,"process_id":process_id,"process_run_uuid":process_run_uuid,"action_id":action_id,"lease_id":lease_id,"resource_fence":context.resource_fence,"context_digest":digest,"state":"preparing","reused":false,"discovery":{"candidates":[],"decision":"new","reason":"no_registered_resource"}}).as_object().cloned().ok_or_else(||invalid("invalid terminal event"))?,sensitivity:Sensitivity::Project,outbox_destinations:vec!["clients".to_owned(),"harness".to_owned()]});
    Ok(Revision::new(1))
}

struct AllocationSqlRow {
    json: String,
    digest: String,
    state: String,
    revision: i64,
    dispatch: Option<String>,
    action_state: Option<String>,
    retirement_evidence_id: Option<String>,
}

pub(crate) fn read_conn(
    conn: &Connection,
    terminal: TerminalId,
) -> Result<Option<TerminalAllocationRecord>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 10 {
        return Ok(None);
    }
    let row:Option<AllocationSqlRow>=conn.query_row("SELECT t.context_json,t.context_digest,t.state,t.revision,t.dispatch_id,a.state,t.retirement_evidence_id FROM terminal_allocations t LEFT JOIN actions a ON a.action_id=t.action_id WHERE t.terminal_id=?1",[terminal.to_string()],|r|Ok(AllocationSqlRow{json:r.get(0)?,digest:r.get(1)?,state:r.get(2)?,revision:r.get(3)?,dispatch:r.get(4)?,action_state:r.get(5)?,retirement_evidence_id:r.get(6)?})).optional()?;
    row.map(|row| {
        let context: TerminalAllocationContext = serde_json::from_str(&row.json)?;
        let context_digest: Sha256Digest = row.digest.parse()?;
        if context.schema_version != 1
            || context.terminal_id != terminal
            || canonical_sha256(&context)? != context_digest
        {
            return Err(invalid("terminal context integrity mismatch"));
        }
        Ok(TerminalAllocationRecord {
            retirement_evidence_id: row.retirement_evidence_id,
            context,
            context_digest,
            state: match row.state.as_str() {
                "allocated" => TerminalAllocationState::Allocated,
                "launch_resolved" => TerminalAllocationState::LaunchResolved,
                "retired" => TerminalAllocationState::Retired,
                _ => return Err(invalid("terminal allocation state invalid")),
            },
            revision: Revision::new(
                u64::try_from(row.revision).map_err(|_| invalid("terminal revision invalid"))?,
            ),
            dispatch_id: row.dispatch.map(|s| s.parse()).transpose()?,
            action_state: row
                .action_state
                .as_deref()
                .map(crate::recovery_ops::parse_action_state)
                .transpose()?,
        })
    })
    .transpose()
}
fn for_action(
    conn: &Connection,
    action: ActionId,
) -> Result<Option<TerminalAllocationRecord>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 10 {
        return Ok(None);
    }
    let terminal: Option<String> = conn
        .query_row(
            "SELECT terminal_id FROM terminal_allocations WHERE action_id=?1",
            [action.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    terminal
        .map(|s| read_conn(conn, s.parse()?))
        .transpose()
        .map(Option::flatten)
}
pub(crate) fn validate_incarnation(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    action: ActionId,
) -> Result<(), StoreError> {
    if let Some(record) = for_action(tx, action)? {
        let c = &record.context;
        if c.authority_generation != d.authority_generation {
            return Err(StoreError::StaleAuthorityGeneration {
                expected: d.authority_generation,
                actual: c.authority_generation,
                command_id: None,
            });
        }
        if c.daemon_epoch != d.daemon_epoch || c.daemon_boot_identity != d.boot_identity {
            return Err(stale("terminal preparation belongs to an old incarnation"));
        }
    }
    Ok(())
}
fn validate_lease(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalAllocationContext,
) -> Result<(), StoreError> {
    let row:Option<(String,String,String,String,String,i64,i64,String,i64)>=tx.query_row("SELECT authority_generation,resource_kind,resource_key,owner_type,owner_id,daemon_epoch,fence,boot_identity,expires_at_ms FROM resource_leases WHERE lease_id=?1 AND state='held'",[c.lease_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?;
    let Some((auth, kind, resource, owner_type, owner, epoch, fence, boot, expiry)) = row else {
        return Err(stale("terminal resource lease is not held"));
    };
    if auth != d.authority_generation.to_string()
        || kind != "terminal"
        || resource != key(&c.preparation)?
        || owner_type != "principal"
        || owner != c.principal_id.as_str()
        || epoch != integer(d.daemon_epoch.as_u64())?
        || fence != integer(c.resource_fence.as_u64())?
        || boot != d.boot_identity
        || expiry <= now.as_i64()
        || c.lease_expires_at.as_i64() <= now.as_i64()
    {
        return Err(stale(
            "terminal resource lease target, incarnation, fence or TTL differs",
        ));
    }
    tx_validate_lease_fence(
        tx,
        d.authority_generation,
        d.daemon_epoch,
        &FenceAssertion {
            lease_id: c.lease_id,
            authority_generation: d.authority_generation,
            daemon_epoch: d.daemon_epoch,
            fence: c.resource_fence,
            owner_id: c.principal_id.to_string(),
        },
    )
}
fn validate_holds(tx: &Transaction<'_>, c: &TerminalAllocationContext) -> Result<(), StoreError> {
    validate_pools(tx, &c.preparation.auxiliary)?;
    for ((pool, kind), id) in pool_bindings(&c.preparation.auxiliary).into_iter().zip([
        c.host_reservation_id,
        c.child_reservation_id,
        c.pty_reservation_id,
    ]) {
        let (owner_type, owner) = if kind == "pty" {
            ("terminal", c.terminal_id.to_string())
        } else {
            ("process", c.process_id.to_string())
        };
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM capacity_reservations WHERE reservation_id=?1 AND pool_id=?2 AND owner_type=?3 AND owner_id=?4 AND amount=1 AND admission_class IS NULL AND state='held' AND authority_generation=?5 AND daemon_epoch=?6)",params![id.to_string(),pool.to_string(),owner_type,owner,c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?],|r|r.get(0))?;
        if !valid {
            return Err(invalid("terminal auxiliary hold differs from preparation"));
        }
    }
    Ok(())
}
fn validate_live(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    r: &TerminalAllocationRecord,
) -> Result<(), StoreError> {
    let c = &r.context;
    validate_incarnation(tx, d, c.action_id)?;
    if r.state == TerminalAllocationState::Retired {
        return Err(invalid(
            "terminal allocation has a qualified disposition; old generation cannot relaunch",
        ));
    }
    ensure_no_registered_resource(tx, c)?;
    if d.mode != unleash_domain::DaemonMode::Ready {
        return Err(invalid("terminal launch requires ready daemon"));
    }
    if principal(tx, &c.actor_id)? != c.principal_id {
        return Err(invalid("terminal principal changed"));
    }
    validate_preparation(tx, &c.preparation, &c.actor_id, &c.principal_id)?;
    validate_lease(tx, d, now, c)?;
    validate_holds(tx, c)?;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = c.preparation.scope;
    if policy(tx, project_id)? != (c.policy_revision, c.security_policy_digest) {
        return Err(invalid("terminal security policy changed"));
    }
    let receipt:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND actor_id=?2 AND aggregate_id=?3 AND command_type='AcquireTerminal' AND authority_generation=?4 AND state='accepted' AND response_state='full')",params![c.receipt_id.to_string(),c.actor_id.as_str(),c.terminal_id.to_string(),d.authority_generation.to_string()],|r|r.get(0))?;
    if !receipt {
        return Err(invalid(
            "terminal receipt is not attributable to the preparation",
        ));
    }
    Ok(())
}
pub(crate) fn validate_intent(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    intent: &BrokerIntent,
) -> Result<(), StoreError> {
    let record = for_action(tx, intent.action_id)?;
    let receipt_type: Option<String> =
        if let BrokerCause::CommandReceipt { receipt_id } = intent.cause {
            tx.query_row(
                "SELECT command_type FROM command_receipts WHERE receipt_id=?1",
                [receipt_id.to_string()],
                |r| r.get(0),
            )
            .optional()?
        } else {
            None
        };
    if record.is_none()
        && receipt_type.as_deref() != Some("AcquireTerminal")
        && intent.manifest.get("verb") != Some(&json!(TERMINAL_ALLOCATION_VERB))
    {
        return Ok(());
    }
    let r = record.ok_or_else(|| invalid("terminal allocation preparation missing"))?;
    let c = &r.context;
    validate_live(tx, d, now, &r)?;
    let TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = c.preparation.scope;
    if intent.principal_id != c.principal_id
        || intent.cause
            != (BrokerCause::CommandReceipt {
                receipt_id: c.receipt_id,
            })
        || intent.project_id != Some(project_id)
        || intent.scope_revision != revision
        || intent.policy_revision != c.policy_revision
        || intent.payload_digest != r.payload_digest()
        || intent.action_deadline != c.lease_expires_at
        || intent.manifest.get("verb") != Some(&json!(TERMINAL_ALLOCATION_VERB))
        || intent.manifest.get("effects_class") != Some(&json!("process_exec"))
        || intent.manifest.get("resource_refs") != Some(&r.resource_refs())
        || intent.manifest.get("secret_refs") != Some(&json!([]))
        || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
    {
        return Err(invalid(
            "terminal intent differs from authenticated allocation",
        ));
    }
    Ok(())
}
pub(crate) fn guard_observation(
    tx: &Transaction<'_>,
    phase: &crate::BrokerPhase,
) -> Result<(), StoreError> {
    if let crate::BrokerPhase::Observation {
        action_id, receipt, ..
    } = phase
        && for_action(tx, *action_id)?.is_some()
        && receipt.state != ActionState::OutcomeUnknown
    {
        return Err(invalid("terminal OS observation producer not qualified"));
    }
    Ok(())
}
pub(crate) fn resolve_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &ResolveTerminalLaunchRequest,
) -> Result<ResolvedTerminalLaunch, StoreError> {
    let d = read_daemon_state_conn(tx)?;
    if request.authority_generation != d.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: d.authority_generation,
            actual: request.authority_generation,
            command_id: None,
        });
    }
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid("terminal lease clock uncertain"));
    }
    require_security_admin(&request.caller, "AcquireTerminal")?;
    let record = read_conn(tx, request.terminal_id)?
        .ok_or_else(|| invalid("terminal preparation absent"))?;
    let c = &record.context;
    if record.state != TerminalAllocationState::Allocated || record.dispatch_id.is_some() {
        return Err(invalid(
            "terminal launch already resolved; inspection and reconciliation required",
        ));
    }
    if record.revision != request.expected_revision {
        return Err(StoreError::RevisionConflict {
            current_revision: record.revision,
            expected_revision: request.expected_revision,
            command_id: None,
        });
    }
    if request.caller.actor_id != c.actor_id
        || request.action_id != c.action_id
        || request.process_run_uuid != c.process_run_uuid
        || request.context_digest != record.context_digest
        || request.payload_digest != record.payload_digest()
    {
        return Err(invalid("terminal resolution differs from preparation"));
    }
    validate_live(tx, &d, now, &record)?;
    if request.lease.lease_id != c.lease_id
        || request.lease.owner_id != c.principal_id.as_str()
        || request.lease.authority_generation != c.authority_generation
        || request.lease.daemon_epoch != c.daemon_epoch
        || request.lease.fence != c.resource_fence
    {
        return Err(stale("terminal resolution resource assertion differs"));
    }
    crate::broker_writer::validate_execution_dispatch_tx(
        tx,
        &d,
        now,
        c.action_id,
        request.dispatch_id,
    )?;
    let changed=tx.execute("UPDATE terminal_allocations SET state='launch_resolved',revision=revision+1,dispatch_id=?2,resolved_at_ms=?3 WHERE terminal_id=?1 AND state='allocated' AND revision=?4 AND dispatch_id IS NULL",params![c.terminal_id.to_string(),request.dispatch_id.to_string(),now.as_i64(),integer(request.expected_revision.as_u64())?])?;
    if changed != 1 {
        return Err(invalid("terminal allocation resolution race"));
    }
    append_resolution_event(tx, config, &d, now, c, request.dispatch_id)?;
    config
        .cutpoints
        .hit(crate::StoreCutpoint::C1BeforeCommandCommit)?;
    Ok(ResolvedTerminalLaunch {
        allocation: read_conn(tx, c.terminal_id)?
            .ok_or_else(|| invalid("terminal preparation disappeared"))?,
    })
}
fn append_resolution_event(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalAllocationContext,
    dispatch: DispatchId,
) -> Result<(), StoreError> {
    let event = config.id_generator.new_uuid().to_string();
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = c.preparation.scope;
    let payload = json!({"terminal_id":c.terminal_id,"process_id":c.process_id,"process_run_uuid":c.process_run_uuid,"action_id":c.action_id,"dispatch_id":dispatch,"lease_id":c.lease_id,"resource_fence":c.resource_fence,"state":"preparing","os_observation":null});
    tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES(?1,?2,'terminal_allocation',?3,2,0,'ProcessLaunchPrepared',1,?4,?5,?5,?6,?6,?7,'full',?8,'restricted','command',?9,?9,?3)",params![event,d.stream_id.to_string(),c.terminal_id.to_string(),project_id.to_string(),c.actor_id.as_str(),now.as_i64(),serde_json::to_string(&payload)?,canonical_sha256(&payload)?.to_string(),c.receipt_id.to_string()])?;
    tx.execute("INSERT INTO outbox(event_id,destination,state,delivery_attempt,next_attempt_at_ms,authority_generation,consumer_version,delivery_key,rearm_count) VALUES(?1,'clients','pending',0,?2,?3,?4,?5,0)",params![event,now.as_i64(),d.authority_generation.to_string(),config.consumer_version,format!("{event}:clients")])?;
    Ok(())
}
