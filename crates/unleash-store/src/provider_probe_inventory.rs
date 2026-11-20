//! Exact, nongenerative inventory preparation and original-owner capabilities.
//! The existing transport context and V008 ledger remain the only authorities.

use std::fmt;

use serde::{Deserialize, Serialize};
use unleash_domain::{LineageFence, ProviderProbeId, ReceiptId, Revision, Sha256Digest};
use unleash_protocol::{
    CommandEnvelope, ProviderAuthentication, ProviderDialectKind, ProviderKind,
    TestProviderProfilePayload,
};

use crate::provider_probe_connect::{self as transport, TransportPhase};
use crate::{CallerAuthority, SnapshotProjection, StoreError, UnleashStore, WriteCriticality};

/// Native and canonical list ceiling, shared by the inventory contract.
pub const INVENTORY_MAX_RESPONSE_BYTES: u64 =
    unleash_protocol::provider_inventory::MAX_INVENTORY_LIST_BYTES as u64;
/// Total native exchange ceiling; no generation timeout is substituted.
pub const INVENTORY_MAX_TIMEOUT_MS: u64 = 60_000;

/// Server-derived, immutable bounds. Missing inventory bounds never grant I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryTransportLimits {
    /// TCP/TLS establishment alone, bounded by the configured connection limit.
    pub connect_timeout_ms: u64,
    /// Whole exchange, including framing and channel closure.
    pub timeout_ms: u64,
    /// Decoded native body and canonical inventory ceiling.
    pub max_response_bytes: u64,
}

impl InventoryTransportLimits {
    pub(crate) fn validate(&self) -> Result<(), StoreError> {
        if self.connect_timeout_ms == 0
            || self.connect_timeout_ms > self.timeout_ms
            || self.timeout_ms > INVENTORY_MAX_TIMEOUT_MS
            || self.max_response_bytes != INVENTORY_MAX_RESPONSE_BYTES
        {
            return Err(invalid("bornes de transport inventory non qualifiées"));
        }
        Ok(())
    }
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

pub(crate) fn bounds(
    profile: &crate::ConfiguredProviderProfile,
    wire: &TestProviderProfilePayload,
) -> Result<InventoryTransportLimits, StoreError> {
    let timeout_ms = wire.limits().timeout_ms().min(INVENTORY_MAX_TIMEOUT_MS);
    let limits = InventoryTransportLimits {
        connect_timeout_ms: profile
            .configuration
            .payload
            .limits()
            .connect_timeout_ms
            .min(timeout_ms),
        timeout_ms,
        max_response_bytes: INVENTORY_MAX_RESPONSE_BYTES,
    };
    limits.validate()?;
    Ok(limits)
}

/// Derives one explicit native route; a configured path is never silently lost.
pub(crate) fn request_endpoint(
    profile: &crate::ConfiguredProviderProfile,
) -> Result<String, StoreError> {
    let payload = &profile.configuration.payload;
    if payload.provider_kind() != ProviderKind::Ollama
        || payload.dialect().kind != ProviderDialectKind::OllamaChat
        || !matches!(payload.auth(), ProviderAuthentication::None)
    {
        return Err(StoreError::EvidenceUnavailable(
            "adaptateur inventory de ce profil non qualifié".to_owned(),
        ));
    }
    let destination = payload
        .endpoint_destination()
        .map_err(|_| invalid("destination inventory invalide"))?;
    if !destination.is_loopback_literal() {
        return Err(StoreError::EvidenceUnavailable(
            "inventory sans destination loopback littérale qualifiée".to_owned(),
        ));
    }
    let host = if destination.host().contains(':') {
        format!("[{}]", destination.host())
    } else {
        destination.host().to_owned()
    };
    let default_port = matches!(
        (destination.scheme(), destination.port()),
        ("http", 80) | ("https", 443)
    );
    let authority = if default_port {
        host
    } else {
        format!("{host}:{}", destination.port())
    };
    let origin = format!("{}://{authority}/", destination.scheme());
    if destination.normalized_endpoint() != origin {
        return Err(invalid(
            "inventory Ollama exige une origine exacte, sans chemin réinterprété",
        ));
    }
    Ok(format!("{origin}api/tags"))
}

pub(crate) fn request_digest(
    preparation: &transport::ProviderProbeConnectPreparation,
) -> Result<Sha256Digest, StoreError> {
    request_digest_for(&preparation.profile, &preparation.wire)
}

fn request_digest_for(
    profile: &crate::ConfiguredProviderProfile,
    wire: &TestProviderProfilePayload,
) -> Result<Sha256Digest, StoreError> {
    let limits = bounds(profile, wire)?;
    let configuration = &profile.configuration;
    Ok(unleash_protocol::canonical_sha256(&serde_json::json!({
        "schema":"unleash.ollama_inventory.v2", "method":"GET",
        "endpoint":request_endpoint(profile)?,
        "profile_id":configuration.profile_id,"profile_version":configuration.profile_version,
        "profile_digest":profile.configuration_digest,
        "connect_timeout_ms":limits.connect_timeout_ms,"timeout_ms":limits.timeout_ms,
        "max_response_bytes":limits.max_response_bytes,
        "request_body_bytes":0,"redirects":false,"retries":0,
    }))?)
}

pub(crate) fn validate_context_profile(
    context: &transport::ProviderProbeConnectContext,
    profile: &crate::ConfiguredProviderProfile,
) -> Result<(), StoreError> {
    // Reconstruct immutable request facts only, never caller authority or a claim.
    let wire = TestProviderProfilePayload::try_new(
        context.profile_id.to_string(),
        context.profile_version,
        context.phase,
        context.capability_set.clone(),
        context.limits.clone(),
        context.locality,
        context.mandate_ref.to_string(),
    )
    .map_err(|_| invalid("contexte inventory non qualifié"))?;
    if context.phase != unleash_protocol::ProviderTestPhase::Inventory
        || context.inventory_limits.as_ref() != Some(&bounds(profile, &wire)?)
        || context.request_digest != request_digest_for(profile, &wire)?
    {
        return Err(invalid(
            "bornes ou empreinte inventory divergentes du profil exact",
        ));
    }
    Ok(())
}

/// Read-only resolution, not a dispatch permission or a published observation.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderProbeInventoryPreparation {
    pub(crate) inner: transport::ProviderProbeConnectPreparation,
}

