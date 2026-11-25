//! Claim one transport permit under the exact current managed admission.

use std::{
    net::{IpAddr, SocketAddr},
    sync::atomic::Ordering,
};

use rusqlite::Transaction;
use serde::Serialize;
use serde_json::json;
use unleash_domain::{ActionId, ActionState, DispatchId, Sha256Digest, TimestampMs};
use unleash_protocol::provider_probe::{
    OllamaChatPlacement, OllamaChatRequestBinding, admitted_ollama_chat_digest,
    owned_loopback_request_digest,
};

use super::{placement::admitted_chat_inputs_at_state, unavailable};
use crate::{
    AdmissionCommit, AttestedProviderServiceLaunch, BrokerPhase, BrokerPhaseCommit,
    BrokerPhaseRequest, ConfiguredProviderProfile, PreparedProviderProbe, ProviderServiceBackend,
    StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
};

pub(crate) const MANAGED_GENERATION_ADAPTER: &str = "unleash-ollama-chat";

/// Native configuration the executor must match when it freezes the admitted bytes.
pub struct ManagedOllamaTransport {
    /// Fingerprint of the native request, owned peer, placement and admission.
    pub request_digest: Sha256Digest,
    /// Fingerprint of the exact resolved profile configuration.
    pub profile_digest: Sha256Digest,
    /// Connection timeout in milliseconds.
    pub connect_ms: u64,
    /// Total exchange timeout in milliseconds.
    pub total_ms: u64,
}

pub(super) fn derive_transport(
    probe: &PreparedProviderProbe,
    profile: &ConfiguredProviderProfile,
    mut placement: OllamaChatPlacement,
    launch: &AttestedProviderServiceLaunch,
) -> Result<(OllamaChatPlacement, ManagedOllamaTransport), StoreError> {
    let destination = profile
        .configuration
        .payload
        .endpoint_destination()
        .map_err(|_| unavailable("destination native Ollama invalide"))?;
    if destination.normalized_endpoint() != placement.endpoint {
        return Err(unavailable("destination de placement Ollama divergente"));
    }
    placement.endpoint = destination
        .ollama_chat_endpoint()
        .ok_or_else(|| unavailable("destination native Ollama non qualifiée"))?;
    let ip: IpAddr = destination
        .host()
        .parse()
        .map_err(|_| unavailable("destination native Ollama non numérique"))?;
    let preparation = &probe.context.preparation;
    let total_ms = preparation.limits.timeout_ms;
    let connect_ms = profile
        .configuration
        .payload
        .limits()
        .connect_timeout_ms
        .min(total_ms);
    let binding = OllamaChatRequestBinding {
        endpoint: &placement.endpoint,
        profile_id: &preparation.requested_profile_id,
        profile_version: preparation.requested_profile_version,
        profile_digest: profile.configuration_digest,
        connect_ms,
        total_ms,
        body_digest: probe.payload_digest(),
    };
    let identity = &launch.attestation.child;
    let backend = match launch.context.plan.backend {
        ProviderServiceBackend::MacosSeatbeltStrictServiceV1 => "macos_proc_pidinfo",
    };
    let owned = owned_loopback_request_digest(
        binding.digest()?,
        SocketAddr::new(ip, destination.port()),
        &json!({
            "pid": identity.pid,
            "pgid": identity.pgid,
            "owner_uid": identity.owner_uid,
            "boot_identity": identity.boot_identity,
            "start_identity": identity.start_identity,
            "backend": backend,
        }),
    )?;
    let transport = ManagedOllamaTransport {
        request_digest: admitted_ollama_chat_digest(owned, &placement)?,
        profile_digest: profile.configuration_digest,
        connect_ms,
        total_ms,
    };
    Ok((placement, transport))
}

#[derive(Serialize)]
pub(crate) struct ManagedGenerationDispatchBinding {
    pub(crate) action_id: ActionId,
    transport_digest: Sha256Digest,
    admission_digest: Sha256Digest,
    placement_digest: Sha256Digest,
    body_digest: Sha256Digest,
}

