//! Mandats F012 émis par `SetPolicy`, sans préparation ni effet fournisseur.
//! Ce module ne reçoit que la transaction writer et des faits authentifiés.
//! Aucun fichier objet, valeur de coffre ou transport n'est consulté ici.

use crate::commands::{DaemonStateRow, ensure_scope_not_frozen_tx, read_daemon_state_conn};
use crate::policies_secrets::{
    CallerAuthority, PolicyScopeKind, SecurityPolicyRecord, SubjectKind,
    ensure_subject_not_blocked_tx, require_security_admin,
};
use crate::{StoreError, provider_profiles};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActorId, AuthorityGeneration, CommandId, DataLocality, ProjectId, ReceiptId, Revision,
    Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, ProviderTestLimits, ProviderTestPhase, TestProviderProfilePayload,
    canonical_sha256,
};
use uuid::Uuid;

const MAX_EXACT_JSON_INTEGER: i64 = 9_007_199_254_740_991;

/// Changement facultatif, fermé, du mandat sous une commande de politique.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderProbeMandateChange {
    /// Émet une permission bornée ; ce n'est ni un probe ni une qualification.
    Issue(ProviderProbeMandateIssue),
    /// Révoque une émission visible dans la même portée.
    Revoke {
        /// Identité exacte du reçu d'émission.
        mandate_ref: ReceiptId,
    },
}

impl std::fmt::Debug for ProviderProbeMandateChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Issue(_) => f.write_str("ProviderProbeMandateChange::Issue(..)"),
            Self::Revoke { mandate_ref } => f
                .debug_struct("ProviderProbeMandateChange::Revoke")
                .field("mandate_ref", mandate_ref)
                .finish(),
        }
    }
}

/// Paramètres de permission demandés par l'administrateur, sans faits de placement.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProbeMandateIssue {
    /// Principal cible existant ; il ne s'agit pas d'une identité d'agent créée.
    pub principal_id: ActorId,
    /// UUID d'un profil configuré.
    pub profile_id: Uuid,
    /// Version exacte, jamais latest.
    pub profile_version: u64,
    /// Une des trois phases actives.
    pub phase: ProviderTestPhase,
    /// Borne à intersecter avec la localité du profil et la politique effective.
    pub locality: DataLocality,
    /// Capacités permises, sans prétendre qu'elles ont été observées.
    pub capability_set: Vec<String>,
    /// Plafonds de permission ; aucun usage ni prix n'en est déduit.
    pub limits: ProviderTestLimits,
    /// Échéance absolue, revérifiée avant tout effet futur.
    pub expires_at_ms: TimestampMs,
}

impl std::fmt::Debug for ProviderProbeMandateIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProbeMandateIssue")
            .field("profile_id", &self.profile_id)
            .field("profile_version", &self.profile_version)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

/// Statut courant dérivé de la lignée, d'une révocation et de l'horloge du Store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProbeMandateStatus {
    /// Émission courante, non révoquée et non expirée.
    Active,
    /// Échéance passée ; aucune réactivation implicite.
    Expired,
    /// Révocation durable distincte de l'émission.
    Revoked,
    /// Autre lignée ; les données ne deviennent pas une nouvelle autorité.
    Historical,
}

/// Projection de permission ; ne comporte ni destination privée ni binding de coffre.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProbeMandateView {
    /// Reçu d'émission.
    pub mandate_ref: ReceiptId,
    /// Portée de la politique.
    pub scope_kind: PolicyScopeKind,
    /// Identité de portée canonique.
    pub scope_id: String,
    /// Lignée d'émission.
    pub authority_generation: AuthorityGeneration,
    /// Principal cible.
    pub principal_id: ActorId,
    /// UUID configuré.
    pub profile_id: Uuid,
    /// Version configurée exacte.
    pub profile_version: u64,
    /// Empreinte de configuration attestée à l'émission.
    pub configuration_digest: Sha256Digest,
    /// Phase permise.
    pub phase: ProviderTestPhase,
    /// Borne de localité, pas une observation de placement.
    pub locality: DataLocality,
    /// Capacités permises, pas observées.
    pub capability_set: Vec<String>,
    /// Plafonds de permission.
    pub limits: ProviderTestLimits,
    /// Échéance de l'émission.
    pub expires_at_ms: TimestampMs,
    /// Empreinte de l'émission immuable complète.
    pub mandate_digest: Sha256Digest,
    /// État courant de la permission.
    pub status: ProviderProbeMandateStatus,
    /// Reçu de révocation distinct, lorsqu'il existe.
    pub revocation_receipt_id: Option<ReceiptId>,
}

impl std::fmt::Debug for ProviderProbeMandateView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProbeMandateView")
            .field("mandate_ref", &self.mandate_ref)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    schema_version: u32,
    mandate_ref: ReceiptId,
    command_id: CommandId,
    issued_by_actor: ActorId,
    issued_at_ms: TimestampMs,
    scope_kind: PolicyScopeKind,
    scope_id: String,
    authority_generation: AuthorityGeneration,
    policy_revision: Revision,
    policy_digest: Sha256Digest,
    permission: ProviderProbeMandateIssue,
    profile_version_ref: String,
    configuration_digest: Sha256Digest,
    configuration_receipt_id: ReceiptId,
    destination: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Revocation {
    schema_version: u32,
    mandate_ref: ReceiptId,
    mandate_digest: Sha256Digest,
    #[serde(rename = "revocation_receipt_id")]
    receipt_id: ReceiptId,
    command_id: CommandId,
    revoked_by_actor: ActorId,
    revoked_at_ms: TimestampMs,
    authority_generation: AuthorityGeneration,
    scope_kind: PolicyScopeKind,
    scope_id: String,
    policy_revision: Revision,
    policy_digest: Sha256Digest,
}

struct StoredGrant {
    grant: Grant,
    digest: Sha256Digest,
    revocation: Option<Revocation>,
}

/// Faits du writer ; aucun callback ou allocation de reçu dans ce module.
pub(crate) struct PolicyMandateCommand<'a> {
    pub daemon: &'a DaemonStateRow,
    pub envelope: &'a CommandEnvelope,
    pub actor_id: &'a ActorId,
    pub caller: &'a CallerAuthority,
    pub receipt_id: ReceiptId,
    pub now: TimestampMs,
    pub scope_kind: PolicyScopeKind,
    pub scope_id: &'a str,
    pub isolation_mode: crate::PolicyIsolationMode,
    pub policy_version: &'a str,
    pub reason: &'a str,
}

