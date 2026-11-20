//! Phase-specific capabilities retained by the shared transport orchestrator.

use unleash_domain::{Fence, LineageFence, ProviderProbeId, ReceiptId, Sha256Digest};
use unleash_protocol::{CommandEnvelope, provider_profile::ProviderEndpointDestination};
use unleash_store::{
    AdoptedProviderConnectObservation, AdoptedProviderInventoryObservation, BrokerPhaseCommit,
    BrokerPhaseRequest, CallerAuthority, ClaimProviderProbeConnectRequest,
    ClaimProviderProbeInventoryRequest, CommitProviderConnectObservation,
    CommitProviderInventoryObservation, ExecutionContinuation, ProviderConnectDispositionCommit,
    ProviderConnectPolicySnapshot, ProviderProbeConnectClaim, ProviderProbeConnectClaimOutcome,
    ProviderProbeConnectContext, ProviderProbeInventoryClaim, ProviderProbeInventoryClaimOutcome,
    StoreError, UnleashStore,
};

pub(super) enum TransportOwner {
    Connect(ProviderProbeConnectClaim),
    Inventory(ProviderProbeInventoryClaim),
}

#[derive(Clone)]
pub(super) enum AdoptedFact {
    Connect(AdoptedProviderConnectObservation),
    Inventory(AdoptedProviderInventoryObservation),
}

impl TransportOwner {
    pub(super) fn claim(
        store: &UnleashStore,
        command: CommandEnvelope,
        caller: CallerAuthority,
        receipt_id: ReceiptId,
    ) -> Result<Option<Self>, StoreError> {
        let probe = ProviderProbeId::from_uuid(command.aggregate_id);
        let fence = |context: &ProviderProbeConnectContext| {
            LineageFence::new(
                context.authority_generation,
                context.daemon_epoch,
                Fence::ZERO,
            )
        };
        match command
            .payload
            .get("phase")
            .and_then(serde_json::Value::as_str)
        {
            Some("connect") => {
                let record = store
                    .inspect_provider_probe_connect_snapshot(probe)?
                    .value
                    .ok_or_else(|| StoreError::NotFound("probe connect".into()))?;
                if record.claim.is_some() {
                    return Ok(None);
                }
                let result =
                    store.claim_provider_probe_connect(ClaimProviderProbeConnectRequest {
                        envelope: command,
                        caller,
                        receipt_id,
                        context_digest: record.context_digest,
                        control_fence: fence(&record.context),
                    })?;
                Ok(match result {
                    ProviderProbeConnectClaimOutcome::FreshClaim(owner) => {
                        Some(Self::Connect(owner))
                    }
                    ProviderProbeConnectClaimOutcome::AlreadyClaimed(_) => None,
                })
            }
            Some("inventory") => {
                let record = store
                    .inspect_provider_probe_inventory_snapshot(probe)?
                    .value
                    .ok_or_else(|| StoreError::NotFound("probe inventory".into()))?;
                if record.claim().is_some() {
                    return Ok(None);
                }
                let result =
                    store.claim_provider_probe_inventory(ClaimProviderProbeInventoryRequest {
                        envelope: command,
                        caller,
                        receipt_id,
                        context_digest: record.context_digest(),
                        control_fence: fence(record.context()),
                    })?;
                Ok(match result {
                    ProviderProbeInventoryClaimOutcome::FreshClaim(owner) => {
                        Some(Self::Inventory(owner))
                    }
                    ProviderProbeInventoryClaimOutcome::AlreadyClaimed(_) => None,
                })
            }
            _ => Err(StoreError::EvidenceUnavailable(
                "phase de transport absente".into(),
            )),
        }
    }

    pub(super) fn context(&self) -> &ProviderProbeConnectContext {
        match self {
            Self::Connect(owner) => owner.context(),
            Self::Inventory(owner) => owner.context(),
        }
    }

