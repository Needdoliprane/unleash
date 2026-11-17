//! Clôtures monotones de ressources (`resource_fences`) et baux (`resource_leases`, `REQ-PERSIST-009`).

use serde::{Deserialize, Serialize};
use unleash_domain::{AuthorityGeneration, DaemonEpoch, Fence, LeaseId, TimestampMs};

/// État d'une lease de ressource exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseState {
    /// Détenue activement dans l'époque courante.
    Held,
    /// Marquée suspecte après redémarrage dans la même lignée.
    Suspect,
    /// En cours de réconciliation.
    Reconciling,
    /// Libérée proprement (sans jamais remettre `last_fence` à zéro).
    Released,
    /// Révoquée explicitement au profit d'un successeur.
    Revoked,
}

impl LeaseState {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::Suspect => "suspect",
            Self::Reconciling => "reconciling",
            Self::Released => "released",
            Self::Revoked => "revoked",
        }
    }
}

/// Demande d'acquisition de lease exclusive avec incrément atomique du fence monotone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquireLeaseRequest {
    /// Famille de ressource (ex: `"worktree"`, `"terminal"`, `"git_ref"`).
    pub resource_kind: String,
    /// Clé canonique de la ressource.
    pub resource_key: String,
    /// Type de propriétaire (ex: `"worker"`, `"agent"`, `"host"`).
    pub owner_type: String,
    /// Identifiant du propriétaire.
    pub owner_id: String,
    /// Durée indicative TTL en ms (> 0, purement informative après reboot).
    pub ttl_ms: i64,
}

/// Enregistrement complet d'une lease acquise ou révoquée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLeaseRecord {
    /// Identifiant unique de la lease.
    pub lease_id: LeaseId,
    /// Lignée d'autorité propriétaire.
    pub authority_generation: AuthorityGeneration,
    /// Type de ressource.
    pub resource_kind: String,
    /// Clé de ressource.
    pub resource_key: String,
    /// Type de propriétaire.
    pub owner_type: String,
    /// Identifiant du propriétaire.
    pub owner_id: String,
    /// Époque de daemon à l'acquisition.
    pub daemon_epoch: DaemonEpoch,
    /// Compteur de fence monotone attribué (>= 1, jamais remis à zéro).
    pub fence: Fence,
    /// Identité de boot observée.
    pub boot_identity: String,
    /// Échéance TTL indicative.
    pub expires_at_ms: TimestampMs,
    /// État courant de la lease.
    pub state: LeaseState,
}

/// Assertion de propriété portée par une mutation ou un résultat tardif de worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceAssertion {
    /// Identifiant de la lease invoquée.
    pub lease_id: LeaseId,
    /// Lignée d'autorité présentée par l'exécuteur.
    pub authority_generation: AuthorityGeneration,
    /// Époque de daemon présentée.
    pub daemon_epoch: DaemonEpoch,
    /// Valeur de fence présentée.
    pub fence: Fence,
    /// Propriétaire invoqué.
    pub owner_id: String,
}

use crate::error::StoreError;
use rusqlite::{OptionalExtension, Transaction};

pub(crate) struct LeaseLineageCtx<'a> {
    pub(crate) authority: AuthorityGeneration,
    pub(crate) daemon_epoch: DaemonEpoch,
    pub(crate) boot_identity: &'a str,
    pub(crate) now_ms: TimestampMs,
}

