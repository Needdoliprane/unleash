// T-F001-001 / AC-F001-001 / REQ-CORE-001
//! Barrière d'autorité unique par UID effectif, indépendante du profil, de `data_root`,
//! de `XDG_RUNTIME_DIR` et de `TMPDIR`, et blocage de bascule de racine tant qu'un host vit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Map, Value};
use unleash_client::{ClientConfig, UnleashClient};
use unleash_daemon::{
    DaemonConfig, DaemonError, DaemonStartOutcome, DrainReport, ExecutorDrainPort, UnleashDaemon,
};
use unleash_domain::{RequestId, StopOutcome};
use unleash_platform::{EnvSnapshot, PlatformOsKind, canonical_rendezvous_dir_with, effective_uid};
use unleash_protocol::{ClientKind, QueryEnvelope};
use unleash_testkit::local_test_dir;

struct SwitchTestController {
    old_root: PathBuf,
    host_alive: Arc<AtomicBool>,
}

impl ExecutorDrainPort for SwitchTestController {
    fn active_hosts_for_other_root(
        &self,
        _rendezvous_dir: &Path,
        requested_root: &Path,
    ) -> Result<Option<(PathBuf, Vec<String>)>, DaemonError> {
        if requested_root != self.old_root && self.host_alive.load(Ordering::Acquire) {
            return Ok(Some((
                self.old_root.clone(),
                vec![String::from("host-legacy-1")],
            )));
        }
        Ok(None)
    }

    fn drain_on_stop(&self, _rendezvous_dir: &Path, _timeout: Duration) -> DrainReport {
        DrainReport {
            outcome: if self.host_alive.load(Ordering::Acquire) {
                StopOutcome::Partial
            } else {
                StopOutcome::Complete
            },
            retained_services: if self.host_alive.load(Ordering::Acquire) {
                vec![String::from("host-legacy-1")]
            } else {
                Vec::new()
            },
            unknown_services: Vec::new(),
        }
    }
}

