//! Keep the direct process-host handle before recording its result.

use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, atomic::Ordering},
    time::{Duration, Instant},
};

use super::{OwnedProviderServiceHost, ProviderServiceLaunchError, spawn_provider_service_host};
use crate::executors::{ExecutorRegistration, ExecutorState, RendezvousExecutorController};
use unleash_domain::{ActionState, DispatchId};
use unleash_platform::{OsProcessIdentity, observe_owned_tcp_listener};
use unleash_process_host::{HOST_SOCKET_FILENAME, HostBootstrapSpec};
use unleash_protocol::CommandEnvelope;
use unleash_store::{
    BrokerPhaseCommit, CallerAuthority, ManagedProviderListener, PreparedProviderProbe,
    ProviderServiceLaunchAttestation, ProviderServiceLaunchContext, ProviderServiceLaunchOutcome,
    ProviderServiceLaunchPermit, ProviderServiceProcessIdentity, StoreError, UnleashStore,
};

/// Durable disposition of a managed provider service launch.
#[derive(Debug)]
pub struct ManagedProviderServiceLaunch {
    /// Commit of the outcome, including the event cursor.
    pub commit: BrokerPhaseCommit,
    /// `ObservedSuccess` for an attested launch or `OutcomeUnknown` otherwise.
    pub state: ActionState,
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
        let registration = ExecutorRegistration {
            executor_id: permit.context().host_id.to_string(),
            data_root: store.config().data_root.clone(),
            pid: None,
            boot_identity: None,
            start_identity: None,
            os_identity: None,
            state: ExecutorState::Active,
        };
        let attestation = if self.register(&registration) {
            let started =
                spawn_provider_service_host(store, envelope, caller, bootstrap, permit, executable);
            self.observe_started(context, registration, started)
        } else {
            None
        };
        let (outcome, state) = match attestation {
            Some(proof) if !store.config().clock_uncertain.load(Ordering::Acquire) => (
                ProviderServiceLaunchOutcome::Attested(Box::new(proof)),
                ActionState::ObservedSuccess,
            ),
            _ => (
                ProviderServiceLaunchOutcome::Unconfirmed,
                ActionState::OutcomeUnknown,
            ),
        };
        let commit =
            store.commit_provider_service_launch_observation(launch_id, dispatch_id, outcome)?;
        Ok(ManagedProviderServiceLaunch { commit, state })
    }

    fn observe_started(
        &self,
        context: ProviderServiceLaunchContext,
        mut registration: ExecutorRegistration,
        started: Result<OwnedProviderServiceHost, ProviderServiceLaunchError>,
    ) -> Option<ProviderServiceLaunchAttestation> {
        let Ok(host) = started else {
            registration.state = ExecutorState::Neutralized;
            self.register(&registration);
            return None;
        };
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
        let identity = host.host_identity.as_ref()?;
        registration.pid = Some(identity.pid);
        registration.boot_identity = Some(identity.boot_identity.as_str().into());
        registration.start_identity = Some(identity.start_identity.as_str().into());
        registration.os_identity = Some(identity.clone());
        if !self.register(&registration) {
            return None;
        }
        wait_for_attestation(&mut host).ok()
    }

    fn register(&self, registration: &ExecutorRegistration) -> bool {
        if RendezvousExecutorController::upsert_registration(&self.rendezvous, registration.clone())
            .is_err()
        {
            tracing::warn!("Managed provider host registry update unavailable");
            return false;
        }
        true
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
                Err(ProviderServiceLaunchError::Store(error))
                    if !matches!(error, StoreError::EvidenceUnavailable(_)) =>
                {
                    return Err(error.into());
                }
                Err(_) => {}
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