/// Métadonnées fermées destinées à la notification de la même commande.
#[derive(Debug, Serialize)]
pub(crate) struct ProviderProbeMandateAudit {
    pub kind: &'static str,
    pub mandate_ref: ReceiptId,
    pub change_digest: Sha256Digest,
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn hidden() -> StoreError {
    StoreError::NotFound("Mandat d'effet fournisseur introuvable".to_owned())
}

fn project(kind: PolicyScopeKind, id: &str) -> Result<Option<ProjectId>, StoreError> {
    match kind {
        PolicyScopeKind::Global if id == "global" => Ok(None),
        PolicyScopeKind::Project => {
            let parsed = id.parse::<ProjectId>()?;
            if parsed.to_string() != id {
                return Err(invalid("portée de mandat non canonique"));
            }
            Ok(Some(parsed))
        }
        PolicyScopeKind::Global => Err(invalid("portée de mandat invalide")),
    }
}

fn validate_command(
    ctx: &PolicyMandateCommand<'_>,
    change: &ProviderProbeMandateChange,
) -> Result<Option<ProjectId>, StoreError> {
    require_security_admin(ctx.caller, "SetPolicy")?;
    let target = project(ctx.scope_kind, ctx.scope_id)?;
    if ctx.envelope.authority_generation != ctx.daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: ctx.daemon.authority_generation,
            actual: ctx.envelope.authority_generation,
            command_id: Some(ctx.envelope.command_id),
        });
    }
    let aggregate = target.map_or(ctx.daemon.authority_generation.as_uuid(), |id| id.as_uuid());
    let declared: ProviderProbeMandateChange = serde_json::from_value(
        ctx.envelope
            .payload
            .get("provider_probe_mandate")
            .cloned()
            .ok_or_else(|| invalid("mutation de mandat absente du wire"))?,
    )
    .map_err(|_| invalid("mutation de mandat wire invalide"))?;
    if ctx.actor_id != &ctx.caller.actor_id
        || ctx.envelope.command_type != "SetPolicy"
        || ctx.envelope.project_id != target
        || ctx.envelope.aggregate_id != aggregate
        || ctx
            .envelope
            .actor_ref
            .as_ref()
            .is_some_and(|actor| actor != ctx.caller.actor_id.as_str())
        || ctx.envelope.payload.get("scope_kind")
            != Some(&serde_json::Value::String(
                ctx.scope_kind.as_str().to_owned(),
            ))
        || ctx.envelope.payload.get("scope_id")
            != Some(&serde_json::Value::String(ctx.scope_id.to_owned()))
        || ctx.envelope.payload.get("isolation_mode")
            != Some(&serde_json::to_value(ctx.isolation_mode)?)
        || ctx.envelope.payload.get("policy_version")
            != Some(&serde_json::Value::String(ctx.policy_version.to_owned()))
        || ctx.envelope.payload.get("reason")
            != Some(&serde_json::Value::String(ctx.reason.to_owned()))
        || &declared != change
    {
        return Err(invalid("mutation de mandat divergente du wire authentifié"));
    }
    Ok(target)
}

fn validate_issue(
    issue: &ProviderProbeMandateIssue,
    ctx: &PolicyMandateCommand<'_>,
) -> Result<(), StoreError> {
    if issue.profile_id.is_nil()
        || !matches!(
            issue.phase,
            ProviderTestPhase::Connect
                | ProviderTestPhase::Inventory
                | ProviderTestPhase::GenerationProbe
        )
        || issue.expires_at_ms.as_i64() <= ctx.now.as_i64()
        || issue.expires_at_ms.as_i64() > MAX_EXACT_JSON_INTEGER
    {
        return Err(invalid("phase ou échéance de mandat invalide"));
    }
    TestProviderProfilePayload::try_new(
        issue.profile_id.to_string(),
        issue.profile_version,
        issue.phase,
        issue.capability_set.clone(),
        issue.limits.clone(),
        issue.locality,
        ctx.receipt_id.to_string(),
    )
    .map_err(|_| invalid("bornes de mandat invalides"))?;
    Ok(())
}

fn issue_tx(
    tx: &Transaction<'_>,
    ctx: &PolicyMandateCommand<'_>,
    permission: &ProviderProbeMandateIssue,
    target: Option<ProjectId>,
    policy: &SecurityPolicyRecord,
) -> Result<ProviderProbeMandateAudit, StoreError> {
    validate_issue(permission, ctx)?;
    ensure_scope_not_frozen_tx(tx, target, None, None)?;
    ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, ctx.caller.actor_id.as_str(), target)?;
    let actor: Option<String> = tx
        .query_row(
            "SELECT actor_id FROM principals WHERE principal_id=?1 AND state='active'",
            [permission.principal_id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    let actor = actor.ok_or_else(hidden)?;
    ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, &actor, target)?;
    let configured =
        provider_profiles::resolve(tx, permission.profile_id, permission.profile_version)?
            .ok_or_else(hidden)?;
    if configured.configuration.project_id != target {
        return Err(hidden());
    }
    provider_profiles::verify_secret(tx, &configured.configuration)?;
    let grant = Grant {
        schema_version: 1,
        mandate_ref: ctx.receipt_id,
        command_id: ctx.envelope.command_id,
        issued_by_actor: ctx.caller.actor_id.clone(),
        issued_at_ms: ctx.now,
        scope_kind: ctx.scope_kind,
        scope_id: ctx.scope_id.to_owned(),
        authority_generation: ctx.daemon.authority_generation,
        policy_revision: policy.revision,
        policy_digest: policy.policy_digest,
        permission: permission.clone(),
        profile_version_ref: configured.profile_version_ref,
        configuration_digest: configured.configuration_digest,
        configuration_receipt_id: configured.receipt_id,
        destination: configured.configuration.payload.endpoint().to_owned(),
    };
    let digest = canonical_sha256(&grant)?;
    tx.execute("INSERT INTO provider_probe_mandates(mandate_ref,scope_kind,scope_id,authority_generation,principal_id,profile_version_ref,profile_id,profile_version,configuration_digest,phase,expires_at_ms,issued_by_actor,issued_at_ms,mandate_digest,grant_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)", params![ctx.receipt_id.to_string(),ctx.scope_kind.as_str(),ctx.scope_id,ctx.daemon.authority_generation.to_string(),permission.principal_id.as_str(),grant.profile_version_ref,permission.profile_id.to_string(),i64::try_from(permission.profile_version).map_err(|_|invalid("version de profil hors plage"))?,grant.configuration_digest.to_string(),phase(permission.phase)?,permission.expires_at_ms.as_i64(),ctx.caller.actor_id.as_str(),ctx.now.as_i64(),digest.to_string(),serde_json::to_string(&grant)?])?;
    Ok(ProviderProbeMandateAudit {
        kind: "issue",
        mandate_ref: ctx.receipt_id,
        change_digest: digest,
    })
}

