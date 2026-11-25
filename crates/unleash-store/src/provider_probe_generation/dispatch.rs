//! Claim one transport permit under the exact current managed admission.

use std::{
    net::{IpAddr, SocketAddr},
    sync::atomic::Ordering,
};

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use unleash_domain::{ActionId, ActionState, DispatchId, Sha256Digest, TimestampMs};
use unleash_protocol::provider_probe::{
    OllamaChatPlacement, OllamaChatRequestBinding, admitted_ollama_chat_digest,
    owned_loopback_request_digest,
};

use super::{placement::admitted_chat_inputs_at_state, unavailable};
use crate::{
    AdmissionCommit, AttestedProviderServiceLaunch, BrokerObservationCause, BrokerPhase,
    BrokerPhaseCommit, BrokerPhaseRequest, ConfiguredProviderProfile, PreparedProviderProbe,
    ProviderServiceBackend, StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
    broker_writer::ManagedGenerationObservationAccess,
};

pub(crate) const MANAGED_GENERATION_ADAPTER: &str = "unleash-ollama-chat";

/// Native configuration the executor must match when it freezes the admitted bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedGenerationDispatchBinding {
    pub(crate) action_id: ActionId,
    pub(crate) transport_digest: Sha256Digest,
    admission_digest: Sha256Digest,
    placement_digest: Sha256Digest,
    body_digest: Sha256Digest,
}

/// Closed native fact of one managed generation exchange, published by the router transport
/// after the socket is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedGenerationObservation {
    /// Document version.
    pub schema_version: u32,
    /// Action owning the exchange.
    pub action_id: ActionId,
    /// Transport dispatch actually executed.
    pub dispatch_id: DispatchId,
    /// Authorized native request, as recorded by the durable dispatch marker.
    pub request_digest: Sha256Digest,
    /// Bounded HTTP body actually received.
    pub response_digest: Sha256Digest,
    /// Canonical digest of the observed outcome; the receipt's `output_digest`.
    pub outcome_digest: Sha256Digest,
    /// Native observation time, after the socket is closed.
    pub observed_at_ms: TimestampMs,
}

/// Single-owner handoff of the durable transport marker.
#[must_use = "the transport executor must retain ownership of this marker"]
pub struct ManagedGenerationDispatchPermit {
    commit: BrokerPhaseCommit,
    dispatch_id: DispatchId,
    transport_digest: Sha256Digest,
    expires_at: TimestampMs,
}

impl std::fmt::Debug for ManagedGenerationDispatchPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedGenerationDispatchPermit")
            .field("dispatch_id", &self.dispatch_id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
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

    /// Commit the observation of a managed generation through its dedicated port.
    ///
    /// The permit is the one of the executed transport. An observed success requires the exact
    /// native fact of that dispatch, bound to its durable marker; an unknown outcome never
    /// carries one. An exact replay returns the original cursor. This observation alone releases
    /// no reservation.
    ///
    /// # Errors
    /// Refuses a permit, receipt or native fact that differs from the durable dispatch, and any
    /// issue other than an observed success or an unknown outcome.
    pub fn commit_managed_generation_observation(
        &self,
        permit: &ManagedGenerationDispatchPermit,
        request: BrokerPhaseRequest,
        observation: Option<ManagedGenerationObservation>,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let BrokerPhase::Observation {
                receipt,
                cause,
                reason,
                ..
            } = &request.phase
            else {
                return Err(unavailable(
                    "observation de génération managée attendue",
                ));
            };
            let action = request.phase.action_id();
            let dispatch = permit.dispatch_id();
            let (adapter, stored, started_at): (String, String, i64) = tx
                .query_row(
                    "SELECT d.adapter_id, e.payload_json, d.started_at_ms
                     FROM broker_dispatches b
                     JOIN action_dispatches d ON d.dispatch_id=b.dispatch_id AND d.action_id=b.action_id
                     JOIN broker_phase_records r ON r.action_id=b.action_id
                      AND r.evaluation_id=b.evaluation_id AND r.phase_key='dispatch'
                     JOIN events e ON e.event_id=r.event_id AND e.payload_state='full'
                     WHERE b.dispatch_id=?1 AND b.action_id=?2 AND b.evaluation_id=?3",
                    params![
                        dispatch.to_string(),
                        action.to_string(),
                        request.evaluation.evaluation_id.to_string(),
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?
                .ok_or_else(|| unavailable("marqueur de génération managée indisponible"))?;
            if adapter != MANAGED_GENERATION_ADAPTER
                || receipt.dispatch_id != dispatch
                || *cause != BrokerObservationCause::Execution
                || reason.is_some()
            {
                return Err(unavailable(
                    "observation de génération managée divergente",
                ));
            }
            let stored: serde_json::Value = serde_json::from_str(&stored)?;
            let binding: ManagedGenerationDispatchBinding = stored
                .get("managed_generation")
                .map(serde_json::from_value)
                .transpose()?
                .ok_or_else(|| unavailable("lien natif du dispatch de génération absent"))?;
            if binding.action_id != action
                || binding.transport_digest != permit.transport_digest()
            {
                return Err(unavailable(
                    "permis de génération divergent du marqueur durable",
                ));
            }
            match (receipt.state, observation.as_ref()) {
                (ActionState::ObservedSuccess, Some(native)) => {
                    if native.schema_version != 1
                        || native.action_id != action
                        || native.dispatch_id != dispatch
                        || native.request_digest != binding.transport_digest
                        || receipt.output_digest != Some(native.outcome_digest)
                        || (!config.clock_uncertain.load(Ordering::Acquire)
                            && native.observed_at_ms.as_i64() < started_at)
                    {
                        return Err(unavailable("fait natif de génération divergent"));
                    }
                }
                (ActionState::OutcomeUnknown, None) => {
                    if receipt.output_digest.is_some() {
                        return Err(unavailable(
                            "issue inconnue de génération sans empreinte",
                        ));
                    }
                }
                _ => {
                    return Err(unavailable(
                        "observation de génération sans issue qualifiée",
                    ));
                }
            }
            crate::broker_writer::commit_managed_generation_observation_tx(
                tx,
                &config,
                now,
                &request,
                ManagedGenerationObservationAccess {
                    dispatch_id: dispatch,
                    request_digest: binding.transport_digest,
                    native: observation.as_ref(),
                },
            )
        })
    }
}

/// Admit an `Observation` phase only through the port dedicated to a managed generation.
///
/// A managed transport dispatch and the dedicated port are bound to each other: the generic
/// writer stays closed to that dispatch, and the dedicated port serves no other dispatch.
pub(crate) fn reject_generic_observation(
    tx: &Transaction<'_>,
    action: ActionId,
    dedicated_port: bool,
) -> Result<(), StoreError> {
    let managed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM action_dispatches WHERE action_id=?1 AND adapter_id=?2)",
        params![action.to_string(), MANAGED_GENERATION_ADAPTER],
        |row| row.get(0),
    )?;
    if managed == dedicated_port {
        return Ok(());
    }
    Err(if managed {
        unavailable("observation de génération exige son port dédié")
    } else {
        unavailable("port d'observation dédié réservé à la génération managée")
    })
}
