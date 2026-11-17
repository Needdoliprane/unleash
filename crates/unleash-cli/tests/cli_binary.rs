// SPDX-License-Identifier: Apache-2.0
//! Tests d'intégration du binaire `unleash` (`--help`, sous-commande `tui` sans TTY).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Stdio};

use serde_json::Value;

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
    let err_msg = err_json
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        err_msg.contains("Terminal interactif requis"),
        "Message d'erreur inattendu : {err_msg}"
    );
}