impl fmt::Debug for ProviderProbeInventoryPreparation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderProbeInventoryPreparation")
            .finish_non_exhaustive()
    }
}

/// Consultative projection. It cannot recreate an original execution owner.
#[derive(Debug, Clone)]
pub struct ProviderProbeInventoryRecord(transport::ProviderProbeConnectRecord);

impl ProviderProbeInventoryRecord {
    /// Exact, immutable transport context; inventory bounds are mandatory.
    #[must_use]
    pub const fn context(&self) -> &transport::ProviderProbeConnectContext {
        &self.0.context
    }
    /// CAS digest read with the context.
    #[must_use]
    pub const fn context_digest(&self) -> Sha256Digest {
        self.0.context_digest
    }
    /// Existing claim metadata grants no fresh execution authority.
    #[must_use]
    pub fn claim(&self) -> Option<&transport::ProviderProbeConnectClaimRecord> {
        self.0.claim.as_ref()
    }
}

/// Original authenticated call, never reconstructed from the row being claimed.
#[derive(Clone)]
pub struct ClaimProviderProbeInventoryRequest {
    /// Exact original wire, with its original digest.
    pub envelope: CommandEnvelope,
    /// Trusted session-derived caller.
    pub caller: CallerAuthority,
    /// Real receipt of this prepared probe.
    pub receipt_id: ReceiptId,
    /// Expected immutable context digest.
    pub context_digest: Sha256Digest,
    /// Control-plane incarnation, distinct from the V008 resource fence.
    pub control_fence: LineageFence,
}

impl fmt::Debug for ClaimProviderProbeInventoryRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClaimProviderProbeInventoryRequest")
            .field("receipt_id", &self.receipt_id)
            .finish_non_exhaustive()
    }
}

