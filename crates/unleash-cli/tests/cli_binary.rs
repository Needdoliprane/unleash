// SPDX-License-Identifier: Apache-2.0
//! Tests d'intégration du binaire `unleash` (`--help`, sous-commande `tui` sans TTY).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Stdio};

use serde_json::Value;

#[test]
fn cli_binary_exposes_explicit_binary_terminal_input() {
    let output = Command::new(env!("CARGO_BIN_EXE_unleash"))
        .args(["process", "input", "--help"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "process input must exist: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in [
        "--project-id",
        "--terminal-id",
        "--process-run-uuid",
        "--lease-id",
        "--fence",
        "--input-seq",
        "--control-revision",
        "--file",
        "--command-id",
        "--authority-generation",
    ] {
        assert!(
            help.contains(flag),
            "explicit input identity and byte source: missing {flag}"
        );
    }
}

#[test]
fn cli_binary_exposes_terminal_release() {
    let output = Command::new(env!("CARGO_BIN_EXE_unleash"))
        .args(["process", "release", "--help"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "process release must exist: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in [
        "--project-id",
        "--terminal-id",
        "--process-run-uuid",
        "--lease-id",
        "--fence",
        "--control-revision",
        "--command-id",
        "--authority-generation",
    ] {
        assert!(help.contains(flag), "missing release option {flag}");
    }
}

#[test]
fn cli_binary_exposes_provider_show() {
    let output = Command::new(env!("CARGO_BIN_EXE_unleash"))
        .args(["provider", "show", "--help"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "provider show help: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in ["--profile-id", "--profile-version", "--project-id"] {
        assert!(help.contains(flag), "missing provider option {flag}");
    }
}

#[test]
fn cli_binary_exposes_provider_set() {
    let output = Command::new(env!("CARGO_BIN_EXE_unleash"))
        .args(["provider", "set", "--help"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "provider set help: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in [
        "--profile-id",
        "--project-id",
        "--file",
        "--expected-revision",
        "--command-id",
        "--authority-generation",
    ] {
        assert!(help.contains(flag), "missing provider set option {flag}");
    }
}

#[test]
fn cli_binary_help_lists_tui_and_non_tty_tui_refuses_raw_mode() {
    let bin = env!("CARGO_BIN_EXE_unleash");

    let help = Command::new(bin)
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .expect("exécution `unleash --help`");
    assert!(
        help.status.success(),
        "`unleash --help` doit réussir : stderr={}",
        String::from_utf8_lossy(&help.stderr)
    );
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_stdout.contains("tui"),
        "`unleash --help` doit lister la sous-commande `tui`"
    );

    let tui = Command::new(bin)
        .args(["--socket", "/tmp/nonexistent-unleash.sock", "--json", "tui"])
        .stdin(Stdio::null())
        .output()
        .expect("exécution `unleash tui`");
    assert!(
        !tui.status.success(),
        "`unleash tui` hors TTY doit sortir avec un code d'échec"
    );
    let err_json: Value =
        serde_json::from_slice(&tui.stderr).expect("sortie JSON structurée attendue sur stderr");
    assert!(err_json.get("code").is_none());
    assert!(err_json.get("reason_code").is_none());
    assert!(err_json.get("retry_class").is_none());
    let err_msg = err_json
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        err_msg.contains("Terminal interactif requis"),
        "Message d'erreur inattendu : {err_msg}"
    );
}