pub(crate) fn tx_acquire_lease(
    tx: &Transaction<'_>,
    ctx: &LeaseLineageCtx<'_>,
    lease_id: LeaseId,
    req: &AcquireLeaseRequest,
) -> Result<ResourceLeaseRecord, StoreError> {
    let authority = ctx.authority;
    let daemon_epoch = ctx.daemon_epoch;
    let boot_identity = ctx.boot_identity;
    let now_ms = ctx.now_ms;
    if req.ttl_ms <= 0 {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "ttl_ms doit être strictement positif".to_owned(),
        });
    }

    let existing_active: Option<String> = tx
        .query_row(
            "SELECT lease_id FROM resource_leases
             WHERE authority_generation = ?1 AND resource_kind = ?2 AND resource_key = ?3
               AND state IN ('held', 'suspect', 'reconciling')",
            rusqlite::params![authority.to_string(), req.resource_kind, req.resource_key],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    if let Some(active_id) = existing_active {
        return Err(StoreError::CapacityBlocked(format!(
            "ressource '{}:{}' déjà verrouillée par lease {active_id}",
            req.resource_kind, req.resource_key
        )));
    }

    let last_fence: Option<i64> = tx
        .query_row(
            "SELECT last_fence FROM resource_fences
             WHERE authority_generation = ?1 AND resource_kind = ?2 AND resource_key = ?3",
            rusqlite::params![authority.to_string(), req.resource_kind, req.resource_key],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let next_fence_i64 = if let Some(prev) = last_fence {
        prev.saturating_add(1)
    } else {
        tx.execute(
            "INSERT INTO resource_fences (authority_generation, resource_kind, resource_key, last_fence)
             VALUES (?1, ?2, ?3, 0)",
            rusqlite::params![authority.to_string(), req.resource_kind, req.resource_key],
        )
        .map_err(StoreError::from_rusqlite)?;
        1
    };

    tx.execute(
        "UPDATE resource_fences SET last_fence = ?1
         WHERE authority_generation = ?2 AND resource_kind = ?3 AND resource_key = ?4",
        rusqlite::params![
            next_fence_i64,
            authority.to_string(),
            req.resource_kind,
            req.resource_key
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    let expires_at_ms = now_ms.as_i64().saturating_add(req.ttl_ms);
    let epoch_i64 = i64::try_from(daemon_epoch.as_u64()).unwrap_or(i64::MAX);
    tx.execute(
        "INSERT INTO resource_leases
         (lease_id, authority_generation, resource_kind, resource_key, owner_type, owner_id,
          daemon_epoch, fence, boot_identity, ttl_ms, last_heartbeat_seq, expires_at_ms, state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, 'held')",
        rusqlite::params![
            lease_id.to_string(),
            authority.to_string(),
            req.resource_kind,
            req.resource_key,
            req.owner_type,
            req.owner_id,
            epoch_i64,
            next_fence_i64,
            boot_identity,
            req.ttl_ms,
            expires_at_ms
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    let fence_u64 = u64::try_from(next_fence_i64).unwrap_or(1);
    Ok(ResourceLeaseRecord {
        lease_id,
        authority_generation: authority,
        resource_kind: req.resource_kind.clone(),
        resource_key: req.resource_key.clone(),
        owner_type: req.owner_type.clone(),
        owner_id: req.owner_id.clone(),
        daemon_epoch,
        fence: Fence::new(fence_u64),
        boot_identity: boot_identity.to_owned(),
        expires_at_ms: TimestampMs::new(expires_at_ms),
        state: LeaseState::Held,
    })
}

pub(crate) fn tx_revoke_and_reassign_lease(
    tx: &Transaction<'_>,
    ctx: &LeaseLineageCtx<'_>,
    old_lease_id: LeaseId,
    new_lease_id: LeaseId,
    req: &AcquireLeaseRequest,
) -> Result<ResourceLeaseRecord, StoreError> {
    let updated = tx
        .execute(
            "UPDATE resource_leases SET state = 'revoked'
             WHERE lease_id = ?1 AND state IN ('held', 'suspect', 'reconciling')",
            [old_lease_id.to_string()],
        )
        .map_err(StoreError::from_rusqlite)?;
    if updated == 0 {
        return Err(StoreError::NotFound(format!(
            "lease active introuvable : {old_lease_id}"
        )));
    }
    tx_acquire_lease(tx, ctx, new_lease_id, req)
}

pub(crate) fn tx_release_lease(tx: &Transaction<'_>, lease_id: LeaseId) -> Result<(), StoreError> {
    let updated = tx
        .execute(
            "UPDATE resource_leases SET state = 'released'
             WHERE lease_id = ?1 AND state IN ('held', 'suspect', 'reconciling')",
            [lease_id.to_string()],
        )
        .map_err(StoreError::from_rusqlite)?;
    if updated == 0 {
        return Err(StoreError::NotFound(format!(
            "lease active à libérer introuvable : {lease_id}"
        )));
    }
    Ok(())
}

pub(crate) fn tx_validate_lease_fence(
    tx: &Transaction<'_>,
    active_authority: AuthorityGeneration,
    active_epoch: DaemonEpoch,
    assertion: &FenceAssertion,
) -> Result<(), StoreError> {
    if assertion.authority_generation != active_authority {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: active_authority,
            actual: assertion.authority_generation,
            command_id: None,
        });
    }

    let row: Option<(String, String, i64, i64, String, String)> = tx
        .query_row(
            "SELECT resource_kind, resource_key, daemon_epoch, fence, owner_id, state
             FROM resource_leases WHERE lease_id = ?1",
            [assertion.lease_id.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((kind, key, lease_epoch, lease_fence, lease_owner, state)) = row else {
        return Err(StoreError::StaleFence {
            message: format!("lease inconnue : {}", assertion.lease_id),
        });
    };

    if state != "held" {
        return Err(StoreError::StaleFence {
            message: format!("lease {} dans l'état '{state}'", assertion.lease_id),
        });
    }
    let active_epoch_i64 = i64::try_from(active_epoch.as_u64()).unwrap_or(i64::MAX);
    let assertion_epoch_i64 = i64::try_from(assertion.daemon_epoch.as_u64()).unwrap_or(i64::MAX);
    if lease_epoch != active_epoch_i64 || assertion_epoch_i64 != active_epoch_i64 {
        return Err(StoreError::StaleFence {
            message: format!(
                "époque obsolète (attendue {active_epoch_i64}, reçue {assertion_epoch_i64})"
            ),
        });
    }
    let last_fence: i64 = tx
        .query_row(
            "SELECT last_fence FROM resource_fences
             WHERE authority_generation = ?1 AND resource_kind = ?2 AND resource_key = ?3",
            rusqlite::params![active_authority.to_string(), kind, key],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;

    let assertion_fence_i64 = i64::try_from(assertion.fence.as_u64()).unwrap_or(i64::MAX);
    if assertion_fence_i64 != lease_fence
        || assertion_fence_i64 < last_fence
        || lease_owner != assertion.owner_id
    {
        return Err(StoreError::StaleFence {
            message: format!(
                "fence ou propriétaire obsolète pour {kind}:{key} (fence={assertion_fence_i64}, actif={last_fence})"
            ),
        });
    }
    Ok(())
}
