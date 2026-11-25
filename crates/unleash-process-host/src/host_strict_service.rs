// SPDX-License-Identifier: Apache-2.0
//! Explicit launch environment for a directly owned, egress-denied macOS service.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{Mode, OFlags};
use sha2::{Digest as _, Sha256};
use unleash_domain::{CommandId, Sha256Digest};
use unleash_platform::{
    OsProcessIdentity, effective_uid, ensure_private_rendezvous_dir, inspect_process_identity,
    read_private_file_to_string, validate_rendezvous_dir,
};

use crate::HostControlState;
use crate::error::ProcessHostError;
use crate::host::{HOST_SOCKET_FILENAME, HostClient};
use crate::isolation::{
    IsolationBackendProbe, IsolationMode, IsolationProfileKind, MACOS_SEATBELT_PROFILE_FILENAME,
    OsIsolationBackendProbe, PreparedLaunchCommand, SandboxProbeOutcome, StrictIsolationSpec,
    prepare_strict_launch_command, render_macos_seatbelt_profile,
};
use crate::manifest::{
    CANARY_SECRETS_FILENAME, CONTROL_SECRET_FILENAME, EnvironmentMode, HOST_MANIFEST_FILENAME,
    HostBootstrapSpec, HostControlSecret, HostLaunchState, HostManifest, ResourceKind,
    effective_environment_digest_v1, explicit_environment_v1,
};
use crate::protocol::{HostRequest, HostResponse};

static SERVICE_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn prepare(
    spec: &HostBootstrapSpec,
    cwd: &Path,
    probe: &dyn IsolationBackendProbe,
) -> Result<(PreparedLaunchCommand, Sha256Digest), ProcessHostError> {
    let plan = validate_launch(spec, cwd)?;
    ensure_private_rendezvous_dir(&cwd.join("child-tmp"))?;
    let prepared = prepare_strict_launch_command(
        &spec.program,
        &spec.arguments,
        plan.environment,
        &plan.confinement,
        probe,
    )?;
    Ok((prepared, plan.environment_digest))
}

struct StrictServicePlan {
    environment: BTreeMap<String, String>,
    confinement: StrictIsolationSpec,
    allocation_context_digest: Sha256Digest,
    environment_digest: Sha256Digest,
}

/// Fingerprints of the effective Strict bootstrap before host execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrictServiceLaunchDigests {
    /// Executable file bytes.
    pub executable_digest: Sha256Digest,
    /// Program, arguments, path and explicit environment keys.
    pub command_digest: Sha256Digest,
    /// Complete effective environment.
    pub environment_digest: Sha256Digest,
    /// Exact confinement profile, with runtime paths protected.
    pub confinement_digest: Sha256Digest,
}