fn phase(value: ProviderTestPhase) -> Result<&'static str, StoreError> {
    match value {
        ProviderTestPhase::Connect => Ok("connect"),
        ProviderTestPhase::Inventory => Ok("inventory"),
        ProviderTestPhase::GenerationProbe => Ok("generation_probe"),
        _ => Err(invalid("phase active de mandat invalide")),
    }
}

fn read_one(conn: &Connection, reference: ReceiptId) -> Result<Option<StoredGrant>, StoreError> {
    let row: Option<(String,String,Option<String>,Option<String>)> = conn.query_row("SELECT grant_json,mandate_digest,revocation_json,revocation_digest FROM provider_probe_mandates WHERE mandate_ref=?1", [reference.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    row.map(|(grant, digest, revoked, revoke_digest)| {
        let grant: Grant = serde_json::from_str(&grant)?;
        let digest: Sha256Digest = digest.parse()?;
        if grant.schema_version != 1
            || grant.mandate_ref != reference
            || canonical_sha256(&grant)? != digest
        {
            return Err(StoreError::InvariantViolation(
                "binding de mandat divergent".to_owned(),
            ));
        }
        let revocation: Option<Revocation> = revoked
            .map(|json| serde_json::from_str(&json))
            .transpose()?;
        match (&revocation, revoke_digest) {
            (None, None) => {}
            (Some(revocation), Some(hash))
                if revocation.schema_version == 1
                    && revocation.mandate_ref == reference
                    && revocation.mandate_digest == digest
                    && revocation.authority_generation == grant.authority_generation
                    && revocation.scope_kind == grant.scope_kind
                    && revocation.scope_id == grant.scope_id
                    && canonical_sha256(revocation)? == hash.parse::<Sha256Digest>()? => {}
            _ => {
                return Err(StoreError::InvariantViolation(
                    "révocation de mandat divergente".to_owned(),
                ));
            }
        }
        Ok(StoredGrant {
            grant,
            digest,
            revocation,
        })
    })
    .transpose()
}

/// Résolution interne d'une émission ; aucune autorisation n'est portée par le client.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ResolvedConnectGrant {
    pub principal_id: ActorId,
    pub mandate_ref: ReceiptId,
    pub mandate_digest: Sha256Digest,
    pub expires_at_ms: TimestampMs,
}

/// Relit les octets immuables, leur reçu et la révocation dans la transaction writer.
/// Une projection `ReadPolicySummary` ou une chaîne `mandate_ref` ne confère aucun droit.
pub(crate) fn resolve_connect_grant(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    wire: &TestProviderProfilePayload,
    profile: &crate::ConfiguredProviderProfile,
) -> Result<ResolvedConnectGrant, StoreError> {
    resolve_transport_grant(
        tx,
        daemon,
        now,
        &TransportGrantRequest {
            envelope,
            caller,
            wire,
            profile,
            phase: crate::provider_probe_connect::TransportPhase::Connect,
        },
    )
}

pub(crate) struct TransportGrantRequest<'a> {
    pub envelope: &'a CommandEnvelope,
    pub caller: &'a CallerAuthority,
    pub wire: &'a TestProviderProfilePayload,
    pub profile: &'a crate::ConfiguredProviderProfile,
    pub phase: crate::provider_probe_connect::TransportPhase,
}

pub(crate) fn resolve_transport_grant(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    request: &TransportGrantRequest<'_>,
) -> Result<ResolvedConnectGrant, StoreError> {
    let TransportGrantRequest {
        envelope,
        caller,
        wire,
        profile,
        phase,
    } = request;
    require_security_admin(caller, "TestProviderProfile")?;
    if envelope.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: envelope.authority_generation,
            command_id: Some(envelope.command_id),
        });
    }
    resolve_connect_binding(
        tx,
        daemon,
        now,
        &ConnectGrantBinding {
            actor_id: &caller.actor_id,
            project_id: envelope.project_id,
            wire,
            profile,
            phase: *phase,
        },
    )
}

struct ConnectGrantBinding<'a> {
    actor_id: &'a ActorId,
    project_id: Option<ProjectId>,
    wire: &'a TestProviderProfilePayload,
    profile: &'a crate::ConfiguredProviderProfile,
    phase: crate::provider_probe_connect::TransportPhase,
}

/// Checks the already authorized context's facts for admission. This does not
/// construct a caller role, a fresh claim or permission to append broker phases.
pub(crate) fn revalidate_connect_grant(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    context: &crate::ProviderProbeConnectContext,
    profile: &crate::ConfiguredProviderProfile,
) -> Result<ResolvedConnectGrant, StoreError> {
    revalidate_transport_grant(
        tx,
        daemon,
        now,
        context,
        profile,
        crate::provider_probe_connect::TransportPhase::Connect,
    )
}

pub(crate) fn revalidate_transport_grant(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    context: &crate::ProviderProbeConnectContext,
    profile: &crate::ConfiguredProviderProfile,
    phase: crate::provider_probe_connect::TransportPhase,
) -> Result<ResolvedConnectGrant, StoreError> {
    let wire = TestProviderProfilePayload::try_new(
        context.profile_id.to_string(),
        context.profile_version,
        context.phase,
        context.capability_set.clone(),
        context.limits.clone(),
        context.locality,
        context.mandate_ref.to_string(),
    )
    .map_err(|_| hidden())?;
    resolve_connect_binding(
        tx,
        daemon,
        now,
        &ConnectGrantBinding {
            actor_id: &context.actor_id,
            project_id: context.project_id,
            wire: &wire,
            profile,
            phase,
        },
    )
}

