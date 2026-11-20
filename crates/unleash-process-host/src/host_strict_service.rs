// SPDX-License-Identifier: Apache-2.0
//! Explicit launch environment for a directly owned, egress-denied macOS service.

use std::path::Path;

use unleash_domain::Sha256Digest;
use unleash_platform::{ensure_private_rendezvous_dir, validate_rendezvous_dir};

use crate::error::ProcessHostError;
use crate::isolation::{
    IsolationBackendProbe, IsolationMode, IsolationProfileKind, PreparedLaunchCommand,
    prepare_strict_launch_command,
};
use crate::manifest::{
    CANARY_SECRETS_FILENAME, CONTROL_SECRET_FILENAME, HostBootstrapSpec, ResourceKind,
    effective_environment_digest_v1, explicit_environment_v1,
};

pub(super) fn prepare(
    spec: &HostBootstrapSpec,
    cwd: &Path,
    probe: &dyn IsolationBackendProbe,
) -> Result<(PreparedLaunchCommand, Sha256Digest), ProcessHostError> {
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
    if spec.resource_kind != ResourceKind::Service
        || spec.terminal_id.is_some()
        || spec.isolation_mode != IsolationMode::Strict
        || strict.profile_kind != IsolationProfileKind::WorkspaceCommand
        || spec.isolation_profile_kind != Some(strict.profile_kind)
        || strict.egress_proxy_addr.is_some()
        || !strict.extra_loopback_control_ports.is_empty()
        || strict.profile_output_dir != spec.runtime_dir
        || spec.allocation_context_digest.is_none()
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
    ensure_private_rendezvous_dir(&child_tmp)?;
    let prepared = prepare_strict_launch_command(
        &spec.program,
        &spec.arguments,
        environment,
        &confinement,
        probe,
    )?;
    Ok((prepared, digest))
}