#[tokio::test]
async fn t_f001_001_single_authority_ten_clients_two_roots_and_host_switch() {
    // 1. Immunité du chemin de rendez-vous aux surcharges XDG_RUNTIME_DIR et TMPDIR
    let uid = effective_uid();
    let env_a = EnvSnapshot {
        home: Some(PathBuf::from("/Users/test-a")),
        xdg_runtime_dir: Some(String::from("/tmp/custom-xdg-a")),
        tmpdir: Some(String::from("/tmp/custom-tmp-a")),
        ..EnvSnapshot::default()
    };
    let env_b = EnvSnapshot {
        home: Some(PathBuf::from("/Users/test-b")),
        xdg_runtime_dir: Some(String::from("/tmp/custom-xdg-b")),
        tmpdir: Some(String::from("/tmp/custom-tmp-b")),
        ..EnvSnapshot::default()
    };
    let rdv_a = canonical_rendezvous_dir_with(PlatformOsKind::Linux, uid, &env_a).unwrap();
    let rdv_b = canonical_rendezvous_dir_with(PlatformOsKind::Linux, uid, &env_b).unwrap();
    assert_eq!(rdv_a, rdv_b);

    // 2. Démarrer l'unique autorité sur data_root_1
    let rdv_tmp = local_test_dir("f001-01-rdv").unwrap();
    std::fs::set_permissions(
        rdv_tmp.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let root_1_tmp = local_test_dir("f001-01-d1").unwrap();
    let root_2_tmp = local_test_dir("f001-01-d2").unwrap();
    let root_1 = std::fs::canonicalize(root_1_tmp.path()).unwrap();
    let root_2 = std::fs::canonicalize(root_2_tmp.path()).unwrap();

    let host_alive = Arc::new(AtomicBool::new(true));
    let controller = Arc::new(SwitchTestController {
        old_root: root_1.clone(),
        host_alive: Arc::clone(&host_alive),
    });

    let cfg_1 = DaemonConfig {
        data_root: Some(root_1.clone()),
        rendezvous_dir: Some(rdv_tmp.path().to_path_buf()),
        executor_controller: controller.clone(),
        ..DaemonConfig::default()
    };

    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg_1).await.unwrap() else {
        panic!("Le premier démarrage doit réussir");
    };
    let expected_gen = handle.authority_generation();

    // 3. Deuxième démarrage sur root_2 doit échouer avec AuthorityAlreadyActive
    //    et ne jamais ouvrir/créer de base SQLite dans root_2.
    let cfg_2 = DaemonConfig {
        data_root: Some(root_2.clone()),
        rendezvous_dir: Some(rdv_tmp.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let err_other_root = UnleashDaemon::start(cfg_2).await.unwrap_err();
    assert!(
        matches!(err_other_root, DaemonError::AuthorityAlreadyActive { .. }),
        "Attendu AuthorityAlreadyActive, reçu {err_other_root:?}"
    );
    assert!(
        !root_2.join("unleash-authority.db").exists(),
        "Aucune base SQLite ne doit être créée sur la seconde racine refusée"
    );

    // 4. Deuxième démarrage sur root_1 avec attach_if_running=false -> DaemonAlreadyRunning
    let cfg_same_strict = DaemonConfig {
        data_root: Some(root_1.clone()),
        rendezvous_dir: Some(rdv_tmp.path().to_path_buf()),
        attach_if_running: false,
        ..DaemonConfig::default()
    };
    let err_same = UnleashDaemon::start(cfg_same_strict).await.unwrap_err();
    assert!(matches!(err_same, DaemonError::DaemonAlreadyRunning { .. }));

    // 5. Même racine avec attach_if_running=true rejoint l'instance via handshake Hello
    let cfg_same_attach = DaemonConfig {
        data_root: Some(root_1.clone()),
        rendezvous_dir: Some(rdv_tmp.path().to_path_buf()),
        attach_if_running: true,
        ..DaemonConfig::default()
    };
    match UnleashDaemon::start(cfg_same_attach).await.unwrap() {
        DaemonStartOutcome::AlreadyRunning { hello, .. } => {
            assert_eq!(hello.authority_generation, expected_gen);
        }
        other => panic!("Attendu AlreadyRunning, reçu {other:?}"),
    }

    // 6. Dix clients concurrents (deux profils Cli/Gui) convergent vers la même autorité
    let mut join_set = tokio::task::JoinSet::new();
    for idx in 0..10_u128 {
        let sock = handle.socket_path().to_path_buf();
        let kind = if idx % 2 == 0 {
            ClientKind::Cli
        } else {
            ClientKind::Gui
        };
        join_set.spawn(async move {
            let mut client = UnleashClient::connect(ClientConfig::new(sock, kind, "0.1.0")).await?;
            let q = QueryEnvelope {
                protocol_version: unleash_protocol::ProtocolVersion,
                request_id: RequestId::from_u128(0x1000 + idx),
                query_type: String::from("GetDaemonStatus"),
                project_id: None,
                payload: Map::new(),
            };
            let resp = client.query(&q).await?;
            Ok::<_, unleash_client::ClientError>((client.hello().authority_generation, resp.result))
        });
    }

    let mut observed_gens = HashSet::new();
    while let Some(res) = join_set.join_next().await {
        let (auth_gen, status): (_, Value) = res.unwrap().unwrap();
        observed_gens.insert(auth_gen);
        assert_eq!(
            status.get("authority_generation").and_then(Value::as_str),
            Some(expected_gen.to_string().as_str())
        );
    }
    assert_eq!(observed_gens.len(), 1);
    assert!(observed_gens.contains(&expected_gen));

    // 7. Arrêt du premier daemon avec host vivant -> StopOutcome::Partial
    let drain = handle.shutdown_graceful("switch root test").await.unwrap();
    assert_eq!(drain.outcome, StopOutcome::Partial);

    // 8. Changement de racine vers root_2 refusé tant que host-legacy-1 est vivant
    let cfg_switch_blocked = DaemonConfig {
        data_root: Some(root_2.clone()),
        rendezvous_dir: Some(rdv_tmp.path().to_path_buf()),
        executor_controller: controller.clone(),
        ..DaemonConfig::default()
    };
    let switch_err = UnleashDaemon::start(cfg_switch_blocked).await.unwrap_err();
    assert!(
        matches!(switch_err, DaemonError::AuthoritySwitchBlocked { .. }),
        "Attendu AuthoritySwitchBlocked, reçu {switch_err:?}"
    );
    assert!(!root_2.join("unleash-authority.db").exists());

    // 9. Neutraliser le host puis changer de racine vers root_2 -> succès
    host_alive.store(false, Ordering::Release);
    let cfg_switch_ok = DaemonConfig {
        data_root: Some(root_2.clone()),
        rendezvous_dir: Some(rdv_tmp.path().to_path_buf()),
        executor_controller: controller,
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle_2) = UnleashDaemon::start(cfg_switch_ok).await.unwrap()
    else {
        panic!("Le démarrage sur root_2 après neutralisation doit réussir");
    };
    assert_ne!(handle_2.authority_generation(), expected_gen);
    let final_drain = handle_2.shutdown_graceful("cleanup").await.unwrap();
    assert_eq!(final_drain.outcome, StopOutcome::Complete);
}