fn resolve_connect_binding(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    binding: &ConnectGrantBinding<'_>,
) -> Result<ResolvedConnectGrant, StoreError> {
    let ConnectGrantBinding {
        actor_id,
        project_id,
        wire,
        profile,
        phase,
    } = binding;
    let reference: ReceiptId = wire.mandate_ref().parse().map_err(|_| hidden())?;
    if reference.as_uuid().is_nil() || reference.to_string() != wire.mandate_ref() {
        return Err(hidden());
    }
    let stored = read_one(tx, reference)?.ok_or_else(hidden)?;
    let grant = &stored.grant;
    let principal = grant.permission.principal_id.clone();
    let current_principal: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM principals WHERE principal_id=?1 AND actor_id=?2 AND state='active' AND principal_kind IN ('user','service','system'))",
        params![principal.as_str(),actor_id.as_str()], |row|row.get(0),
    )?;
    let target = project(grant.scope_kind, &grant.scope_id)?;
    if stored.revocation.is_some()
        || grant.authority_generation != daemon.authority_generation
        || grant.permission.phase != phase.wire()
        || wire.phase() != phase.wire()
        || grant.permission.expires_at_ms <= now
        || grant.issued_at_ms > now
        || !current_principal
        || target != *project_id
        || target != profile.configuration.project_id
        || grant.permission.profile_id != profile.configuration.profile_id
        || wire.profile_id() != profile.configuration.profile_id.to_string()
        || wire.profile_version() != profile.configuration.profile_version
        || grant.permission.profile_version != profile.configuration.profile_version
        || grant.profile_version_ref != profile.profile_version_ref
        || grant.configuration_digest != profile.configuration_digest
        || grant.configuration_receipt_id != profile.receipt_id
        || grant.destination != profile.configuration.payload.endpoint()
    {
        return Err(hidden());
    }
    validate_connect_bounds(wire, &grant.permission, profile, *phase)?;
    let source: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND command_type='SetPolicy' AND command_id=?2 AND actor_id=?3 AND authority_generation=?4 AND state IN ('accepted','completed'))",
        params![reference.to_string(),grant.command_id.to_string(),grant.issued_by_actor.as_str(),daemon.authority_generation.to_string()], |row|row.get(0),
    )?;
    if !source {
        return Err(hidden());
    }
    ensure_scope_not_frozen_tx(tx, target, None, None)?;
    for subject in [principal.as_str(), actor_id.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, subject, target)?;
    }
    provider_profiles::verify_secret(tx, &profile.configuration)?;
    Ok(ResolvedConnectGrant {
        principal_id: principal,
        mandate_ref: reference,
        mandate_digest: stored.digest,
        expires_at_ms: grant.permission.expires_at_ms,
    })
}

fn validate_connect_bounds(
    wire: &TestProviderProfilePayload,
    permission: &ProviderProbeMandateIssue,
    profile: &crate::ConfiguredProviderProfile,
    phase: crate::provider_probe_connect::TransportPhase,
) -> Result<(), StoreError> {
    let limits = wire.limits();
    if limits.timeout_ms() > permission.limits.timeout_ms()
        || (phase == crate::provider_probe_connect::TransportPhase::Connect
            && limits.timeout_ms() > profile.configuration.payload.limits().connect_timeout_ms)
        || limits.max_calls() > permission.limits.max_calls()
        || limits.max_output_tokens() > permission.limits.max_output_tokens()
        || limits.max_cost_usd_micros() > permission.limits.max_cost_usd_micros()
        || wire
            .capability_set()
            .iter()
            .any(|capability| !permission.capability_set.contains(capability))
        || (wire.locality() == DataLocality::ApprovedRemote
            && permission.locality == DataLocality::LocalOnly)
        || (wire.locality() == DataLocality::LocalOnly
            && profile.configuration.payload.locality() != DataLocality::LocalOnly)
    {
        return Err(hidden());
    }
    Ok(())
}

fn revoke_tx(
    tx: &Transaction<'_>,
    ctx: &PolicyMandateCommand<'_>,
    reference: ReceiptId,
    policy: &SecurityPolicyRecord,
) -> Result<ProviderProbeMandateAudit, StoreError> {
    if ctx.receipt_id == reference {
        return Err(invalid(
            "la révocation exige un reçu distinct de l'émission",
        ));
    }
    let stored = read_one(tx, reference)?.ok_or_else(hidden)?;
    if stored.grant.scope_kind != ctx.scope_kind
        || stored.grant.scope_id != ctx.scope_id
        || stored.grant.authority_generation != ctx.daemon.authority_generation
    {
        return Err(hidden());
    }
    if stored.revocation.is_some() {
        return Err(invalid("mandat déjà révoqué"));
    }
    let revocation = Revocation {
        schema_version: 1,
        mandate_ref: reference,
        mandate_digest: stored.digest,
        receipt_id: ctx.receipt_id,
        command_id: ctx.envelope.command_id,
        revoked_by_actor: ctx.caller.actor_id.clone(),
        revoked_at_ms: ctx.now,
        authority_generation: ctx.daemon.authority_generation,
        scope_kind: ctx.scope_kind,
        scope_id: ctx.scope_id.to_owned(),
        policy_revision: policy.revision,
        policy_digest: policy.policy_digest,
    };
    let digest = canonical_sha256(&revocation)?;
    let changed = tx.execute("UPDATE provider_probe_mandates SET revocation_receipt_id=?2,revocation_digest=?3,revocation_json=?4 WHERE mandate_ref=?1 AND revocation_receipt_id IS NULL", params![reference.to_string(),ctx.receipt_id.to_string(),digest.to_string(),serde_json::to_string(&revocation)?])?;
    if changed != 1 {
        return Err(invalid("révocation de mandat concurrente"));
    }
    Ok(ProviderProbeMandateAudit {
        kind: "revoke",
        mandate_ref: reference,
        change_digest: digest,
    })
}

