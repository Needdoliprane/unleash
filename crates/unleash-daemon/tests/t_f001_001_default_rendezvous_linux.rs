// T-F001-001 / AC-F001-001 / REQ-CORE-001
//! Rendez-vous canonique par défaut du vrai binaire `unleashd` sous Linux, sans `--rendezvous-dir`.
//!
//! Ces tests sont `#[ignore]` : ils dépendent de l'état réel de la racine runtime `/run/user/<uid>`
//! de l'hôte. Quand leur précondition n'est pas satisfaite, ils échouent explicitement au lieu de
//! passer silencieusement. Ils se lancent un par un, jamais ensemble :
//!
//! ```text
//! cargo test -p unleash-daemon --test t_f001_001_default_rendezvous_linux -- --ignored --exact <nom>
//! ```
//!
//! - le refus : hôte sans `/run/user/<uid>` (banc Docker) ;
//! - le démarrage : hôte dont `/run/user/<uid>` existe, appartient à l'UID effectif et est en `0700`.

#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names
)]

use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use unleash_platform::effective_uid;
use unleash_testkit::{local_test_dir, private_test_dir};

/// Délai laissé à `unleashd` pour sortir seul quand un refus est attendu.
const EXIT_DEADLINE: Duration = Duration::from_secs(60);
/// Délai laissé à `unleashd` pour publier sa ligne prête.
const READY_DEADLINE: Duration = Duration::from_secs(60);

/// Processus du démon de test : jamais laissé vivant derrière un test.
struct DaemonChild(Child);

impl Drop for DaemonChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Sortie observée de `unleashd`.
struct DaemonExit {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// Commande du vrai binaire `unleashd`, isolée du domicile de l'hôte.
fn unleashd_command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_unleashd"));
    command.env("HOME", home);
    command
}

fn read_all(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Lance `unleashd` et attend sa sortie, bornée par `EXIT_DEADLINE`.
fn run_unleashd_until_exit(args: &[&str], home: &Path) -> DaemonExit {
    let mut command = unleashd_command(home);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = DaemonChild(command.spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + EXIT_DEADLINE;
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.0.kill();
            let _ = child.0.wait();
            let stdout = lossy(&stdout_reader.join().unwrap().unwrap());
            let stderr = lossy(&stderr_reader.join().unwrap().unwrap());
            panic!(
                "unleashd n'a pas terminé dans {EXIT_DEADLINE:?} ; \
                 stdout={stdout:?} ; stderr={stderr:?}"
            );
        }
        thread::sleep(Duration::from_millis(20));
    };
    DaemonExit {
        status,
        stdout: lossy(&stdout_reader.join().unwrap().unwrap()),
        stderr: lossy(&stderr_reader.join().unwrap().unwrap()),
    }
}

/// Lance `unleashd` et attend sa ligne prête, bornée par `READY_DEADLINE`.
fn spawn_unleashd_until_ready(args: &[&str], home: &Path) -> (DaemonChild, Value) {
    let mut command = unleashd_command(home);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = DaemonChild(command.spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let (sender, ready) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        let read = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(read);
    });
    let line = match ready.recv_timeout(READY_DEADLINE) {
        Ok(read) => read.unwrap(),
        Err(_) => panic!("unleashd n'a pas publié de ligne prête dans {READY_DEADLINE:?}"),
    };
    let ready = serde_json::from_str(line.trim()).unwrap();
    (child, ready)
}

/// Noms triés des entrées de `dir` ; vide si `dir` n'existe pas.
fn sorted_entries(dir: &Path) -> Vec<OsString> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            let path = dir.display().to_string();
            panic!("lecture de {path} impossible : {e}")
        }
    };
    let mut names: Vec<OsString> = read
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<_, _>>()
        .unwrap();
    names.sort();
    names
}

fn unleash_entries_under_tmp() -> Vec<OsString> {
    sorted_entries(Path::new("/tmp"))
        .into_iter()
        .filter(|name| name.to_string_lossy().starts_with("unleash"))
        .collect()
}

