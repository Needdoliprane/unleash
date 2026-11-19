//! Typed host launch seam. The product implementation preserves the packaged
//! sibling binary and a cleared environment. Test injection is Rust-only.

use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Starts the host only after the caller consumed its durable dispatch permit.
/// The caller retains responsibility for identity capture and registration.
pub trait TerminalHostLauncher: Send + Sync {
    /// Launch the already selected packaged executable with its private bootstrap.
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
        Command::new(executable)
            .args([std::ffi::OsStr::new("--bootstrap"), bootstrap.as_os_str()])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }
}