impl ClaimProviderProbeInventoryRequest {
    fn into_transport(self) -> transport::ClaimProviderProbeConnectRequest {
        transport::ClaimProviderProbeConnectRequest {
            envelope: self.envelope,
            caller: self.caller,
            receipt_id: self.receipt_id,
            context_digest: self.context_digest,
            control_fence: self.control_fence,
        }
    }
}

/// Opaque, non-clonable capability emitted only by an acknowledged inventory CAS.
#[derive(Debug)]
pub struct ProviderProbeInventoryClaim {
    inner: transport::ProviderProbeConnectClaim,
}

impl ProviderProbeInventoryClaim {
    /// Immutable context; a clone of these facts is not another capability.
    #[must_use]
    pub const fn context(&self) -> &transport::ProviderProbeConnectContext {
        self.inner.context()
    }
    /// Exact acknowledged context identity.
    #[must_use]
    pub const fn context_digest(&self) -> Sha256Digest {
        self.inner.context_digest()
    }
    /// Exact durable claim identity.
    #[must_use]
    pub const fn claim_id(&self) -> uuid::Uuid {
        self.inner.claim_id()
    }
    /// Exact durable claim revision.
    #[must_use]
    pub const fn claim_revision(&self) -> Revision {
        self.inner.claim_revision()
    }
    /// Configured network destination, not an inferred server identity.
    #[must_use]
    pub const fn destination(
        &self,
    ) -> &unleash_protocol::provider_profile::ProviderEndpointDestination {
        self.inner.destination()
    }

    pub(crate) fn permission(&self) -> InventoryPhasePermission {
        InventoryPhasePermission {
            inner: self.inner.permission(),
        }
    }
}

pub(crate) struct InventoryPhasePermission {
    inner: transport::ConnectPhasePermission,
}

impl InventoryPhasePermission {
    pub(crate) const fn inner(&self) -> &transport::ConnectPhasePermission {
        &self.inner
    }
}

/// A repeated claim is inspection, including after an uncertain acknowledgement.
#[derive(Debug)]
pub enum ProviderProbeInventoryClaimOutcome {
    /// The only fresh owner for this probe attempt.
    FreshClaim(ProviderProbeInventoryClaim),
    /// Existing immutable owner facts; no positive execution acknowledgement.
    AlreadyClaimed(ProviderProbeInventoryRecord),
}

impl UnleashStore {
    /// Revalidate the original claim before selecting the effective broker policy.
    ///
    /// # Errors
    ///
    /// Rejects an unavailable policy or a stale inventory claim.
    pub fn provider_probe_inventory_policy_snapshot(
        &self,
        claim: &ProviderProbeInventoryClaim,
    ) -> Result<crate::ProviderConnectPolicySnapshot, StoreError> {
        self.provider_probe_transport_policy_snapshot(
            claim.inner.permission(),
            TransportPhase::Inventory,
        )
    }

