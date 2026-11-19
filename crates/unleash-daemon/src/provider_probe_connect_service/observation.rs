//! Pending Store adapter for the required local transport publication port.
//!
//! This file is deliberately not declared by the service until its real writer
//! backend and the daemon's model-router dependency are connected. No fallback
//! backend or acknowledgement in memory is provided for the product path.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::executors::BrokerActivityGuard;
use unleash_action_broker::BoxFuture;
use unleash_domain::{
    ActionId, ActorId, LineageFence, ProviderProbeId, ReceiptId, Revision, Sha256Digest,
};
use unleash_model_router::connect_transport::{
    CertificateValidation, ConnectError, ConnectObservation, ConnectObservationPort,
};
use unleash_protocol::{canonical_sha256, canonicalize_to_vec};
use unleash_store::objects::PublishedObjectReference;
use unleash_store::{
    AdmissionUnit, ExecutionContinuation, PrepareObjectRequest, PublishObjectReferenceRequest,
    StoreError, UnleashStore,
};

const PREPARATION_OWNER: &str = "provider_probe_connect_observation_preparation";

#[derive(Clone, Copy)]
pub(super) enum TransportKind {
    Tcp,
    OfflineTls,
}

#[derive(Clone, Copy)]
pub(super) enum PeerBinding {
    Literal {
        address: SocketAddr,
        transport: TransportKind,
    },
    LocalhostTls {
        port: u16,
    },
}

impl PeerBinding {
    fn matches(self, document: &ConnectObservation) -> bool {
        let transport = match self {
            Self::Literal { address, transport } => {
                if document.peer != address {
                    return false;
                }
                transport
            }
            Self::LocalhostTls { port } => {
                if document.peer.port() != port || !document.peer.ip().is_loopback() {
                    return false;
                }
                TransportKind::OfflineTls
            }
        };
        matches!(
            (transport, document.certificate_validation),
            (TransportKind::Tcp, CertificateValidation::NotTls)
                | (
                    TransportKind::OfflineTls,
                    CertificateValidation::OfflineSystemRoots
                )
        )
    }
}

/// Facts to obtain from a newly acknowledged claim and real V008 admission.
/// The control-plane fence and the resource lease fence remain separate.
#[derive(Clone)]
pub(super) struct ObservationBinding {
    pub(super) probe_id: ProviderProbeId,
    pub(super) receipt_id: ReceiptId,
    pub(super) actor_id: ActorId,
    pub(super) principal_id: ActorId,
    pub(super) observer_principal_id: ActorId,
    pub(super) action_id: ActionId,
    pub(super) claim_revision: Revision,
    pub(super) context_digest: Sha256Digest,
    pub(super) request_digest: Sha256Digest,
    pub(super) control_fence: LineageFence,
    pub(super) peer: PeerBinding,
    pub(super) continuation: ExecutionContinuation,
}

impl fmt::Debug for ObservationBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservationBinding")
            .field("probe_id", &self.probe_id)
            .field("action_id", &self.action_id)
            .finish_non_exhaustive()
    }
}

/// Implement only by the real Store adapter, with no network or secret access.
/// The writer must validate the active claim, actual marker, lineage, exact lease
/// and document hash, frozen peer/port/TLS and observation date, and adopt a
/// permanent reference before acknowledging. Scope/claim checks in an earlier
/// snapshot never replace this transaction's checks.
pub(super) trait ObservationWriter: Send + Sync {
    fn commit_local_observation(
        &self,
        binding: &ObservationBinding,
        document: &ConnectObservation,
        content_hash: Sha256Digest,
        candidate: &PublishedObjectReference,
    ) -> Result<(), StoreError>;
}

pub(super) struct ObservationAdapter {
    store: Arc<UnleashStore>,
    writer: Arc<dyn ObservationWriter>,
    binding: Arc<ObservationBinding>,
    activity: Arc<BrokerActivityGuard>,
}

impl ObservationAdapter {
    pub(super) fn new(
        store: Arc<UnleashStore>,
        writer: Arc<dyn ObservationWriter>,
        binding: ObservationBinding,
        activity: Arc<BrokerActivityGuard>,
    ) -> Result<Self, StoreError> {
        if binding.continuation.unit != AdmissionUnit::Probe(binding.probe_id)
            || binding.continuation.authority_generation
                != binding.control_fence.authority_generation
            || binding.continuation.daemon_epoch != binding.control_fence.epoch
            || binding.continuation.fence.as_u64() == 0
        {
            return Err(invalid("liaison d'admission connect incohérente"));
        }
        Ok(Self {
            store,
            writer,
            binding: Arc::new(binding),
            activity,
        })
    }
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn persist_blocking(
    store: &UnleashStore,
    writer: &dyn ObservationWriter,
    binding: &ObservationBinding,
    document: &ConnectObservation,
) -> Result<(), StoreError> {
    if document.schema_version != 1
        || document.action_id != binding.action_id
        || document.request_digest != binding.request_digest
        || document.http_application_bytes_sent != 0
        || document.revocation_checked
        || !document.peer.ip().is_loopback()
        || !binding.peer.matches(document)
    {
        return Err(invalid("document d'observation connect incohérent"));
    }
    let bytes = canonicalize_to_vec(document)?;
    let content_hash = canonical_sha256(document)?;
    let owner = store.config().id_generator.new_uuid().to_string();
    let handle = store.prepare_object(PrepareObjectRequest {
        object_hash: content_hash,
        byte_length: u64::try_from(bytes.len())
            .map_err(|_| invalid("document connect trop grand"))?,
        media_type: "application/json".to_owned(),
        owner_kind: PREPARATION_OWNER.to_owned(),
        owner_id: owner.clone(),
    })?;
    let publication = (|| {
        store.write_prepared_object_bytes(&handle, &bytes)?;
        store.publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id.clone(),
            object_hash: content_hash,
            object_generation: handle.object_generation,
            owner_kind: PREPARATION_OWNER.to_owned(),
            owner_id: owner,
            source_field: "observation".to_owned(),
        })
    })();
    let candidate = match publication {
        Ok(candidate) => candidate,
        Err(error) => {
            // This synchronous writer has returned. Only its own preparation
            // pin is released; another producer of the same hash is untouched.
            store.release_pin(&handle.pin_id)?;
            return Err(error);
        }
    };
    let committed = writer.commit_local_observation(binding, document, content_hash, &candidate);
    // A successful writer transaction has adopted a separate permanent reference.
    // After an uncertain commit, retiring this one candidate cannot remove it.
    let retired =
        store.retire_owned_object_reference(candidate, "connect_observation_candidate_finished");
    committed?;
    retired?;
    Ok(())
}

impl ConnectObservationPort for ObservationAdapter {
    fn persist<'a>(
        &'a self,
        document: &'a ConnectObservation,
    ) -> BoxFuture<'a, Result<(), ConnectError>> {
        let store = Arc::clone(&self.store);
        let writer = Arc::clone(&self.writer);
        let binding = Arc::clone(&self.binding);
        let activity = Arc::clone(&self.activity);
        let document = document.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let _activity = activity;
                persist_blocking(&store, writer.as_ref(), &binding, &document)
            })
            .await
            .map_err(|_| ConnectError::ObservationUnavailable)?
            .map_err(|_| ConnectError::ObservationUnavailable)
        })
    }
}
