//! Native transport adoption after channel closure, before broker completion.

use super::admission::FactSlot;
use crate::broker_activity::BrokerActivityGuard;
use std::sync::{Arc, Mutex};
use unleash_action_broker::BoxFuture;
use unleash_domain::Sha256Digest;
use unleash_model_router::connect_transport::{
    CertificateValidation, ConnectError, ConnectObservation, ConnectObservationPort,
};
use unleash_protocol::{canonical_sha256, canonicalize_to_vec};
use unleash_store::{
    AdoptProviderConnectObservation, ConnectCertificateValidation, ExecutionContinuation,
    PrepareObjectRequest, ProviderConnectObservation, ProviderProbeConnectClaim,
    PublishObjectReferenceRequest, StoreError, UnleashStore,
};

pub(super) struct ObservationAdapter {
    pub(super) store: Arc<UnleashStore>,
    pub(super) claim: Arc<ProviderProbeConnectClaim>,
    pub(super) continuation: Arc<Mutex<Option<ExecutionContinuation>>>,
    pub(super) fact: Arc<FactSlot>,
    pub(super) activity: Arc<BrokerActivityGuard>,
}

fn unavailable() -> StoreError {
    StoreError::EvidenceUnavailable("observation connect indisponible".into())
}

fn native_document(document: &ConnectObservation) -> ProviderConnectObservation {
    ProviderConnectObservation {
        schema_version: document.schema_version,
        action_id: document.action_id,
        dispatch_id: document.dispatch_id,
        request_digest: document.request_digest,
        peer: document.peer,
        certificate_validation: match document.certificate_validation {
            CertificateValidation::NotTls => ConnectCertificateValidation::NotTls,
            CertificateValidation::OfflineSystemRoots => {
                ConnectCertificateValidation::OfflineSystemRoots
            }
        },
        revocation_checked: document.revocation_checked,
        http_application_bytes_sent: document.http_application_bytes_sent,
        observed_at_ms: document.observed_at_ms,
    }
}

fn publish_candidate<T: serde::Serialize>(
    store: &UnleashStore,
    document: &T,
    kind: &str,
    field: &str,
) -> Result<
    (
        Sha256Digest,
        unleash_store::objects::PublishedObjectReference,
    ),
    StoreError,
> {
    let bytes = canonicalize_to_vec(document)?;
    let hash = canonical_sha256(document)?;
    let owner = store.config().id_generator.new_uuid().to_string();
    let handle = store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: u64::try_from(bytes.len()).map_err(|_| unavailable())?,
        media_type: "application/json".into(),
        owner_kind: kind.into(),
        owner_id: owner.clone(),
    })?;
    let result = (|| {
        store.write_prepared_object_bytes(&handle, &bytes)?;
        store.publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id.clone(),
            object_hash: hash,
            object_generation: handle.object_generation,
            owner_kind: kind.into(),
            owner_id: owner,
            source_field: field.into(),
        })
    })();
    match result {
        Ok(reference) => Ok((hash, reference)),
        Err(error) => {
            store.release_pin(&handle.pin_id)?;
            Err(error)
        }
    }
}

pub(super) fn publish_document<T: serde::Serialize>(
    store: &UnleashStore,
    document: &T,
    kind: &str,
    field: &str,
) -> Result<Sha256Digest, StoreError> {
    // The precise producer reference remains live while the eligibility evidence
    // is consumable; it is never retired by hash or confused with native proof.
    publish_candidate(store, document, kind, field).map(|(hash, _)| hash)
}

impl ConnectObservationPort for ObservationAdapter {
    fn persist<'a>(
        &'a self,
        document: &'a ConnectObservation,
    ) -> BoxFuture<'a, Result<(), ConnectError>> {
        let store = Arc::clone(&self.store);
        let claim = Arc::clone(&self.claim);
        let continuation = Arc::clone(&self.continuation);
        let fact = Arc::clone(&self.fact);
        let activity = Arc::clone(&self.activity);
        let document = native_document(document);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let _activity = activity;
                let continuation = continuation
                    .lock()
                    .map_err(|_| unavailable())?
                    .clone()
                    .ok_or_else(unavailable)?;
                let (hash, candidate) = publish_candidate(
                    &store,
                    &document,
                    "provider_probe_connect_observation_preparation",
                    "observation",
                )?;
                let result = store.adopt_provider_probe_connect_observation(
                    &claim,
                    AdoptProviderConnectObservation {
                        continuation,
                        document,
                        content_hash: hash,
                        candidate: candidate.clone(),
                    },
                );
                // On lost adoption ACK, this exact candidate may be retired; a
                // separate permanent ref already adopted is never touched.
                let retired = store.retire_owned_object_reference(
                    candidate,
                    "connect_observation_candidate_finished",
                );
                let adopted = result?;
                retired?;
                *fact.lock().map_err(|_| unavailable())? = Some(adopted);
                if store
                    .config()
                    .clock_uncertain
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    return Err(unavailable());
                }
                Ok(())
            })
            .await
            .map_err(|_| ConnectError::ObservationUnavailable)?
            .map_err(|_: StoreError| ConnectError::ObservationUnavailable)
        })
    }
}
