//! Typed host launch seam. The product implementation preserves the packaged
//! sibling binary and a cleared environment. Test injection is Rust-only.

use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Starts the host only after the caller consumed its durable dispatch permit.
/// The caller retains responsibility for identity capture and registration.
pub trait TerminalHostLauncher: Send + Sync {
    /// Launch the already selected packaged executable with its private bootstrap.
    /// The returned child must already be the leader of its own OS session.
    /// Neither argument carries a control secret value.
    ///
    /// # Errors
    /// Returns the operating system spawn failure; it does not retry or fall back.
    fn spawn(&self, executable: &Path, bootstrap: &Path) -> io::Result<Child>;
}

/// Default product launcher, with the same environment and stdio as the direct
/// launch path. It exposes no IPC or environment-controlled injection mechanism.
#[derive(Debug, Default)]
pub struct PackagedTerminalHostLauncher;

impl TerminalHostLauncher for PackagedTerminalHostLauncher {
    fn spawn(&self, executable: &Path, bootstrap: &Path) -> io::Result<Child> {
        if !executable.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "packaged process-host binary absent",
            ));
        }
        let mut command = Command::new(executable);
        command
            .args([std::ffi::OsStr::new("--bootstrap"), bootstrap.as_os_str()])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Capture the owned process after its session exists, before observing a
        // socket whose peer must match that exact native identity.
        unleash_process_host::configure_setsid_pre_exec(&mut command);
        command.spawn()
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::{PackagedTerminalHostLauncher, TerminalHostLauncher};

    #[test]
    fn t_f011_003_packaged_host_identity_starts_in_its_own_session()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = unleash_testkit::local_test_dir("packaged-host-session")?;
        let program = dir.path().join("host-without-setsid");
        // This fixture stays live and never changes its own group after exec.
        std::fs::write(&program, b"#!/bin/sh\nexec /bin/sleep 60\n")?;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))?;

        let mut child = PackagedTerminalHostLauncher.spawn(&program, &dir.path().join("unused"))?;
        let pid = child.id();
        let observed = unleash_platform::inspect_process_identity(pid);
        let stopped = child.kill();
        child.wait()?;
        stopped?;
        let identity = observed?;
        assert_eq!(identity.pid, pid);
        assert_eq!(identity.pgid, pid);
        Ok(())
    }
}