/// Engage le CAS de politique et son mandat avant l'insertion du nouveau reçu.
pub(crate) fn apply_policy_change_tx(
    tx: &Transaction<'_>,
    ctx: &PolicyMandateCommand<'_>,
    change: &ProviderProbeMandateChange,
) -> Result<(Revision, ProviderProbeMandateAudit), StoreError> {
    let target = validate_command(ctx, change)?;
    let existing: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 OR (actor_id=?2 AND command_id=?3)) OR EXISTS(SELECT 1 FROM provider_probe_mandates WHERE mandate_ref=?1 OR revocation_receipt_id=?1)",
        params![ctx.receipt_id.to_string(),ctx.actor_id.as_str(),ctx.envelope.command_id.to_string()],
        |row|row.get(0),
    )?;
    if existing || ctx.receipt_id.as_uuid().is_nil() {
        return Err(invalid("le mandat exige le nouveau reçu de cette commande"));
    }
    let revision = crate::policies_secrets::tx_set_policy(
        tx,
        ctx.now,
        ctx.envelope.expected_revision,
        ctx.envelope.command_id,
        ctx.scope_kind,
        ctx.scope_id,
        ctx.isolation_mode,
        ctx.policy_version,
        ctx.reason,
        ctx.caller,
    )?;
    let policy = crate::policies_secrets::read_policy_row(tx, ctx.scope_kind, ctx.scope_id)?
        .ok_or_else(hidden)?;
    let audit = match change {
        ProviderProbeMandateChange::Issue(permission) => {
            issue_tx(tx, ctx, permission, target, &policy)
        }
        ProviderProbeMandateChange::Revoke { mandate_ref } => {
            revoke_tx(tx, ctx, *mandate_ref, &policy)
        }
    }?;
    Ok((revision, audit))
}

fn view(
    stored: StoredGrant,
    authority: AuthorityGeneration,
    now: TimestampMs,
) -> ProviderProbeMandateView {
    let grant = stored.grant;
    let status = if grant.authority_generation != authority {
        ProviderProbeMandateStatus::Historical
    } else if stored.revocation.is_some() {
        ProviderProbeMandateStatus::Revoked
    } else if grant.permission.expires_at_ms.as_i64() <= now.as_i64() {
        ProviderProbeMandateStatus::Expired
    } else {
        ProviderProbeMandateStatus::Active
    };
    ProviderProbeMandateView {
        mandate_ref: grant.mandate_ref,
        scope_kind: grant.scope_kind,
        scope_id: grant.scope_id,
        authority_generation: grant.authority_generation,
        principal_id: grant.permission.principal_id,
        profile_id: grant.permission.profile_id,
        profile_version: grant.permission.profile_version,
        configuration_digest: grant.configuration_digest,
        phase: grant.permission.phase,
        locality: grant.permission.locality,
        capability_set: grant.permission.capability_set,
        limits: grant.permission.limits,
        expires_at_ms: grant.permission.expires_at_ms,
        mandate_digest: stored.digest,
        status,
        revocation_receipt_id: stored.revocation.map(|revocation| revocation.receipt_id),
    }
}

