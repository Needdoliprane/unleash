// E2E M0 binaire réel : unleashd + unleash CLI (--json) + actor_ref forgé + SIGKILL + reprise + daemon stop.
//! Aucun `sleep` de synchronisation : synchronisation via `--ready-json` sur stdout et `daemon stop`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_domain::{CommandId, MissionId, ProjectId, RequestId, Revision};
use unleash_protocol::{ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

fn unleash_cli_bin() -> std::path::PathBuf {
    let daemon_exe = std::path::PathBuf::from(env!("CARGO_BIN_EXE_unleashd"));
    let cli_name = format!("unleash{}", std::env::consts::EXE_SUFFIX);
    let candidate = daemon_exe.with_file_name(cli_name);
    assert!(
        candidate.is_file(),
        "Binaire unleash introuvable à '{}' (lancer cargo test --workspace)",
        candidate.display()
    );
    candidate
}

fn spawn_unleashd(data_root: &Path, rdv_dir: &Path) -> (Child, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_unleashd"))
        .arg("--data-root")
        .arg(data_root)
        .arg("--rendezvous-dir")
        .arg(rdv_dir)
        .arg("--foreground")
        .arg("--ready-json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Échec lancement binaire unleashd");

    let stdout = child.stdout.take().expect("stdout pipe requis");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .expect("Lecture ligne --ready-json échouée");
    assert!(
        n > 0,
        "unleashd s'est terminé sans émettre la ligne JSON ready"
    );
    let ready: Value = serde_json::from_str(line.trim()).expect("JSON ready invalide");
    assert_eq!(ready.get("status").and_then(Value::as_str), Some("ready"));
    (child, ready)
}

fn run_cli_json(socket: &Path, args: &[&str]) -> Value {
    let cli = unleash_cli_bin();
    let output = Command::new(cli)
        .arg("--socket")
        .arg(socket)
        .arg("--json")
        .args(args)
        .output()
        .expect("Échec exécution binaire unleash");
    assert!(
        output.status.success(),
        "Commande `unleash {}` a échoué (status={}): stderr={}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("Sortie JSON CLI invalide")
}

#[tokio::test]
async fn e2e_m0_real_binaries_lifecycle_sigkill_and_clean_stop() {
    let rdv = local_test_dir("e2e-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("e2e-dat").unwrap();
    let sock = rdv.path().join("daemon.sock");
    assert!(
        sock.as_os_str().len() <= 103,
        "Socket E2E doit respecter SUN_LEN < 104 octets"
    );

    // 1. Démarrer le vrai binaire unleashd en processus fils
    let (mut child_1, ready_1) = spawn_unleashd(data.path(), rdv.path());
    let auth_1 = ready_1
        .get("authority_generation")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    assert_eq!(ready_1.get("daemon_epoch").and_then(Value::as_u64), Some(1));

    // 2. Appels réels au binaire unleash --json : status, health, snapshot
    let status_1 = run_cli_json(&sock, &["status"]);
    assert_eq!(status_1.get("mode").and_then(Value::as_str), Some("ready"));
    assert_eq!(
        status_1.get("authority_generation").and_then(Value::as_str),
        Some(auth_1.as_str())
    );

    let health_1 = run_cli_json(&sock, &["health"]);
    assert!(health_1.get("dead_letter").is_some());
    assert!(health_1.get("browser_runtimes").is_some());

    // Créer un projet et une mission via unleash CLI
    let proj_id = ProjectId::from_u128(0xE2E0_0001).to_string();
    let mission_id = MissionId::from_u128(0xE2E0_0002).to_string();
    let _proj_rcpt = run_cli_json(
        &sock,
        &[
            "project",
            "create",
            "--name",
            "E2E-M0",
            "--project-id",
            &proj_id,
        ],
    );
    let _assign_rcpt = run_cli_json(
        &sock,
        &[
            "mission",
            "assign",
            "--project-id",
            &proj_id,
            "--mission-id",
            &mission_id,
        ],
    );

    let snap_1 = run_cli_json(&sock, &["snapshot", "--project-id", &proj_id]);
    assert!(snap_1.get("watermark").is_some());

    // Pause -> Resume -> Cancel via unleash CLI --json
    let pause_cmd_id = Uuid::from_u128(0xE2E0_C001).to_string();
    let pause_rcpt = run_cli_json(
        &sock,
        &[
            "mission",
            "pause",
            &mission_id,
            "--project-id",
            &proj_id,
            "--expected-revision",
            "1",
            "--reason",
            "e2e pause",
            "--command-id",
            &pause_cmd_id,
        ],
    );
    assert_eq!(
        pause_rcpt.get("new_revision").and_then(Value::as_u64),
        Some(2)
    );

    let receipt_inspected = run_cli_json(&sock, &["receipt", &pause_cmd_id]);
    assert_eq!(
        receipt_inspected
            .get("lineage_status")
            .and_then(Value::as_str),
        Some("current")
    );
    assert_eq!(
        receipt_inspected
            .get("authority_generation")
            .and_then(Value::as_str),
        Some(auth_1.as_str())
    );

    let resume_rcpt = run_cli_json(
        &sock,
        &[
            "mission",
            "resume",
            &mission_id,
            "--project-id",
            &proj_id,
            "--expected-revision",
            "2",
            "--reason",
            "e2e resume",
        ],
    );
    assert_eq!(
        resume_rcpt.get("new_revision").and_then(Value::as_u64),
        Some(3)
    );

    // 3. Vérifier le refus d'un actor_ref forgé contre le vrai unleashd
    {
        let mut raw_client =
            UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
                .await
                .unwrap();
        let forged_err = raw_client
            .send_command(&CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0xE2E0_A001),
                command_id: CommandId::from_u128(0xE2E0_C999),
                authority_generation: raw_client.hello().authority_generation,
                actor_ref: Some(String::from("admin:forged-root")),
                project_id: Some(ProjectId::from_u128(0xE2E0_0001)),
                aggregate_id: Uuid::from_u128(0xE2E0_0002),
                expected_revision: Revision::new(3),
                command_type: String::from("CancelMission"),
                payload: match json!({ "reason": "forged" }) {
                    Value::Object(m) => m,
                    _ => Map::new(),
                },
            })
            .await
            .unwrap_err();
        assert!(matches!(
            forged_err,
            ClientError::Remote(ref e) if e.code == ErrorCode::AuthenticationFailed
        ));
    }

    // 4. Tuer unleashd par SIGKILL, relancer unleashd et vérifier reprise durable
    child_1.kill().expect("SIGKILL sur unleashd échoué");
    let killed_status = child_1.wait().expect("wait après SIGKILL échoué");
    assert!(!killed_status.success());

    let (mut child_2, ready_2) = spawn_unleashd(data.path(), rdv.path());
    assert_eq!(
        ready_2.get("authority_generation").and_then(Value::as_str),
        Some(auth_1.as_str())
    );
    assert_eq!(ready_2.get("daemon_epoch").and_then(Value::as_u64), Some(2));

    let status_2 = run_cli_json(&sock, &["status"]);
    assert_eq!(
        status_2.get("daemon_epoch").and_then(Value::as_u64),
        Some(2)
    );

    let receipt_after_crash = run_cli_json(&sock, &["receipt", &pause_cmd_id]);
    assert_eq!(
        receipt_after_crash
            .get("lineage_status")
            .and_then(Value::as_str),
        Some("current")
    );

    // Finaliser cancel mission après crash puis arrêt propre par `unleash daemon stop`
    let cancel_rcpt = run_cli_json(
        &sock,
        &[
            "mission",
            "cancel",
            &mission_id,
            "--project-id",
            &proj_id,
            "--expected-revision",
            "3",
            "--reason",
            "e2e cancel after crash",
        ],
    );
    assert_eq!(
        cancel_rcpt.get("new_revision").and_then(Value::as_u64),
        Some(4)
    );

    let snap_after = run_cli_json(&sock, &["snapshot", "--project-id", &proj_id]);
    let missions = snap_after
        .get("snapshot")
        .and_then(|s| s.get("missions"))
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(
        missions
            .first()
            .and_then(|m| m.get("state"))
            .and_then(Value::as_str),
        Some("cancelled")
    );

    let stop_out = run_cli_json(&sock, &["daemon", "stop"]);
    assert_eq!(
        stop_out.get("stop_outcome").and_then(Value::as_str),
        Some("complete")
    );

    let clean_exit = child_2.wait().expect("wait après daemon stop échoué");
    assert!(
        clean_exit.success(),
        "unleashd doit sortir proprement (code 0)"
    );
}
