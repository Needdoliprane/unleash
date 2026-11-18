// T-F018-005/007 / AC-F018-005/007 / REQ-REC-005/007
//! Vrais binaires daemon/CLI et IPC : restauration offline, lignée épinglée et reçu historique.
//! Les seules mutations sont des projets locaux ; aucun exécuteur ni secret fournisseur.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_domain::{AuthorityGeneration, CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceiptInspection, ErrorCode, LineageStatus,
    ProtocolVersion, QueryEnvelope, ReasonCode, ReceiptState, canonical_sha256,
};
use unleash_store::{
    BackupRetentionTier, CapsuleCoverage, ExecutorInventoryItem, ExecutorNeutralizer,
    PrepareBackupRequest, RestoreBackupRequest, RestoreCapsule, StoreConfig, StoreError,
    UnleashStore,
};
use unleash_testkit::local_test_dir;

const PROCESS_TIMEOUT: Duration = Duration::from_secs(20);

/// Owns only the child created by this fixture, including cleanup after a failed assertion.
struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn wait_clean(&mut self) {
        let status = self.0.take().unwrap().wait().unwrap();
        assert!(status.success(), "fixture child failed: {status}");
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

fn spawn_daemon(data: &Path, rendezvous: &Path) -> (ChildGuard, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_unleashd"))
        .arg("--data-root")
        .arg(data)
        .arg("--rendezvous-dir")
        .arg(rendezvous)
        .args(["--foreground", "--ready-json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let guard = ChildGuard(Some(child));
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    let line = rx
        .recv_timeout(PROCESS_TIMEOUT)
        .expect("daemon ready timeout")
        .unwrap();
    let ready: Value = serde_json::from_str(&line).expect("daemon ready JSON");
    assert_eq!(ready["status"], "ready");
    (guard, ready)
}

fn run_cli_json(socket: &Path, args: &[&str]) -> Value {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_unleashd"))
        .with_file_name(format!("unleash{}", std::env::consts::EXE_SUFFIX));
    assert!(cli.is_file(), "build unleash-cli before this E2E");
    let mut child = Command::new(cli)
        .arg("--socket")
        .arg(socket)
        .arg("--json")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut guard = ChildGuard(Some(child));
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout.read_to_end(&mut output).map(|_| output);
        let _ = tx.send(result);
    });
    let output = rx
        .recv_timeout(PROCESS_TIMEOUT)
        .expect("CLI output timeout")
        .unwrap();
    guard.wait_clean();
    serde_json::from_slice(&output).expect("CLI JSON")
}

fn stop_daemon(child: &mut ChildGuard, socket: &Path) {
    let stop = run_cli_json(socket, &["daemon", "stop"]);
    assert_eq!(stop["stop_outcome"], "complete");
    assert_eq!(stop["retained_services"], json!([]));
    assert_eq!(stop["unknown_services"], json!([]));
    child.wait_clean();
    assert!(!socket.exists(), "source socket survived clean stop");
}

/// An empty inventory is returned only after inspecting the isolated source's actual state.
struct CheckedNoExecutors {
    source_db: PathBuf,
    rendezvous: PathBuf,
}

impl ExecutorNeutralizer for CheckedNoExecutors {
    fn inventory_and_neutralize(
        &self,
        generation: AuthorityGeneration,
    ) -> Result<Vec<ExecutorInventoryItem>, StoreError> {
        let check = || -> Result<(), String> {
            if self.rendezvous.join("daemon.sock").exists() {
                return Err("source socket is still present".into());
            }
            let manifest = self.rendezvous.join("hosts.json");
            if manifest.exists() {
                let value: Value = serde_json::from_slice(
                    &std::fs::read(manifest).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                if !value["executors"].as_array().is_some_and(Vec::is_empty) {
                    return Err("source host inventory is not empty".into());
                }
            }
            let connection =
                Connection::open_with_flags(&self.source_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|error| error.to_string())?;
            let observed: String = connection
                .query_row("SELECT authority_generation FROM daemon_state", [], |row| {
                    row.get(0)
                })
                .map_err(|error| error.to_string())?;
            if observed != generation.to_string() {
                return Err("source authority changed".into());
            }
            for table in [
                "processes",
                "actions",
                "action_dispatches",
                "invocations",
                "replay_workers",
            ] {
                let count: i64 = connection
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .map_err(|error| error.to_string())?;
                if count != 0 {
                    return Err(format!("source inventory contains {count} {table}"));
                }
            }
            Ok(())
        };
        check().map_err(StoreError::RestorationBlocked)?;
        Ok(Vec::new())
    }
}

fn projects(snapshot: &Value) -> &Vec<Value> {
    snapshot["snapshot"]["projects"].as_array().unwrap()
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Real lifecycle preserves the pinned envelope across backup, restore and reconnect"
)]
fn e2e_m1_restore_preserves_lineage_receipt_and_prevents_second_effect() {
    // Store's blocking writer APIs run outside a Tokio context; only IPC futures enter it.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = local_test_dir("m1-restore-e2e").unwrap();
    let source = fixture.path().join("source");
    let target = fixture.path().join("target");
    let backup = fixture.path().join("backup");
    let rendezvous = fixture.path().join("rdv");
    std::fs::create_dir(&rendezvous).unwrap();
    std::fs::set_permissions(&rendezvous, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = rendezvous.join("daemon.sock");
    let pre_project = ProjectId::from_u128(0xE2E1_0001);
    let post_project = ProjectId::from_u128(0xE2E1_0002);
    let pre_id = pre_project.to_string();

    let (mut first, ready) = spawn_daemon(&source, &rendezvous);
    let source_generation: AuthorityGeneration = ready["authority_generation"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let pre_receipt = run_cli_json(
        &socket,
        &[
            "project",
            "create",
            "--name",
            "before-backup",
            "--project-id",
            &pre_id,
        ],
    );
    stop_daemon(&mut first, &socket);

    let source_config = StoreConfig::new(&source);
    let store = UnleashStore::open(source_config.clone()).unwrap();
    let manifest = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: backup.clone(),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".into(),
        })
        .unwrap();
    assert_eq!(manifest.source_authority_generation, source_generation);
    let validated = UnleashStore::validate_backup(&backup, &source_config).unwrap();
    assert!(validated.integrity_check_ok);
    assert_eq!(validated.foreign_key_violations, 0);
    store.shutdown().unwrap();

    let (mut second, ready) = spawn_daemon(&source, &rendezvous);
    assert_eq!(ready["authority_generation"], source_generation.to_string());
    let mut source_client = runtime
        .block_on(UnleashClient::connect(ClientConfig::new(
            socket.clone(),
            ClientKind::Cli,
            "m1-e2e",
        )))
        .unwrap();
    let pinned = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xE2E1_A001),
        command_id: CommandId::from_u128(0xE2E1_C001),
        authority_generation: source_generation,
        actor_ref: None,
        project_id: Some(post_project),
        aggregate_id: post_project.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "CreateProject".into(),
        payload: json!({"name":"after-backup", "weight":1})
            .as_object()
            .unwrap()
            .clone(),
    };
    let post_receipt = runtime
        .block_on(source_client.send_command(&pinned))
        .unwrap();
    assert_eq!(post_receipt.state, ReceiptState::Accepted);
    assert_eq!(projects(&run_cli_json(&socket, &["snapshot"])).len(), 2);
    let source_inspection = run_cli_json(&socket, &["receipt", &pinned.command_id.to_string()]);
    assert_eq!(source_inspection["lineage_status"], "current");
    assert_eq!(
        source_inspection["receipt"]["receipt_id"],
        post_receipt.receipt_id.to_string()
    );
    drop(source_client);
    stop_daemon(&mut second, &socket);

    let target_config = StoreConfig::new(&target);
    let activated = UnleashStore::restore_backup(RestoreBackupRequest {
        backup_dir: backup,
        target_data_root: target.clone(),
        command_id: CommandId::from_u128(0xE2E1_C002),
        actor_id: format!("uid:{}", unleash_platform::effective_uid()),
        command_digest: canonical_sha256(&json!({"operation":"RestoreBackup", "backup_id":manifest.backup_id, "target":target})).unwrap(),
        source_authority_generation: source_generation,
        source_db_unreadable: false,
        source_db_path: Some(source_config.db_path.clone()),
        source_artifacts_dir: Some(source_config.artifacts_dir.clone()),
        neutralizer: Arc::new(CheckedNoExecutors { source_db: source_config.db_path, rendezvous: rendezvous.clone() }),
    }, target_config.clone()).unwrap();
    assert_ne!(activated.target_authority_generation, source_generation);
    assert_eq!(activated.coverage, CapsuleCoverage::Complete);

    // The capsule projection is observable independently and cannot substitute for a full receipt.
    let restored = UnleashStore::open(target_config.clone()).unwrap();
    let capsule: RestoreCapsule = serde_json::from_slice(
        &restored
            .read_verified_object_bytes(activated.capsule_object_hash)
            .unwrap(),
    )
    .unwrap();
    assert!(
        capsule
            .receipts
            .iter()
            .any(|receipt| receipt.command_id == pinned.command_id
                && receipt.receipt_id == post_receipt.receipt_id)
    );
    restored.shutdown().unwrap();

    let (mut third, ready) = spawn_daemon(&target, &rendezvous);
    assert_eq!(
        ready["authority_generation"],
        activated.target_authority_generation.to_string()
    );
    let before = run_cli_json(&socket, &["snapshot"]);
    assert_eq!(projects(&before).len(), 1);
    assert_eq!(projects(&before).first().unwrap()["project_id"], pre_id);
    let mut client = runtime
        .block_on(UnleashClient::connect(ClientConfig::new(
            socket.clone(),
            ClientKind::Cli,
            "m1-e2e",
        )))
        .unwrap();
    let error = runtime.block_on(client.send_command(&pinned)).unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(ref error) if error.code == ErrorCode::StaleFence && error.reason_code == Some(ReasonCode::StaleAuthorityGeneration)),
        "old pinned command must fail before receipt lookup: {error:?}"
    );
    let after = run_cli_json(&socket, &["snapshot"]);
    assert_eq!(
        projects(&before),
        projects(&after),
        "old command produced a second project effect"
    );
    let connection =
        Connection::open_with_flags(&target_config.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let dispatches: i64 = connection
        .query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(dispatches, 0);
    drop(connection);
    let historical_pre = run_cli_json(
        &socket,
        &["receipt", pre_receipt["command_id"].as_str().unwrap()],
    );
    assert_eq!(historical_pre["lineage_status"], "historical");
    assert_eq!(
        historical_pre["authority_generation"],
        source_generation.to_string()
    );

    let historical_post = runtime.block_on(
        client.query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xE2E1_A002),
            query_type: "GetCommandReceipt".into(),
            project_id: None,
            payload: json!({"command_id":pinned.command_id})
                .as_object()
                .unwrap()
                .clone(),
        }),
    );
    drop(client);
    stop_daemon(&mut third, &socket);
    let response = historical_post.expect("POST-SNAPSHOT FULL RECEIPT must remain inspectable after restore; capsule metadata is insufficient");
    let inspected: CommandReceiptInspection = serde_json::from_value(response.result).unwrap();
    assert_eq!(inspected.lineage_status, LineageStatus::Historical);
    assert_eq!(inspected.authority_generation, source_generation);
    assert_eq!(inspected.receipt.receipt_id, post_receipt.receipt_id);
    assert_eq!(
        inspected.receipt, post_receipt,
        "historical response was reconstructed or altered"
    );
}