    /// Revalidates the exact admission before any inventory dispatch.
    ///
    /// # Errors
    ///
    /// Rejects stale authority, ownership or continuation.
    pub fn validate_provider_probe_inventory_continuation(
        &self,
        claim: &ProviderProbeInventoryClaim,
        continuation: crate::ExecutionContinuation,
    ) -> Result<(), StoreError> {
        let permission = claim.permission();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = crate::commands::read_daemon_state_conn(tx)?;
            let record = transport::validate_live_transport_permission(
                tx,
                &daemon,
                now,
                permission.inner(),
                TransportPhase::Inventory,
            )?;
            crate::provider_probe_connect_observation::validate_continuation(
                tx,
                &daemon,
                now,
                &record,
                &continuation,
                true,
            )
        })
    }

    /// Disposes an exact admission after a qualified pre-dispatch refusal.
    ///
    /// # Errors
    ///
    /// Rejects stale ownership or any durable dispatch or observation marker.
    pub fn dispose_provider_probe_inventory_without_dispatch(
        &self,
        claim: &ProviderProbeInventoryClaim,
        continuation: &crate::ExecutionContinuation,
    ) -> Result<crate::ProviderConnectDispositionCommit, StoreError> {
        self.dispose_provider_probe_transport_without_dispatch(
            claim.inner.permission(),
            continuation,
            TransportPhase::Inventory,
        )
    }

    /// Append only for the orchestrator retaining its original inventory claim.
    /// The generic journal never grants this access, including on exact replay.
    ///
    /// # Errors
    /// Rejects stale or divergent ownership, phase, mandate or V008 reservation.
    /// Observation stays closed until the specialized native proof is available.
    pub fn commit_provider_probe_inventory_phase(
        &self,
        claim: &ProviderProbeInventoryClaim,
        request: crate::BrokerPhaseRequest,
    ) -> Result<crate::BrokerPhaseCommit, StoreError> {
        let permission = claim.permission();
        let cutpoint = match &request.phase {
            crate::BrokerPhase::Authorizing { .. } => {
                Some(crate::StoreCutpoint::C3AfterBrokerIntentCommit)
            }
            crate::BrokerPhase::Dispatch { .. } => {
                Some(crate::StoreCutpoint::C4AfterBrokerDispatchCommit)
            }
            _ => None,
        };
        let config = self.config().clone();
        let committed = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            crate::broker_writer::commit_inventory_phase_tx(tx, &config, now, &request, &permission)
        })?;
        if let Some(cutpoint) = cutpoint {
            self.config().cutpoints.hit(cutpoint)?;
        }
        Ok(committed)
    }

    /// Resolve the exact authenticated inventory mandate/profile without effect.
    /// # Errors
    /// Rejects stale scope, identity, bytes, limits or unsupported native target.
    pub fn prepare_provider_probe_inventory(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
    ) -> Result<ProviderProbeInventoryPreparation, StoreError> {
        let envelope = envelope.clone();
        let caller = caller.clone();
        let inner = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = crate::commands::read_daemon_state_conn(tx)?;
            transport::prepare_transport_tx(
                tx,
                &daemon,
                now,
                &envelope,
                &caller,
                TransportPhase::Inventory,
            )
        })?;
        crate::provider_profiles::verify_configuration_objects(
            self,
            &inner.profile.configuration,
            inner.profile.configuration_digest,
        )?;
        Ok(ProviderProbeInventoryPreparation { inner })
    }

    /// Read an exact inventory context and its watermark without minting a claim.
    /// # Errors
    /// Rejects inconsistent context bytes, phase or digest.
    pub fn inspect_provider_probe_inventory_snapshot(
        &self,
        probe: ProviderProbeId,
    ) -> Result<SnapshotProjection<Option<ProviderProbeInventoryRecord>>, StoreError> {
        self.project_snapshot(|conn| {
            Ok(
                transport::read_transport_context(conn, probe, TransportPhase::Inventory)?
                    .map(ProviderProbeInventoryRecord),
            )
        })
    }

    /// Claim once under the original authenticated call and current incarnation.
    /// # Errors
    /// Rejects a different actor, receipt, context, phase, fence or stale identity.
    pub fn claim_provider_probe_inventory(
        &self,
        request: ClaimProviderProbeInventoryRequest,
    ) -> Result<ProviderProbeInventoryClaimOutcome, StoreError> {
        let request = request.into_transport();
        let config = self.config().clone();
        let outcome = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            transport::claim_transport_tx(tx, &config, now, &request, TransportPhase::Inventory)
        })?;
        Ok(match outcome {
            transport::ProviderProbeConnectClaimOutcome::FreshClaim(inner) => {
                ProviderProbeInventoryClaimOutcome::FreshClaim(ProviderProbeInventoryClaim {
                    inner,
                })
            }
            transport::ProviderProbeConnectClaimOutcome::AlreadyClaimed(record) => {
                ProviderProbeInventoryClaimOutcome::AlreadyClaimed(ProviderProbeInventoryRecord(
                    record,
                ))
            }
        })
    }
}