/// Single-owner handoff of the durable transport marker.
#[must_use = "the transport executor must retain ownership of this marker"]
pub struct ManagedGenerationDispatchPermit {
    commit: BrokerPhaseCommit,
    dispatch_id: DispatchId,
    transport_digest: Sha256Digest,
    expires_at: TimestampMs,
}

impl ManagedGenerationDispatchPermit {
    /// Cursor and action of the durable dispatch commit.
    #[must_use]
    pub const fn commit(&self) -> &BrokerPhaseCommit {
        &self.commit
    }

    /// Exact transport dispatch recorded by the writer.
    #[must_use]
    pub const fn dispatch_id(&self) -> DispatchId {
        self.dispatch_id
    }

    /// Exact authorized native request.
    #[must_use]
    pub const fn transport_digest(&self) -> Sha256Digest {
        self.transport_digest
    }

    /// Earliest expiry of the transport inputs or the broker decision.
    #[must_use]
    pub const fn expires_at(&self) -> TimestampMs {
        self.expires_at
    }
}

impl UnleashStore {
    /// Commit the native transport once for the already admitted and authorized probe.
    ///
    /// # Errors
    /// Refuses stale or divergent admission, authorization or transport and any second claim.
    pub fn commit_managed_generation_dispatch(
        &self,
        admission: &AdmissionCommit,
        request: BrokerPhaseRequest,
        transport_digest: Sha256Digest,
    ) -> Result<ManagedGenerationDispatchPermit, StoreError> {
        let admission = admission.clone();
        let config = self.config().clone();
        let permit = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let BrokerPhase::Dispatch {
                action_id,
                decision,
                dispatch_id,
                ..
            } = &request.phase
            else {
                return Err(unavailable("phase de transport génération invalide"));
            };
            let inputs = admitted_chat_inputs_at_state(
                tx,
                &config,
                now,
                &admission,
                true,
                ActionState::Authorized,
            )?;
            let transport = inputs
                .managed_transport
                .as_ref()
                .ok_or_else(|| unavailable("transport génération non lié"))?;
            if *action_id != inputs.probe.context.action_id
                || transport.request_digest != transport_digest
            {
                return Err(unavailable("requête native génération divergente"));
            }
            let binding = ManagedGenerationDispatchBinding {
                action_id: *action_id,
                transport_digest,
                admission_digest: inputs.transport_placement.admission_digest,
                placement_digest: inputs.transport_placement.evidence_digest,
                body_digest: inputs.probe.payload_digest(),
            };
            let until = inputs.transport_placement.valid_until_ms;
            let expires_at = decision.expiry.map_or(until, |expiry| expiry.min(until));
            let dispatch_at = config.clock.now_ms();
            if dispatch_at < now
                || dispatch_at >= expires_at
                || config.clock_uncertain.load(Ordering::Acquire)
            {
                return Err(unavailable(
                    "requête native génération expirée avant dispatch",
                ));
            }
            let commit = crate::broker_writer::commit_managed_generation_dispatch_tx(
                tx,
                &config,
                dispatch_at,
                &request,
                &binding,
            )?;
            Ok(ManagedGenerationDispatchPermit {
                commit,
                dispatch_id: *dispatch_id,
                transport_digest,
                expires_at,
            })
        })?;
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C4AfterBrokerDispatchCommit)?;
        Ok(permit)
    }
}

pub(crate) fn reject_generic_observation(
    tx: &Transaction<'_>,
    action: ActionId,
) -> Result<(), StoreError> {
    let managed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM action_dispatches WHERE action_id=?1 AND adapter_id=?2)",
        rusqlite::params![action.to_string(), MANAGED_GENERATION_ADAPTER],
        |row| row.get(0),
    )?;
    if managed {
        return Err(unavailable(
            "observation de génération exige son port dédié",
        ));
    }
    Ok(())
}
