//! Binaire éphémère de tentative agente.
#![forbid(unsafe_code)]

use std::io::{self, IsTerminal};
use std::process::ExitCode;

fn main() -> ExitCode {
    let (stdin, stdout) = (io::stdin(), io::stdout());
    if stdin.is_terminal() || stdout.is_terminal() {
        return ExitCode::FAILURE;
    }
    match unleash_agent_worker::run_inherited_channel(stdin.lock(), stdout.lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