// T-F001-001 / AC-F001-001 / REQ-CORE-001
#[test]
#[ignore = "exige un Linux sans /run/user/<uid> : à lancer explicitement dans le banc Docker"]
fn t_f001_001_binary_without_runtime_root_refuses_before_any_creation() {
    let uid = effective_uid();
    let runtime_root = PathBuf::from(format!("/run/user/{uid}"));
    let rendezvous = format!("/run/user/{uid}/unleash");
    let tmp_candidate = format!("/tmp/unleash-{uid}");
    let run_user = Path::new("/run/user");
    let run_user_before = sorted_entries(run_user);
    let tmp_unleash_before = unleash_entries_under_tmp();
    let existing = runtime_root.display().to_string();
    assert!(
        !runtime_root.exists(),
        "Précondition non satisfaite : {existing} existe ; le refus ne se prouve que si la racine runtime est absente"
    );
    assert!(
        !Path::new(&tmp_candidate).exists(),
        "Précondition non satisfaite : {tmp_candidate} existe déjà"
    );

    let sandbox = local_test_dir("rdvrefus").unwrap();
    let data_root = sandbox.path().join("data");
    let home = sandbox.path().join("home");
    let data_arg = data_root.to_str().unwrap();
    let exit = run_unleashd_until_exit(
        &["--data-root", data_arg, "--foreground", "--ready-json"],
        &home,
    );

    let DaemonExit {
        status,
        stdout,
        stderr,
    } = exit;
    assert!(
        !status.success(),
        "unleashd doit échouer quand la racine runtime manque ; stdout={stdout:?} ; stderr={stderr}"
    );
    assert!(
        stdout.trim().is_empty(),
        "aucune ligne prête ne doit être publiée ; stdout={stdout:?}"
    );
    assert!(
        stderr.contains("failed_start"),
        "l'issue de démarrage failed_start doit être nommée ; stderr={stderr}"
    );
    assert!(
        stderr.contains(rendezvous.as_str()),
        "le refus doit nommer le chemin du rendez-vous {rendezvous} ; stderr={stderr}"
    );
    assert!(
        stderr.contains("logind"),
        "le refus doit nommer la remédiation ; stderr={stderr}"
    );

    // Rien n'a été créé : ni racine runtime, ni entrée de `/run/user`, ni candidat sous `/tmp`,
    // ni racine de données.
    assert!(
        !runtime_root.exists(),
        "la racine runtime ne doit jamais être créée"
    );
    assert_eq!(
        sorted_entries(run_user),
        run_user_before,
        "/run/user ne doit recevoir aucune entrée"
    );
    assert_eq!(
        unleash_entries_under_tmp(),
        tmp_unleash_before,
        "aucun candidat sous /tmp ne doit apparaître"
    );
    assert!(
        !Path::new(&tmp_candidate).exists(),
        "aucun candidat sous /tmp ne doit être retenu"
    );
    assert!(
        !data_root.exists(),
        "aucune base ne doit être ouverte ni créée avant le refus"
    );

    // Inverse : un chemin explicite (`--rendezvous-dir`, tests et isolement) reste créateur et le
    // démarrage reste possible.
    let explicit = private_test_dir("rdvexplicit").unwrap();
    let explicit_arg = explicit.path().to_str().unwrap();
    let (mut daemon, ready) = spawn_unleashd_until_ready(
        &[
            "--data-root",
            data_arg,
            "--rendezvous-dir",
            explicit_arg,
            "--foreground",
            "--ready-json",
        ],
        &home,
    );
    assert_eq!(
        ready.get("status").and_then(Value::as_str),
        Some("ready"),
        "un chemin explicite doit rester créateur ; ready={ready}"
    );
    let expected_socket = explicit.path().join("daemon.sock");
    assert_eq!(
        ready.get("socket_path").and_then(Value::as_str),
        expected_socket.to_str(),
        "le socket doit rester dans le rendez-vous explicite ; ready={ready}"
    );
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    assert!(
        !runtime_root.exists(),
        "la racine runtime ne doit toujours pas être créée"
    );
}
