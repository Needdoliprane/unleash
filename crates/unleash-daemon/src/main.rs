// SPDX-License-Identifier: Apache-2.0
//! Binaire `unleashd` du daemon utilisateur OVPT Unleash.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use serde_json::json;
use tokio::signal::unix::{SignalKind, signal};
use unleash_daemon::{DaemonConfig, DaemonError, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::DaemonStartupIssue;
use unleash_platform::PlatformError;

#[derive(Debug, Parser)]
#[command(name = "unleashd", version, about = "OVPT Unleash user daemon")]
struct Args {
    /// Explicit data root directory.
    #[arg(long)]
    data_root: Option<PathBuf>,
    /// Explicit private rendezvous directory (test/isolation use).
    #[arg(long)]
    rendezvous_dir: Option<PathBuf>,
    /// Explicit foreground execution mode.
    #[arg(long)]
    foreground: bool,
    /// Join an existing same-root instance via Hello instead of failing.
    #[arg(long)]
    attach: bool,
    /// Print one JSON readiness line to stdout once ready.
    #[arg(long)]
    ready_json: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let cfg = DaemonConfig {
        data_root: args.data_root,
        rendezvous_dir: args.rendezvous_dir,
        attach_if_running: args.attach,
        ..DaemonConfig::default()
    };

    match UnleashDaemon::start(cfg).await {
        Ok(DaemonStartOutcome::AlreadyRunning { hello, socket_path }) => {
            if args.ready_json {
                let line = json!({
                    "status": "already_running",
                    "authority_generation": hello.authority_generation,
                    "daemon_epoch": hello.daemon_epoch.as_u64(),
                    "daemon_instance_id": hello.daemon_instance_id,
                    "stream_id": hello.stream_id,
                    "socket_path": socket_path.display().to_string(),
                });
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{line}");
                let _ = out.flush();
            }
            ExitCode::SUCCESS
        }
        Ok(DaemonStartOutcome::Started(handle)) => {
            if args.ready_json {
                let line = json!({
                    "status": "ready",
                    "authority_generation": handle.authority_generation(),
                    "daemon_epoch": handle.daemon_epoch().as_u64(),
                    "daemon_instance_id": handle.daemon_instance_id(),
                    "stream_id": handle.stream_id(),
                    "socket_path": handle.socket_path().display().to_string(),
                });
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{line}");
                let _ = out.flush();
            }

            let _ = args.foreground;
            wait_for_stop_or_signal(&handle).await;
            ExitCode::SUCCESS
        }
        Ok(DaemonStartOutcome::Diagnostic(handle)) => {
            if args.ready_json {
                let line = json!({
                    "status": "read_only_recovery",
                    "socket_path": handle.socket_path().display().to_string(),
                });
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{line}");
                let _ = out.flush();
            }
            wait_for_diagnostic_stop_or_signal(&handle).await;
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(err @ DaemonError::Platform(PlatformError::RuntimeRootUnavailable { .. })) => {
            // Issue de démarrage `failed_start` (`runtime-lifecycle.md`) : aucun socket n'existe
            // encore, la sortie d'erreur standard est le seul canal. L'erreur nomme le chemin
            // refusé et sa remédiation ; rien n'a été créé.
            let mut err_out = std::io::stderr().lock();
            let _ = writeln!(err_out, "{} : {err}", DaemonStartupIssue::FailedStart);
            ExitCode::FAILURE
        }
        Err(err) => {
            let mut err_out = std::io::stderr().lock();
            let _ = writeln!(err_out, "{err}");
            ExitCode::FAILURE
        }
    }
}

async fn wait_for_diagnostic_stop_or_signal(handle: &unleash_daemon::diagnostic::DiagnosticHandle) {
    let mut sigterm = signal(SignalKind::terminate()).ok();
    tokio::select! {
        () = handle.wait_stopped() => {}
        _ = tokio::signal::ctrl_c() => handle.shutdown_graceful().await,
        () = wait_sigterm(&mut sigterm) => handle.shutdown_graceful().await,
    }
}

async fn wait_for_stop_or_signal(handle: &unleash_daemon::DaemonHandle) {
    let mut sigterm = signal(SignalKind::terminate()).ok();

    tokio::select! {
        _ = handle.wait_stopped() => {}
        _ = tokio::signal::ctrl_c() => {
            let _ = handle.shutdown_graceful("SIGINT").await;
        }
        () = wait_sigterm(&mut sigterm) => {
            let _ = handle.shutdown_graceful("SIGTERM").await;
        }
    }
}

async fn wait_sigterm(sig: &mut Option<tokio::signal::unix::Signal>) {
    match sig {
        Some(s) => {
            let _ = s.recv().await;
        }
        None => std::future::pending::<()>().await,
    }
}
