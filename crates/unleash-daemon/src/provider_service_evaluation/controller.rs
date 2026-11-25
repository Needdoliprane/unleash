//! Keep the direct process-host handle before recording its result.

use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, atomic::Ordering},
    time::{Duration, Instant},
};

use super::{
    OwnedProviderServiceHost, ProviderServiceLaunchError, ProviderServiceLaunchRefused,
    spawn_provider_service_host,
};
use crate::executors::{ExecutorRegistration, ExecutorState, RendezvousExecutorController};
use unleash_domain::{ActionState, DispatchId};
use unleash_platform::{OsProcessIdentity, observe_owned_tcp_listener};
use unleash_process_host::{HOST_SOCKET_FILENAME, HostBootstrapSpec};
use unleash_protocol::CommandEnvelope;
use unleash_store::{
    BrokerPhaseCommit, CallerAuthority, ManagedProviderListener, PreparedProviderProbe,
    ProviderServiceLaunchAttestation, ProviderServiceLaunchContext, ProviderServiceLaunchOutcome,
    ProviderServiceLaunchPermit, ProviderServiceLaunchRefusal, ProviderServiceProcessIdentity,
    StoreError, UnleashStore,
};

/// Durable disposition of a managed provider service launch.
#[derive(Debug)]
pub struct ManagedProviderServiceLaunch {
    /// Commit of the outcome, including the event cursor.
    pub commit: BrokerPhaseCommit,
    /// `ObservedSuccess`, `ObservedFailure` for a refusal without effect, or `OutcomeUnknown`.
    pub state: ActionState,
    /// Certain or unknown issue of the launch, distinct from the broker state.
    pub disposition: ProviderServiceLaunchDisposition,
}

/// Issue d'un lancement de service managé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderServiceLaunchDisposition {
    /// Hôte de processus lancé et service attesté.
    Attested,
    /// Aucun processus lancé : refus certain, sans effet à réconcilier.
    NotStarted(ProviderServiceLaunchRefused),
    /// Hôte lancé sans attestation : l'issue reste à réconcilier.
    Unconfirmed,
}

pub(crate) struct ManagedProviderServiceHosts {
    rendezvous: PathBuf,
    hosts: Mutex<BTreeMap<DispatchId, Arc<OwnedService>>>,
}

struct OwnedService {
    context: ProviderServiceLaunchContext,
    host: Mutex<OwnedProviderServiceHost>,
}