/// Inspect the exact configuration that a Strict service would use.
/// This reads the executable and renders the profile in memory. The host
/// still has to check and enforce the profile at launch.
///
/// # Errors
/// Refuses a bootstrap unsupported by the native Strict service plan.
pub fn inspect_strict_service_launch_plan(
    spec: &HostBootstrapSpec,
) -> Result<StrictServiceLaunchDigests, ProcessHostError> {
    let cwd = spec.cwd.canonicalize()?;
    let plan = validate_launch(spec, &cwd)?;
    let profile = render_macos_seatbelt_profile(&plan.confinement)?;
    let fd = rustix::fs::open(
        Path::new(&spec.program),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    let mut executable = File::from(fd);
    if !executable.metadata()?.is_file() {
        return Err(ProcessHostError::InvalidArgument {
            reason: "strict service executable is not a regular file".into(),
        });
    }
    let mut digest = Sha256::new();
    std::io::copy(&mut executable, &mut digest)?;
    Ok(StrictServiceLaunchDigests {
        executable_digest: Sha256Digest::from_raw_bytes(digest.finalize().into()),
        command_digest: spec.compute_command_digest()?,
        environment_digest: plan.environment_digest,
        confinement_digest: Sha256Digest::from_bytes(profile.as_bytes()),
    })
}

/// Confirm that native Seatbelt can load the exact prepared profile.
/// The OS probe runs its inert command and the profile file is then removed.
///
/// # Errors
/// Refuses a divergent profile, an unavailable backend, or a failed profile probe.
pub fn probe_strict_service_confinement(
    spec: &HostBootstrapSpec,
    expected_profile: Sha256Digest,
) -> Result<(), ProcessHostError> {
    let cwd = spec.cwd.canonicalize()?;
    let plan = validate_launch(spec, &cwd)?;
    let profile = render_macos_seatbelt_profile(&plan.confinement)?;
    if Sha256Digest::from_bytes(profile.as_bytes()) != expected_profile {
        return Err(ProcessHostError::InvalidArgument {
            reason: "strict service profile differs from its preparation".into(),
        });
    }
    validate_rendezvous_dir(&spec.runtime_dir)?;
    let sequence = SERVICE_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let probe_profile = spec.runtime_dir.join(format!(
        "service-probe-{}-{sequence}.sb",
        std::process::id()
    ));
    let fd = rustix::fs::open(
        &probe_profile,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(std::io::Error::from)?;
    let probe = OsIsolationBackendProbe::default();
    let written = File::from(fd).write_all(profile.as_bytes());
    let result = written
        .map(|()| probe.probe_macos_seatbelt(probe.macos_sandbox_exec_path(), &probe_profile));
    std::fs::remove_file(probe_profile)?;
    match result? {
        SandboxProbeOutcome::Supported => Ok(()),
        SandboxProbeOutcome::MissingBackend { detail } => {
            Err(ProcessHostError::IsolationBackendMissing {
                backend: "macos-seatbelt".into(),
                remediation: detail,
            })
        }
        SandboxProbeOutcome::Unenforceable { detail } => {
            Err(ProcessHostError::IsolationObligationUnenforceable {
                backend: "macos-seatbelt".into(),
                reason: detail,
                remediation: "verify availability of the prepared Strict service profile".into(),
            })
        }
    }
}

fn validate_launch(
    spec: &HostBootstrapSpec,
    cwd: &Path,
) -> Result<StrictServicePlan, ProcessHostError> {
    let invalid = || ProcessHostError::InvalidArgument {
        reason: "strict service launch is not qualified".into(),
    };
    if !cfg!(target_os = "macos") {
        return Err(ProcessHostError::IsolationObligationUnenforceable {
            backend: "strict_service_v1".into(),
            reason: "this service profile requires qualified macOS confinement".into(),
            remediation: "use a platform with qualified service confinement".into(),
        });
    }
    let strict = spec.strict_spec.as_ref().ok_or_else(invalid)?;
    let program = Path::new(&spec.program);
    let allocation_context_digest = spec.allocation_context_digest.ok_or_else(invalid)?;
    if spec.environment_mode != Some(EnvironmentMode::StrictServiceV1)
        || spec.resource_kind != ResourceKind::Service
        || spec.terminal_id.is_some()
        || spec.isolation_mode != IsolationMode::Strict
        || strict.profile_kind != IsolationProfileKind::WorkspaceCommand
        || spec.isolation_profile_kind != Some(strict.profile_kind)
        || strict.egress_proxy_addr.is_some()
        || !strict.extra_loopback_control_ports.is_empty()
        || strict.profile_output_dir != spec.runtime_dir
        || cwd != spec.cwd
        || !program.is_absolute()
        || spec.program.contains('\0')
        || spec.arguments.iter().any(|arg| arg.contains('\0'))
    {
        return Err(invalid());
    }
    let runtime = spec.runtime_dir.canonicalize()?;
    if runtime != spec.runtime_dir
        || spec.control_secret_path != runtime.join(CONTROL_SECRET_FILENAME)
        || spec
            .canaries_secret_path
            .as_deref()
            .is_some_and(|path| path != runtime.join(CANARY_SECRETS_FILENAME))
        || cwd.starts_with(&runtime)
        || runtime.starts_with(cwd)
        || program.canonicalize()? != program
    {
        return Err(invalid());
    }
    validate_rendezvous_dir(cwd)?;
    let child_tmp = cwd.join("child-tmp");
    if !strict
        .assigned_write_roots
        .iter()
        .any(|root| root == cwd || root == &child_tmp)
    {
        return Err(invalid());
    }
    let environment = explicit_environment_v1(&spec.explicit_env, &child_tmp)?;
    let digest = effective_environment_digest_v1(&environment)?;
    if spec.expected_effective_environment_digest != Some(digest) {
        return Err(invalid());
    }

    let mut confinement = strict.clone();
    // The child may not read or overwrite the host's token, manifest, or profile.
    // The shared strict renderer rejects write roots overlapping this directory.
    confinement.declared_credential_paths.push(runtime);
    Ok(StrictServicePlan {
        environment,
        confinement,
        allocation_context_digest,
        environment_digest: digest,
    })
}

/// Live identity and launch digests accepted from a verified strict service host.
///
/// This observation is tied to an already authorized launch supplied by the
/// caller. It does not grant permission, establish build bytes or prove placement.
#[derive(Debug)]
pub struct StrictServiceLaunchObservation {
    child_identity: OsProcessIdentity,
    allocation_context_digest: Sha256Digest,
    environment_digest: Sha256Digest,
    isolation_profile_digest: Sha256Digest,
}

impl StrictServiceLaunchObservation {
    /// Exact identity of the direct child verified against the OS.
    #[must_use]
    pub const fn child_identity(&self) -> &OsProcessIdentity {
        &self.child_identity
    }

    /// Digest of the already authorized allocation which selected the host.
    #[must_use]
    pub const fn allocation_context_digest(&self) -> Sha256Digest {
        self.allocation_context_digest
    }

    /// Exact verified explicit launch environment digest.
    #[must_use]
    pub const fn environment_digest(&self) -> Sha256Digest {
        self.environment_digest
    }

    /// Exact expected and observed Seatbelt profile digest.
    #[must_use]
    pub const fn isolation_profile_digest(&self) -> Sha256Digest {
        self.isolation_profile_digest
    }
}

/// Confirm a strict service launch using the identity of the owned host and
/// its retained control secret. The caller supplies the original authorization
/// context through the bootstrap spec and performs the call off an async reactor.
///
/// This performs the host's normal fenced, non-rotating handshake. It never
/// starts, adopts from disk, signals, or connects to the network service.
///
/// # Errors
/// Rejects unqualified bootstraps, stale or mismatched process, host, epoch or
/// fence, invalid private artifacts and absent or inconsistent acknowledgements.
pub fn attest_strict_service_launch(
    spec: &HostBootstrapSpec,
    expected_host: &OsProcessIdentity,
    secret: &HostControlSecret,
    command_id: CommandId,
) -> Result<StrictServiceLaunchObservation, ProcessHostError> {
    let cwd = spec.cwd.canonicalize()?;
    let plan = validate_launch(spec, &cwd)?;
    let profile = render_macos_seatbelt_profile(&plan.confinement)?;
    let profile_digest = Sha256Digest::from_bytes(profile.as_bytes());
    let command_digest = spec.compute_command_digest()?;
    let before = read_verified_child(spec, &plan, command_digest, &profile)?;
    let mut client = HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        spec.daemon_epoch,
        spec.resource_fence,
    )?;
    client.verify_owned_peer(expected_host)?;
    let header = client.make_header(command_id, spec.action_id, plan.allocation_context_digest);
    let HostResponse::HandshakeAck {
        effective_environment_digest,
        allocation_context_digest,
        process_run_uuid,
        authority_generation,
        daemon_epoch,
        highest_fence,
        os_identity,
        secret_rotated,
        last_confirmed_input_seq,
        unknown_input_seqs,
        ..
    } = client.call(&HostRequest::Handshake {
        header,
        current_secret: secret.expose_secret().into(),
        rotate_to_secret: None,
        require_higher_epoch: false,
    })?
    else {
        return Err(unverified());
    };
    if effective_environment_digest != Some(plan.environment_digest)
        || allocation_context_digest != Some(plan.allocation_context_digest)
        || process_run_uuid != spec.process_run_uuid
        || authority_generation != spec.authority_generation
        || daemon_epoch != spec.daemon_epoch
        || highest_fence != spec.resource_fence
        || os_identity != before
        || secret_rotated
        || last_confirmed_input_seq != 0
        || !unknown_input_seqs.is_empty()
    {
        return Err(unverified());
    }
    client.verify_owned_peer(expected_host)?;
    let after = read_verified_child(spec, &plan, command_digest, &profile)?;
    if after != before {
        return Err(unverified());
    }
    Ok(StrictServiceLaunchObservation {
        child_identity: after,
        allocation_context_digest: plan.allocation_context_digest,
        environment_digest: plan.environment_digest,
        isolation_profile_digest: profile_digest,
    })
}

fn read_verified_child(
    spec: &HostBootstrapSpec,
    plan: &StrictServicePlan,
    command_digest: Sha256Digest,
    profile: &str,
) -> Result<OsProcessIdentity, ProcessHostError> {
    let runtime = &spec.runtime_dir;
    let manifest = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    if manifest.schema_version != 1
        || manifest.host_id != spec.host_id
        || manifest.process_id != spec.process_id
        || manifest.process_run_uuid != spec.process_run_uuid
        || manifest.action_id != spec.action_id
        || manifest.authority_generation != spec.authority_generation
        || manifest.daemon_epoch != spec.daemon_epoch
        || manifest.highest_fence != spec.resource_fence
        || manifest.terminal_id.is_some()
        || manifest.mission_id != spec.mission_id
        || manifest.owner_agent_id != spec.owner_agent_id
        || manifest.worktree_id != spec.worktree_id
        || manifest.cwd_canonical != spec.cwd
        || manifest.command_digest != command_digest
        || manifest.allocation_context_digest != Some(plan.allocation_context_digest)
        || manifest.effective_environment_digest != Some(plan.environment_digest)
        || manifest.owner_uid != effective_uid()
        || manifest.isolation_mode != IsolationMode::Strict
        || manifest.isolation_backend.as_deref() != Some("macos-seatbelt")
        || manifest.isolation_profile_digest != Some(Sha256Digest::from_bytes(profile.as_bytes()))
        || manifest.socket_path != runtime.join(HOST_SOCKET_FILENAME)
        || manifest.control_secret_path != spec.control_secret_path
        || manifest.spool_dir != runtime.join("spool")
        || manifest.launch_state != HostLaunchState::Running
        || manifest.last_confirmed_input_seq != 0
        || manifest.in_flight_input_seq.is_some()
        || !matches!(
            manifest.control_state,
            Some(HostControlState::Active {
                last_reacquisition: None
            })
        )
        || read_private_file_to_string(&runtime.join(MACOS_SEATBELT_PROFILE_FILENAME))?.as_deref()
            != Some(profile)
    {
        return Err(unverified());
    }

    let live = inspect_process_identity(manifest.pid.ok_or_else(unverified)?)?;
    if live.owner_uid != manifest.owner_uid
        || manifest.pgid != Some(live.pgid)
        || manifest.boot_identity.as_ref() != Some(&live.boot_identity)
        || manifest.start_identity.as_ref() != Some(&live.start_identity)
        || manifest.identity_backend != live.backend
    {
        return Err(unverified());
    }
    Ok(live)
}

fn unverified() -> ProcessHostError {
    ProcessHostError::IdentityUnverified {
        reason: "strict service launch proof differs".into(),
    }
}
