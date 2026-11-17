// SPDX-License-Identifier: Apache-2.0
//! Binaire `unleash` de la ligne de commande OVPT Unleash.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use serde_json::json;
use unleash_cli::{Cli, Commands, run_cli};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let emit_json = cli.json;
    match run_cli(&cli).await {
        Ok(val) => {
            if !matches!(cli.command, Commands::Tui { .. }) || emit_json {
                let mut out = std::io::stdout().lock();
                let rendered =
                    serde_json::to_string_pretty(&val).unwrap_or_else(|_| "{}".to_owned());
                let _ = writeln!(out, "{rendered}");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            let mut err_out = std::io::stderr().lock();
            if emit_json {
                let payload = json!({ "error": err.to_string() });
                let _ = writeln!(err_out, "{payload}");
            } else {
                let _ = writeln!(err_out, "{err}");
            }
            ExitCode::FAILURE
        }
    }
}