impl ManagedProviderServiceHosts {
    pub(crate) fn new(rendezvous: PathBuf) -> Self {
        Self {
            rendezvous,
            hosts: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn launch(
        &self,
        store: &UnleashStore,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
        bootstrap: HostBootstrapSpec,
        permit: ProviderServiceLaunchPermit,
    ) -> Result<ManagedProviderServiceLaunch, ProviderServiceLaunchError> {
        let executable = std::env::current_exe()?.with_file_name("unleash-process-host");
        self.launch_with_binary(store, envelope, caller, bootstrap, permit, &executable)
    }

    /// Lance le service ou solde son refus ; l'issue durable n'est jamais laissée indéterminée.
    ///
    /// Sûreté à l'annulation : cette fonction est synchrone. Un appelant interrompu ne peut
    /// l'arrêter qu'entre le `spawn` de l'hôte et la consignation du writer ; le marqueur de
    /// dispatch reste alors engagé et impose la réconciliation, sans second permis.
    pub(super) fn launch_with_binary(
        &self,
        store: &UnleashStore,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
        bootstrap: HostBootstrapSpec,
        permit: ProviderServiceLaunchPermit,
        executable: &Path,
    ) -> Result<ManagedProviderServiceLaunch, ProviderServiceLaunchError> {
        let context = permit.context().clone();
        let launch_id = context.launch_id;
        let dispatch_id = permit.dispatch_id();
        let mut registration = ExecutorRegistration {
            executor_id: permit.context().host_id.to_string(),
            data_root: store.config().data_root.clone(),
            pid: None,
            boot_identity: None,
            start_identity: None,
            os_identity: None,
            state: ExecutorState::Active,
        };
        // Tout refus certain survenu avant un processus fils est journalisé et soldé en échec
        // observé sans effet : il ne reste jamais une issue inconnue (REQ-MODEL-015).
        let (refused, attested) = match self.register(&registration) {
            Err(code) => {
                tracing::warn!(
                    code,
                    launch_id = %launch_id,
                    dispatch_id = %dispatch_id,
                    "Lancement du service fournisseur refusé : registre d'exécuteurs indisponible"
                );
                (
                    Some(ProviderServiceLaunchRefused::from_kind(
                        ProviderServiceLaunchRefusal::LaunchUnavailable,
                        "executor_registry_unavailable",
                        "io",
                    )),
                    None,
                )
            }
            Ok(()) => match spawn_provider_service_host(
                store,
                envelope,
                caller,
                bootstrap,
                permit,
                executable,
            ) {
                Ok(host) => (None, self.observe_started(context, registration, host, launch_id)),
                Err(refused) => {
                    tracing::warn!(
                        code = refused.code,
                        kind = refused.kind,
                        cause = ?refused.cause,
                        launch_id = %launch_id,
                        dispatch_id = %dispatch_id,
                        "Lancement du service fournisseur refusé avant tout processus"
                    );
                    registration.state = ExecutorState::Neutralized;
                    if let Err(code) = self.register(&registration) {
                        tracing::warn!(
                            code,
                            launch_id = %launch_id,
                            dispatch_id = %dispatch_id,
                            "Retrait du registre indisponible après un lancement refusé"
                        );
                    }
                    (Some(refused), None)
                }
            },
        };
        let (outcome, state, disposition) = match (refused, attested) {
            (Some(refused), _) => (
                ProviderServiceLaunchOutcome::NotStarted {
                    cause: refused.cause,
                },
                ActionState::ObservedFailure,
                ProviderServiceLaunchDisposition::NotStarted(refused),
            ),
            (None, Some(proof)) if !store.config().clock_uncertain.load(Ordering::Acquire) => (
                ProviderServiceLaunchOutcome::Attested(Box::new(proof)),
                ActionState::ObservedSuccess,
                ProviderServiceLaunchDisposition::Attested,
            ),
            (None, attested) => {
                if attested.is_some() {
                    tracing::warn!(
                        code = "clock_uncertain",
                        launch_id = %launch_id,
                        dispatch_id = %dispatch_id,
                        "Hôte du service fournisseur attesté sous horloge incertaine : issue inconnue"
                    );
                }
                (
                    ProviderServiceLaunchOutcome::Unconfirmed,
                    ActionState::OutcomeUnknown,
                    ProviderServiceLaunchDisposition::Unconfirmed,
                )
            }
        };
        let commit =
            store.commit_provider_service_launch_observation(launch_id, dispatch_id, outcome)?;
        Ok(ManagedProviderServiceLaunch {
            commit,
            state,
            disposition,
        })
    }

    /// Retient l'hôte démarré avant de chercher son attestation ; `None` laisse l'issue inconnue.
    fn observe_started(
        &self,
        context: ProviderServiceLaunchContext,
        mut registration: ExecutorRegistration,
        host: OwnedProviderServiceHost,
        launch_id: uuid::Uuid,
    ) -> Option<ProviderServiceLaunchAttestation> {
        let dispatch = host.dispatch_id();
        let service = Arc::new(OwnedService {
            context,
            host: Mutex::new(host),
        });
        self.hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(dispatch, Arc::clone(&service));
        let mut host = service.host.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(identity) = host.host_identity.as_ref() else {
            tracing::warn!(
                code = "host_identity_unverified",
                launch_id = %launch_id,
                dispatch_id = %dispatch,
                "Hôte du service fournisseur lancé sans identité vérifiable : issue inconnue"
            );
            return None;
        };
        registration.pid = Some(identity.pid);
        registration.boot_identity = Some(identity.boot_identity.as_str().into());
        registration.start_identity = Some(identity.start_identity.as_str().into());
        registration.os_identity = Some(identity.clone());
        if let Err(code) = self.register(&registration) {
            tracing::warn!(
                code,
                launch_id = %launch_id,
                dispatch_id = %dispatch,
                "Hôte du service fournisseur lancé : registre d'exécuteurs indisponible"
            );
            return None;
        }
        match wait_for_attestation(&mut host) {
            Ok(proof) => Some(proof),
            Err(error) => {
                tracing::warn!(
                    code = %error.diagnostic_code(),
                    launch_id = %launch_id,
                    dispatch_id = %dispatch,
                    "Hôte du service fournisseur lancé sans attestation : issue inconnue"
                );
                None
            }
        }
    }

    fn register(&self, registration: &ExecutorRegistration) -> Result<(), &'static str> {
        if RendezvousExecutorController::upsert_registration(&self.rendezvous, registration.clone())
            .is_err()
        {
            return Err("executor_registry_unavailable");
        }
        Ok(())
    }

    pub(crate) fn observe_listener(
        &self,
        store: &UnleashStore,
        dispatch: DispatchId,
    ) -> Result<ManagedProviderListener, ProviderServiceLaunchError> {
        let launch = store.read_attested_provider_service_launch(dispatch)?;
        let profile = store
            .resolve_provider_profile_snapshot(
                launch.context.plan.profile_id,
                launch.context.plan.profile_version,
            )?
            .value
            .filter(|profile| profile.configuration_digest == launch.context.configuration_digest)
            .ok_or(ProviderServiceLaunchError::ListenerUnverified)?;
        let destination = profile
            .configuration
            .payload
            .endpoint_destination()
            .map_err(|_| ProviderServiceLaunchError::ListenerUnverified)?;
        if destination.normalized_endpoint() != launch.context.endpoint
            || !destination.is_loopback_literal()
        {
            return Err(ProviderServiceLaunchError::ListenerUnverified);
        }
        let ip: IpAddr = destination
            .host()
            .parse()
            .map_err(|_| ProviderServiceLaunchError::ListenerUnverified)?;
        let endpoint = SocketAddr::new(ip, destination.port());
        let host = self
            .hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&dispatch)
            .cloned()
            .ok_or(ProviderServiceLaunchError::HostUnverified)?;
        let mut host = host.host.lock().unwrap_or_else(PoisonError::into_inner);
        let current = host.attest()?;
        if identity_document(current.child_identity()) != launch.attestation.child
            || host.host_identity.as_ref().map(identity_document).as_ref()
                != Some(&launch.attestation.host)
            || current.allocation_context_digest() != launch.attestation.context_digest
        {
            return Err(ProviderServiceLaunchError::HostUnverified);
        }
        let listener = observe_owned_tcp_listener(current.child_identity(), endpoint)
            .map_err(|_| ProviderServiceLaunchError::ListenerUnverified)?;
        Ok(ManagedProviderListener {
            dispatch_id: dispatch,
            child: identity_document(listener.process_identity()),
            endpoint: listener.endpoint(),
            observed_at: store.config().clock.now_ms(),
        })
    }

