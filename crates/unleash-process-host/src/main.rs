// SPDX-License-Identifier: Apache-2.0
//! Binaire autonome `unleash-process-host`.
//!
//! Lit exclusivement son fichier privé `--bootstrap <chemin>` (`0600`), n'accepte aucun secret en
//! ligne de commande (`argv`), s'isole dans sa propre session (`setsid`) afin de survivre à un
//! `SIGKILL` du daemon, et sert le canal de contrôle privé jusqu'à fermeture propre.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use unleash_platform::{
    assert_not_under_claude, read_private_file_to_string, validate_rendezvous_dir,
};
use unleash_process_host::{
    HOST_READY_PREFIX, HostBootstrapSpec, OsIsolationBackendProbe, OsProcessInspector,
    ProcessHostError, ProcessHostServer, StdioBarrierCutpointObserver,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let msg = format!("unleash-process-host error: {err}\n");
            let _ = std::io::stderr().write_all(msg.as_bytes());
            let _ = std::io::stderr().flush();
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), ProcessHostError> {
    let bootstrap_path = parse_bootstrap_arg(std::env::args_os().skip(1))?;
    assert_not_under_claude(&bootstrap_path)?;
    if let Some(parent) = bootstrap_path.parent() {
        validate_rendezvous_dir(parent)?;
    }

    let meta = fs::symlink_metadata(&bootstrap_path)?;
    if (meta.permissions().mode() & 0o7777) != 0o600 {
        return Err(ProcessHostError::Platform(
            unleash_platform::PlatformError::InsecurePermissions {
                path: bootstrap_path.display().to_string(),
                expected_mode: 0o600,
                actual_mode: meta.permissions().mode() & 0o7777,
            },
        ));
    }

    let raw = read_private_file_to_string(&bootstrap_path)?.ok_or_else(|| {
        ProcessHostError::InvalidArgument {
            reason: format!("fichier bootstrap absent : '{}'", bootstrap_path.display()),
        }
    })?;
    let spec: HostBootstrapSpec = serde_json::from_str(&raw)?;

    // S'isoler dans sa propre session pour survivre à la mort brutale (SIGKILL) du daemon parent.
    // Ignore EPERM si le parent a déjà appelé setsid() dans pre_exec.
    let _ = rustix::process::setsid();

    let cutpoints = StdioBarrierCutpointObserver::from_env()
        .map(|obs| Arc::new(obs) as Arc<dyn unleash_process_host::HostCutpointObserver>);
    let probe = OsIsolationBackendProbe::new();
    let server = ProcessHostServer::start(&spec, &probe, Box::new(OsProcessInspector), cutpoints)?;

    let ready_line = format!(
        "{HOST_READY_PREFIX}{}:{}\n",
        server.process_run_uuid(),
        server.socket_path().display()
    );
    let mut stdout = std::io::stdout();
    stdout.write_all(ready_line.as_bytes())?;
    stdout.flush()?;

    server.serve_until_shutdown()
}

fn parse_bootstrap_arg<I>(args: I) -> Result<PathBuf, ProcessHostError>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut iter = args.into_iter();
    let Some(flag) = iter.next() else {
        return Err(ProcessHostError::InvalidArgument {
            reason: String::from("argument obligatoire manquant : --bootstrap <chemin>"),
        });
    };
    if flag != "--bootstrap" {
        return Err(ProcessHostError::InvalidArgument {
            reason: String::from(
                "seul l'argument '--bootstrap <chemin>' est accepté (aucun secret en argv)",
            ),
        });
    }
    let Some(path) = iter.next() else {
        return Err(ProcessHostError::InvalidArgument {
            reason: String::from("chemin manquant après --bootstrap"),
        });
    };
    if iter.next().is_some() {
        return Err(ProcessHostError::InvalidArgument {
            reason: String::from("arguments supplémentaires non autorisés"),
        });
    }
    Ok(PathBuf::from(path))
}