    pub(super) fn destination(&self) -> &ProviderEndpointDestination {
        match self {
            Self::Connect(owner) => owner.destination(),
            Self::Inventory(owner) => owner.destination(),
        }
    }

    pub(super) fn claim_id(&self) -> uuid::Uuid {
        match self {
            Self::Connect(owner) => owner.claim_id(),
            Self::Inventory(owner) => owner.claim_id(),
        }
    }

    pub(super) fn context_digest(&self) -> Sha256Digest {
        match self {
            Self::Connect(owner) => owner.context_digest(),
            Self::Inventory(owner) => owner.context_digest(),
        }
    }

    pub(super) fn verb(&self) -> &'static str {
        match self {
            Self::Connect(_) => unleash_model_router::connect_transport::CONNECT_VERB,
            Self::Inventory(_) => unleash_model_router::inventory_transport::INVENTORY_VERB,
        }
    }

    pub(super) fn policy(
        &self,
        store: &UnleashStore,
    ) -> Result<ProviderConnectPolicySnapshot, StoreError> {
        match self {
            Self::Connect(owner) => store.provider_probe_connect_policy_snapshot(owner),
            Self::Inventory(owner) => store.provider_probe_inventory_policy_snapshot(owner),
        }
    }

    pub(super) fn validate_continuation(
        &self,
        store: &UnleashStore,
        continuation: ExecutionContinuation,
    ) -> Result<(), StoreError> {
        match self {
            Self::Connect(owner) => {
                store.validate_provider_probe_connect_continuation(owner, continuation)
            }
            Self::Inventory(owner) => {
                store.validate_provider_probe_inventory_continuation(owner, continuation)
            }
        }
    }

    pub(super) fn dispose_without_dispatch(
        &self,
        store: &UnleashStore,
        continuation: &ExecutionContinuation,
    ) -> Result<ProviderConnectDispositionCommit, StoreError> {
        match self {
            Self::Connect(owner) => {
                store.dispose_provider_probe_connect_without_dispatch(owner, continuation)
            }
            Self::Inventory(owner) => {
                store.dispose_provider_probe_inventory_without_dispatch(owner, continuation)
            }
        }
    }

    pub(super) fn commit_phase(
        &self,
        store: &UnleashStore,
        request: BrokerPhaseRequest,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        match self {
            Self::Connect(owner) => store.commit_provider_probe_connect_phase(owner, request),
            Self::Inventory(owner) => store.commit_provider_probe_inventory_phase(owner, request),
        }
    }

    pub(super) fn commit_observation(
        &self,
        store: &UnleashStore,
        request: BrokerPhaseRequest,
        continuation: ExecutionContinuation,
        mut fact: Option<AdoptedFact>,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        if matches!(&request.phase, unleash_store::BrokerPhase::Observation { receipt, .. }
            if receipt.state == unleash_domain::ActionState::OutcomeUnknown)
        {
            fact = None;
        }
        match (self, fact) {
            (Self::Connect(owner), Some(AdoptedFact::Connect(fact))) => store
                .commit_provider_probe_connect_observation(
                    owner,
                    CommitProviderConnectObservation {
                        continuation,
                        fact: Some(fact),
                        request,
                    },
                ),
            (Self::Connect(owner), None) => store.commit_provider_probe_connect_observation(
                owner,
                CommitProviderConnectObservation {
                    continuation,
                    fact: None,
                    request,
                },
            ),
            (Self::Inventory(owner), Some(AdoptedFact::Inventory(fact))) => store
                .commit_provider_probe_inventory_observation(
                    owner,
                    CommitProviderInventoryObservation {
                        continuation,
                        fact: Some(fact),
                        request,
                    },
                ),
            (Self::Inventory(owner), None) => store.commit_provider_probe_inventory_observation(
                owner,
                CommitProviderInventoryObservation {
                    continuation,
                    fact: None,
                    request,
                },
            ),
            _ => Err(StoreError::EvidenceUnavailable(
                "fait d’une autre phase de transport".into(),
            )),
        }
    }
}