    pub(crate) fn observe_probe_listener(
        &self,
        store: &UnleashStore,
        probe: &PreparedProviderProbe,
    ) -> Result<Option<ManagedProviderListener>, ProviderServiceLaunchError> {
        let probe = &probe.context;
        let candidates: Vec<_> = self
            .hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, service)| {
                let context = &service.context;
                context.profile_version_ref == probe.preparation.profile_version_ref
                    && context.project_id == probe.project_id
                    && context.scope_revision == probe.scope_revision
                    && context.policy_digest == probe.security_policy_digest
                    && context.lineage_fence.authority_generation == probe.authority_generation
                    && context.lineage_fence.epoch == probe.daemon_epoch
            })
            .map(|(dispatch, _)| *dispatch)
            .collect();
        let mut selected = None;
        for dispatch in candidates {
            match self.observe_listener(store, dispatch) {
                Ok(_) if selected.is_some() => return Ok(None),
                Ok(listener) => selected = Some(listener),
                // La preuve de lancement attesté n'est pas encore engagée : attente normale.
                Err(error @ ProviderServiceLaunchError::Store(StoreError::EvidenceUnavailable(_))) => {
                    tracing::debug!(
                        code = %error.diagnostic_code(),
                        dispatch_id = %dispatch,
                        "Écoute du service managé en attente de sa preuve de lancement"
                    );
                }
                Err(ProviderServiceLaunchError::Store(error)) => return Err(error.into()),
                Err(error) => tracing::warn!(
                    code = %error.diagnostic_code(),
                    dispatch_id = %dispatch,
                    "Écoute du service managé non confirmée pour le probe de génération"
                ),
            }
        }
        Ok(selected)
    }

    // Only the macOS evaluation tests retain the spawned fixture host to reap
    // it. `validate_launch` refuses the Strict service plan off macOS, so the
    // accessor stays gated exactly like its callers (REQ-MODEL-015).
    #[cfg(all(test, target_os = "macos"))]
    pub(super) fn take_fixture_host(
        &self,
        dispatch: DispatchId,
    ) -> Option<OwnedProviderServiceHost> {
        let owned = self.hosts.lock().ok()?.remove(&dispatch)?;
        Arc::try_unwrap(owned).ok()?.host.into_inner().ok()
    }
}

fn wait_for_attestation(
    host: &mut OwnedProviderServiceHost,
) -> Result<ProviderServiceLaunchAttestation, ProviderServiceLaunchError> {
    let ready = host.bootstrap.runtime_dir.join(HOST_SOCKET_FILENAME);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready.try_exists()? {
        if host.try_wait()?.is_some() || Instant::now() >= deadline {
            return Err(ProviderServiceLaunchError::HostUnverified);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let observed = host.attest()?;
    let identity = host
        .host_identity
        .as_ref()
        .ok_or(ProviderServiceLaunchError::HostUnverified)?;
    Ok(ProviderServiceLaunchAttestation {
        context_digest: observed.allocation_context_digest(),
        environment_digest: observed.environment_digest(),
        confinement_digest: observed.isolation_profile_digest(),
        host: identity_document(identity),
        child: identity_document(observed.child_identity()),
    })
}

fn identity_document(identity: &OsProcessIdentity) -> ProviderServiceProcessIdentity {
    ProviderServiceProcessIdentity {
        pid: identity.pid,
        pgid: identity.pgid,
        owner_uid: identity.owner_uid,
        boot_identity: identity.boot_identity.as_str().into(),
        start_identity: identity.start_identity.as_str().into(),
    }
}