pub(crate) fn read_visible(
    conn: &Connection,
    project_id: Option<ProjectId>,
    now: TimestampMs,
) -> Result<Vec<ProviderProbeMandateView>, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    if daemon.schema_version < 13 {
        return Ok(Vec::new());
    }
    let mut query = conn.prepare("SELECT mandate_ref FROM provider_probe_mandates WHERE scope_kind='global' AND scope_id='global' OR scope_kind='project' AND scope_id=?1 ORDER BY mandate_ref")?;
    let references = query
        .query_map([project_id.map(|id| id.to_string())], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    references
        .into_iter()
        .map(|reference| {
            let stored = read_one(conn, reference.parse()?)?.ok_or_else(hidden)?;
            Ok(view(stored, daemon.authority_generation, now))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AggregateKind, BackupRetentionTier, CallerRole, CommandMutation, CommitCommandRequest,
        CutpointObserver, PrepareBackupRequest, PrepareObjectRequest, ProviderProfileConfiguration,
        ProviderProfilePreparation, PublishObjectReferenceRequest, StoreConfig, StoreCutpoint,
        UnleashStore, WriteCriticality,
    };
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use unleash_domain::{DaemonMode, MicrosAmount, RequestId};
    use unleash_protocol::{ProtocolVersion, ReceiptState, canonicalize_to_vec};
    use unleash_testkit::local_test_dir;

    const PROFILE: Uuid = Uuid::from_u128(0xf012_d013);
    const PRINCIPAL: &str = "mandate-fixture-principal";

    fn actor() -> ActorId {
        ActorId::new("user:mandate-fixture").unwrap()
    }

    fn configure(store: &UnleashStore) {
        store
            .register_principal(PRINCIPAL, "user", actor().as_str())
            .unwrap();
        let configuration = ProviderProfileConfiguration {
            schema_version:1, profile_id:PROFILE, profile_version:1, project_id:None,
            payload:serde_json::from_value(json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":"http://127.0.0.1:1234/v1","model":"synthetic-mandate-fixture","locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2000,"queue_timeout_ms":1000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":60_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}})).unwrap(),
            secret_binding:None, catalog_binding:None,
        };
        let bytes = canonicalize_to_vec(&configuration).unwrap();
        let hash = Sha256Digest::from_bytes(&bytes);
        let handle = store
            .prepare_object(PrepareObjectRequest {
                object_hash: hash,
                byte_length: u64::try_from(bytes.len()).unwrap(),
                media_type: "application/json".to_owned(),
                owner_kind: "mandate_fixture".to_owned(),
                owner_id: PROFILE.to_string(),
            })
            .unwrap();
        store.write_prepared_object_bytes(&handle, &bytes).unwrap();
        store
            .publish_owned_object_reference(PublishObjectReferenceRequest {
                pin_id: handle.pin_id,
                object_hash: hash,
                object_generation: handle.object_generation,
                owner_kind: "mandate_fixture".to_owned(),
                owner_id: PROFILE.to_string(),
                source_field: "configuration".to_owned(),
            })
            .unwrap();
        let payload = serde_json::to_value(&configuration.payload)
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        store
            .commit_command(CommitCommandRequest {
                actor_id: actor(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_u128(1),
                    command_id: CommandId::from_u128(1),
                    authority_generation: store.active_authority_generation().unwrap(),
                    actor_ref: Some(actor().to_string()),
                    project_id: None,
                    aggregate_id: PROFILE,
                    expected_revision: Revision::INITIAL,
                    command_type: "SetProviderProfile".to_owned(),
                    payload,
                },
                target_aggregate: AggregateKind::ProviderProfile,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::SetProviderProfile {
                    caller: CallerAuthority {
                        actor_id: actor(),
                        role: CallerRole::HumanOperator,
                    },
                    preparation: Box::new(ProviderProfilePreparation {
                        configuration,
                        content_object_hash: hash,
                    }),
                },
                events: vec![],
                capacity_reservations: vec![],
                budget_reservations: vec![],
            })
            .unwrap();
    }

    fn fixture() -> (UnleashStore, impl std::fmt::Debug) {
        let directory = local_test_dir("f012-mandate-store").unwrap();
        let store = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
        assert_eq!(store.daemon_state().unwrap().mode, DaemonMode::Ready);
        configure(&store);
        (store, directory)
    }

    fn change(expires: TimestampMs) -> ProviderProbeMandateChange {
        ProviderProbeMandateChange::Issue(ProviderProbeMandateIssue {
            principal_id: ActorId::new(PRINCIPAL).unwrap(),
            profile_id: PROFILE,
            profile_version: 1,
            phase: ProviderTestPhase::Connect,
            locality: DataLocality::LocalOnly,
            capability_set: vec![],
            limits: ProviderTestLimits::new(1000, 0, 0, MicrosAmount::ZERO),
            expires_at_ms: expires,
        })
    }

    fn wire(
        daemon: &DaemonStateRow,
        id: u128,
        change: &ProviderProbeMandateChange,
    ) -> CommandEnvelope {
        CommandEnvelope { protocol_version:ProtocolVersion, request_id:RequestId::from_u128(id),
            command_id:CommandId::from_u128(id), authority_generation:daemon.authority_generation,
            actor_ref:Some(actor().to_string()), project_id:None,
            aggregate_id:daemon.authority_generation.as_uuid(), expected_revision:Revision::INITIAL,
            command_type:"SetPolicy".to_owned(), payload:json!({"scope_kind":"global","scope_id":"global","isolation_mode":"standard","policy_version":"fixture-v1","reason":"synthetic writer context","provider_probe_mandate":change}).as_object().unwrap().clone() }
    }

    fn apply(
        tx: &Transaction<'_>,
        now: TimestampMs,
        id: u128,
        expected: u64,
        change: &ProviderProbeMandateChange,
    ) -> Result<ReceiptId, StoreError> {
        apply_with_receipt(tx, now, id, expected, ReceiptId::from_u128(id), change)
    }

    fn apply_with_receipt(
        tx: &Transaction<'_>,
        now: TimestampMs,
        id: u128,
        expected: u64,
        receipt_id: ReceiptId,
        change: &ProviderProbeMandateChange,
    ) -> Result<ReceiptId, StoreError> {
        let daemon = read_daemon_state_conn(tx)?;
        let mut envelope = wire(&daemon, id, change);
        envelope.expected_revision = Revision::new(expected);
        let caller = CallerAuthority {
            actor_id: actor(),
            role: CallerRole::HumanOperator,
        };
        let actor_id = actor();
        apply_policy_change_tx(
            tx,
            &PolicyMandateCommand {
                daemon: &daemon,
                envelope: &envelope,
                actor_id: &actor_id,
                caller: &caller,
                receipt_id,
                now,
                scope_kind: PolicyScopeKind::Global,
                scope_id: "global",
                isolation_mode: crate::PolicyIsolationMode::Standard,
                policy_version: "fixture-v1",
                reason: "synthetic writer context",
            },
            change,
        )?;
        Ok(receipt_id)
    }

    // Synthetic receipt rows only qualify the deferred FK and atomic writer primitive.
    // The distinct IPC oracle qualifies the real CommitCommand/receipt producer later.
    fn receipt_fixture(tx: &Transaction<'_>, now: TimestampMs, id: u128) -> Result<(), StoreError> {
        receipt_fixture_kind(tx, now, id, "SetPolicy")
    }

    fn receipt_fixture_kind(
        tx: &Transaction<'_>,
        now: TimestampMs,
        id: u128,
        kind: &str,
    ) -> Result<(), StoreError> {
        let daemon = read_daemon_state_conn(tx)?;
        tx.execute("INSERT INTO command_receipts(actor_id,command_id,authority_generation,receipt_id,request_id,command_type,command_digest,aggregate_id,state,response_state,response_json,response_retained_until_ms,committed_at_ms) VALUES(?1,?2,?3,?4,?2,?5,?6,?3,'accepted','tombstone',NULL,?7,?7)", params![actor().as_str(),CommandId::from_u128(id).to_string(),daemon.authority_generation.to_string(),ReceiptId::from_u128(id).to_string(),kind,Sha256Digest::from_bytes(b"synthetic receipt fixture").to_string(),now.as_i64()])?;
        Ok(())
    }

    fn mandate_count(store: &UnleashStore) -> i64 {
        store
            .read_snapshot(|conn, _| {
                Ok(
                    conn.query_row("SELECT COUNT(*) FROM provider_probe_mandates", [], |row| {
                        row.get(0)
                    })?,
                )
            })
            .unwrap()
    }

    fn policy_request(store: &UnleashStore, id: u128) -> CommitCommandRequest {
        let daemon = store.daemon_state().unwrap();
        let permission = change(TimestampMs::new(
            store.config().clock.now_ms().as_i64() + 60_000,
        ));
        CommitCommandRequest {
            actor_id: actor(),
            envelope: wire(&daemon, id, &permission),
            target_aggregate: AggregateKind::SecurityPolicy,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetPolicyWithProviderProbeMandate {
                scope_kind: PolicyScopeKind::Global,
                scope_id: "global".to_owned(),
                isolation_mode: crate::PolicyIsolationMode::Standard,
                policy_version: "fixture-v1".to_owned(),
                reason: "synthetic writer context".to_owned(),
                caller: CallerAuthority {
                    actor_id: actor(),
                    role: CallerRole::HumanOperator,
                },
                mandate: permission,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        }
    }

    fn commit_counts(store: &UnleashStore) -> (i64, i64, i64, i64) {
        store
            .read_snapshot(|conn, _| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?,
                    conn.query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
                        row.get(0)
                    })?,
                    conn.query_row("SELECT COUNT(*) FROM provider_probe_mandates", [], |row| {
                        row.get(0)
                    })?,
                    conn.query_row("SELECT COUNT(*) FROM outbox", [], |row| row.get(0))?,
                ))
            })
            .unwrap()
    }

    #[test]
    fn t_f012_013_commit_command_emits_once_and_classic_policy_preserves_the_grant() {
        let (store, _directory) = fixture();
        let request = policy_request(&store, 200);
        let receipt = store
            .commit_command(request.clone())
            .unwrap()
            .receipt()
            .clone();
        let snapshot = store.read_policy_summary_snapshot(None).unwrap();
        assert_eq!(snapshot.cursor, receipt.event_cursor);
        assert_eq!(snapshot.value.provider_probe_mandates.len(), 1);
        let original = snapshot
            .value
            .provider_probe_mandates
            .first()
            .unwrap()
            .clone();
        assert_eq!(
            original.mandate_ref, receipt.receipt_id,
            "identité du vrai reçu engagé"
        );
        assert_eq!(original.status, ProviderProbeMandateStatus::Active);
        let before = commit_counts(&store);
        let replay = store.commit_command(request.clone()).unwrap();
        assert_eq!(replay.receipt(), &receipt);
        assert!(matches!(replay, crate::CommitOutcome::IdempotentReplay(_)));
        assert_eq!(
            commit_counts(&store),
            before,
            "aucun reçu, événement, mandat ou outbox au rejeu"
        );
        let reference = receipt.receipt_id;
        store.read_snapshot(move|conn,_| {
            let delivered:i64=conn.query_row("SELECT COUNT(*) FROM command_receipts c JOIN events e ON e.sequence=c.last_event_seq JOIN outbox o ON o.event_id=e.event_id WHERE c.receipt_id=?1 AND o.destination='clients'",[reference.to_string()],|row|row.get(0))?;
            assert_eq!(delivered,1,"notification engagée dans le même journal/outbox");
            Ok(())
        }).unwrap();
        let mut classic = request;
        classic.envelope.command_id = CommandId::from_u128(201);
        classic.envelope.request_id = RequestId::from_u128(201);
        classic.envelope.expected_revision = receipt.new_revision;
        classic.envelope.payload.remove("provider_probe_mandate");
        classic
            .envelope
            .payload
            .insert("isolation_mode".to_owned(), json!("strict"));
        classic.mutation = CommandMutation::SetPolicy {
            scope_kind: PolicyScopeKind::Global,
            scope_id: "global".to_owned(),
            isolation_mode: crate::PolicyIsolationMode::Strict,
            policy_version: "fixture-v1".to_owned(),
            reason: "synthetic writer context".to_owned(),
            caller: CallerAuthority {
                actor_id: actor(),
                role: CallerRole::HumanOperator,
            },
        };
        let updated = store.commit_command(classic).unwrap().receipt().clone();
        let after = store.read_policy_summary_snapshot(None).unwrap();
        assert_eq!(after.cursor, updated.event_cursor);
        assert_eq!(updated.new_revision, Revision::new(2));
        assert_eq!(
            after.value.effective_isolation_mode,
            crate::PolicyIsolationMode::Strict
        );
        assert_eq!(after.value.provider_probe_mandates, vec![original]);
        let counters = commit_counts(&store);
        assert_eq!(counters.0, before.0 + 1);
        assert_eq!(counters.1, before.1 + 1);
        assert_eq!(counters.2, 1);
    }

    #[test]
    fn t_f012_013_mandate_and_late_receipt_commit_atomically_without_effect() {
        let (store, _directory) = fixture();
        store
            .execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
                let reference = apply(
                    tx,
                    now,
                    100,
                    0,
                    &change(TimestampMs::new(now.as_i64() + 60_000)),
                )?;
                let present: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM command_receipts WHERE receipt_id=?1",
                    [reference.to_string()],
                    |row| row.get(0),
                )?;
                assert_eq!(present, 0, "la FK est différée jusqu'à l'insertion du reçu");
                receipt_fixture(tx, now, 100)
            })
            .unwrap();
        assert_eq!(mandate_count(&store), 1);
        let summary = store.read_policy_summary_snapshot(None).unwrap();
        assert_eq!(summary.value.provider_probe_mandates.len(), 1);
        assert_eq!(
            summary
                .value
                .provider_probe_mandates
                .first()
                .unwrap()
                .status,
            ProviderProbeMandateStatus::Active
        );
        store
            .read_snapshot(|conn, _| {
                for table in [
                    "provider_probes",
                    "actions",
                    "action_dispatches",
                    "model_calls",
                    "capacity_reservations",
                    "budget_reservations",
                    "secret_references",
                ] {
                    let count: i64 =
                        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                            row.get(0)
                        })?;
                    assert_eq!(count, 0);
                }
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn t_f012_013_missing_receipt_or_writer_error_rolls_back_policy_and_mandate() {
        let (store, _directory) = fixture();
        let missing = store.execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
            apply(
                tx,
                now,
                100,
                0,
                &change(TimestampMs::new(now.as_i64() + 60_000)),
            )?;
            Ok(())
        });
        assert!(
            missing.is_err(),
            "commit sans reçu interdit par la FK différée"
        );
        assert_eq!(mandate_count(&store), 0);
        let failed: Result<(), StoreError> =
            store.execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
                apply(
                    tx,
                    now,
                    101,
                    0,
                    &change(TimestampMs::new(now.as_i64() + 60_000)),
                )?;
                receipt_fixture(tx, now, 101)?;
                Err(StoreError::CutpointInjected {
                    cutpoint: "synthetic transaction rollback".to_owned(),
                    message: "fixture".to_owned(),
                })
            });
        assert!(matches!(failed, Err(StoreError::CutpointInjected { .. })));
        assert_eq!(mandate_count(&store), 0);
        assert!(
            store
                .read_policy_summary(None)
                .unwrap()
                .global_policy
                .is_none()
        );
    }

    #[test]
    fn t_f012_013_binding_revocation_expiry_and_lineage_are_distinct() {
        let (store, _directory) = fixture();
        let expiry = store
            .execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
                let expiry = TimestampMs::new(now.as_i64() + 60_000);
                apply(tx, now, 100, 0, &change(expiry))?;
                receipt_fixture(tx, now, 100)?;
                Ok(expiry)
            })
            .unwrap();
        let self_revoke = store.execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
            apply_with_receipt(
                tx,
                now,
                101,
                1,
                ReceiptId::from_u128(100),
                &ProviderProbeMandateChange::Revoke {
                    mandate_ref: ReceiptId::from_u128(100),
                },
            )
        });
        assert!(matches!(
            self_revoke,
            Err(StoreError::InvalidArgument { .. })
        ));
        assert_eq!(
            store
                .read_policy_summary(None)
                .unwrap()
                .global_policy
                .unwrap()
                .revision,
            Revision::new(1)
        );
        store.execute_in_writer_tx(WriteCriticality::Normal,move|tx,now| {
            assert!(tx.execute("UPDATE provider_probe_mandates SET expires_at_ms=expires_at_ms+1",[]).is_err());
            let expired=read_visible(tx,None,expiry)?;
            let expired = expired.first().unwrap();
            assert_eq!(expired.status,ProviderProbeMandateStatus::Expired);
            let old=read_one(tx,ReceiptId::from_u128(100))?.ok_or_else(hidden)?;
            let historical=view(old,AuthorityGeneration::from_u128(0xf012_dead),now);
            assert_eq!(historical.status,ProviderProbeMandateStatus::Historical,"autorité injectée du banc seulement");
            let revoke=ProviderProbeMandateChange::Revoke {mandate_ref:ReceiptId::from_u128(100)};
            apply(tx,now,101,1,&revoke)?;
            receipt_fixture(tx,now,101)?;
            let revoked=read_visible(tx,None,now)?;
            let revoked = revoked.first().unwrap();
            assert_eq!(revoked.status,ProviderProbeMandateStatus::Revoked);
            assert_eq!(revoked.revocation_receipt_id,Some(ReceiptId::from_u128(101)));
            assert_eq!(revoked.mandate_digest,expired.mandate_digest);
            assert!(tx.execute("UPDATE provider_probe_mandates SET revocation_receipt_id=revocation_receipt_id",[]).is_err());
            Ok(())
        }).unwrap();
    }

    struct MigrationFailure(AtomicBool);
    impl CutpointObserver for MigrationFailure {
        fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
            if point == StoreCutpoint::C11MigrationInProgress
                && self.0.swap(false, Ordering::AcqRel)
            {
                return Err(StoreError::CutpointInjected {
                    cutpoint: format!("{point:?}"),
                    message: "synthetic V013 migration interruption".to_owned(),
                });
            }
            Ok(())
        }
    }

    #[test]
    fn t_f012_013_v013_preserves_v012_profiles_and_migrates_atomically() {
        let directory = local_test_dir("f012-mandates-migration").unwrap();
        let mut old = StoreConfig::new(directory.path());
        old.max_supported_schema_version = 12;
        let store = UnleashStore::open(old.clone()).unwrap();
        configure(&store);
        let before = store
            .resolve_provider_profile_snapshot(PROFILE, 1)
            .unwrap()
            .value
            .unwrap();
        let backup = store
            .prepare_backup(PrepareBackupRequest {
                destination_dir: directory.path().join("pre-migration"),
                retention_tier: BackupRetentionTier::Manual,
                coverage: "db_and_objects".to_owned(),
            })
            .unwrap();
        store.shutdown().unwrap();
        let mut interrupted = StoreConfig::new(directory.path());
        interrupted.max_supported_schema_version = 13;
        interrupted.cutpoints = Arc::new(MigrationFailure(AtomicBool::new(true)));
        assert!(
            UnleashStore::open_with_migration_backup(interrupted, Some(backup.backup_id)).is_err()
        );
        let historical = UnleashStore::open(old).unwrap();
        assert_eq!(historical.daemon_state().unwrap().schema_version, 12);
        historical
            .read_snapshot(|conn, _| {
                let exists: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name='provider_probe_mandates'",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(exists, 0);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            historical
                .resolve_provider_profile_snapshot(PROFILE, 1)
                .unwrap()
                .value
                .unwrap(),
            before
        );
        historical.shutdown().unwrap();
        let mut target = StoreConfig::new(directory.path());
        target.max_supported_schema_version = 13;
        let current =
            UnleashStore::open_with_migration_backup(target, Some(backup.backup_id)).unwrap();
        assert_eq!(current.daemon_state().unwrap().schema_version, 13);
        assert_eq!(
            current
                .resolve_provider_profile_snapshot(PROFILE, 1)
                .unwrap()
                .value
                .unwrap(),
            before
        );
        assert_eq!(mandate_count(&current), 0);
        current
            .read_snapshot(|conn, _| {
                let broken: i64 =
                    conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                        row.get(0)
                    })?;
                assert_eq!(broken, 0);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn t_f012_013_old_receipt_prior_cas_and_unrelated_receipt_cannot_issue() {
        let (store, _directory) = fixture();
        let configured_receipt = store
            .resolve_provider_profile_snapshot(PROFILE, 1)
            .unwrap()
            .value
            .unwrap()
            .receipt_id;
        let old = store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            apply_with_receipt(
                tx,
                now,
                100,
                0,
                configured_receipt,
                &change(TimestampMs::new(now.as_i64() + 60_000)),
            )
        });
        assert!(matches!(old, Err(StoreError::InvalidArgument { .. })));
        assert!(
            store
                .read_policy_summary(None)
                .unwrap()
                .global_policy
                .is_none()
        );
        store
            .execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
                crate::policies_secrets::tx_set_policy(
                    tx,
                    now,
                    Revision::INITIAL,
                    CommandId::from_u128(50),
                    PolicyScopeKind::Global,
                    "global",
                    crate::PolicyIsolationMode::Standard,
                    "fixture-v1",
                    "prior transaction",
                    &CallerAuthority {
                        actor_id: actor(),
                        role: CallerRole::HumanOperator,
                    },
                )?;
                Ok(())
            })
            .unwrap();
        let prior = store.execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
            apply(
                tx,
                now,
                100,
                0,
                &change(TimestampMs::new(now.as_i64() + 60_000)),
            )
        });
        assert!(matches!(prior, Err(StoreError::RevisionConflict { .. })));
        let wrong = store.execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
            apply(
                tx,
                now,
                101,
                1,
                &change(TimestampMs::new(now.as_i64() + 60_000)),
            )?;
            receipt_fixture_kind(tx, now, 101, "SetSettings")
        });
        assert!(
            wrong.is_err(),
            "un autre type de reçu ne cause pas l'émission"
        );
        assert_eq!(mandate_count(&store), 0);
        assert_eq!(
            store
                .read_policy_summary(None)
                .unwrap()
                .global_policy
                .unwrap()
                .revision,
            Revision::new(1)
        );
    }
}
