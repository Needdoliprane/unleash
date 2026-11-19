// SPDX-License-Identifier: Apache-2.0
//! Binaire `unleash` de la ligne de commande OVPT Unleash.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use serde_json::{Value, json};
use unleash_cli::{Cli, CliError, Commands, run_cli};
use unleash_client::ClientError;

fn error_json(error: &CliError) -> Value {
    let mut payload = match error {
        CliError::TerminalReceiptMissing {
            terminal_id,
            project_id,
            command_id,
            authority_generation,
            ..
        } => json!({
            "error": error.to_string(),
            "project_id": project_id,
            "terminal_id": terminal_id,
            "command_id": command_id,
            "authority_generation": authority_generation,
        }),
        CliError::ConfigurationReceiptMissing {
            aggregate_id,
            command_id,
            authority_generation,
            ..
        } => json!({
            "error": error.to_string(),
            "aggregate_id": aggregate_id,
            "command_id": command_id,
            "authority_generation": authority_generation,
        }),
        _ => json!({ "error": error.to_string() }),
    };
    let client_error = match error {
        CliError::Client(source) => Some(source),
        CliError::BackupReceiptMissing { source, .. }
        | CliError::TerminalReceiptMissing { source, .. }
        | CliError::ConfigurationReceiptMissing { source, .. } => Some(source.as_ref()),
        _ => None,
    };
    if let (Some(ClientError::Remote(remote)), Some(fields)) =
        (client_error, payload.as_object_mut())
    {
        fields.insert("code".into(), json!(remote.code));
        fields.insert("reason_code".into(), json!(remote.reason_code));
        fields.insert("retry_class".into(), json!(remote.retry_class));
    }
    payload
}

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
                let payload = error_json(&err);
                let _ = writeln!(err_out, "{payload}");
            } else {
                let _ = writeln!(err_out, "{err}");
            }
            ExitCode::FAILURE
        }
    }
}
