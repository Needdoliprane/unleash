// T-F001-010 / AC-F001-010 / REQ-CORE-010
//! Reprise `SubscribeEvents` par `(stream_id, sequence)` ou rechargement atomique
//! `GetStateSnapshot` + watermark sur `ResyncRequired` après purge ou restauration vers un nouveau stream.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use unleash_client::{
    ClientConfig, ClientError, EventFollowerConfig, EventSyncUpdate, EventSynchronizer,
    UnleashClient,
};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, StoreOverrides, UnleashDaemon};
use unleash_domain::{
    ActorId, CommandId, DaemonMode, ProjectId, RequestId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, SubscribeEventsFrame,
    SubscribeEventsRequest, compute_command_digest_v2,
};
use unleash_store::{
    EmptyExecutorNeutralizer, PrepareBackupRequest, RestoreBackupRequest, StoreConfig, UnleashStore,
};
use unleash_testkit::{ManualClock, local_test_dir};

#[tokio::test]
async fn t_f001_010_reconnect_purge_and_restore_trigger_resync_without_gaps() {
    // Partie A : Purge d'historique après rétention > 7 jours -> ResyncRequired + snapshot cohérent
    let rdv = local_test_dir("f001-10-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-10-dat").unwrap();
    let t0_ms: i64 = 1_700_000_000_000;
    let clock = Arc::new(ManualClock::new(t0_ms));

    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: StoreOverrides {
            clock: Some(clock.clone()),
            ..StoreOverrides::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };
    let sock = handle.socket_path().to_path_buf();

    let mut client =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
            .await
            .unwrap();
    let auth_1 = client.hello().authority_generation;
    let stream_1 = client.hello().stream_id;

    let mut sync = EventSynchronizer::new(
        ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"),
        EventFollowerConfig::default(),
    );
    let (_, stream) = sync
        .sync_initial(client.connection_mut(), RequestId::from_u128(0x1))
        .await
        .unwrap();
    let old_cursor = sync.current_cursor().unwrap();
    drop(stream);

    // Committer un événement à t0 puis avancer l'horloge de 8 jours et purger
    let p1 = ProjectId::from_u128(0x2A01);
    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x2),
            command_id: CommandId::from_u128(0xCA01),
            authority_generation: auth_1,
            actor_ref: None,
            project_id: Some(p1),
            aggregate_id: p1.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "BeforeBackup", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();
    drop(client);
    handle.shutdown_graceful("purge maintenance").await.unwrap();

    clock.advance(Duration::from_secs(8 * 86_400));
    let cutoff = TimestampMs::new(t0_ms + 86_400_000);
    let data_purge_path = data.path().to_path_buf();
    let clock_purge = clock.clone();
    let purged_rows = tokio::task::spawn_blocking(move || {
        let mut sc = StoreConfig::new(&data_purge_path);
        sc.clock = clock_purge;
        let store = UnleashStore::open(sc).unwrap();
        let stats = store.purge_history(cutoff).unwrap();
        store.shutdown().unwrap();
        stats.affected_rows
    })
    .await
    .unwrap();
    assert!(
        purged_rows >= 1,
        "La purge après 8 jours doit tombstoner au moins ProjectCreated"
    );

    // Redémarrer sur la même base après purge et vérifier ResyncRequired -> reprise continue en Ready
    let DaemonStartOutcome::Started(handle_purged) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: StoreOverrides {
            clock: Some(clock.clone()),
            ..StoreOverrides::default()
        },
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("Daemon post-purge attendu démarré");
    };
    let sock_purged = handle_purged.socket_path().to_path_buf();
    let mut client_purged = UnleashClient::connect(ClientConfig::new(
        sock_purged.clone(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();

    let mut purged_stale_stream = client_purged
        .subscribe_events(&unleash_protocol::SubscribeEventsRequest {
            stream_id: old_cursor.stream_id,
            after_sequence: old_cursor.sequence,
            project_id: None,
            aggregate_id: None,
            batch_limit: 16,
        })
        .await
        .unwrap();
    let mut purge_sync = EventSynchronizer::new(
        ClientConfig::new(sock_purged, ClientKind::Cli, "0.1.0"),
        EventFollowerConfig::default(),
    );
    let (purge_upd, purge_new_reader) = purge_sync
        .poll_next(
            &mut purged_stale_stream,
            client_purged.connection_mut(),
            || RequestId::from_u128(0x111),
        )
        .await
        .unwrap();
    let mut active_purge_reader = purge_new_reader.expect("Purge doit provoquer ResyncRequired");
    let wm_after_purge = match purge_upd {
        EventSyncUpdate::SnapshotLoaded {
            snapshot,
            watermark,
            resync_reason,
        } => {
            assert!(resync_reason.is_some());
            let projects = snapshot.get("projects").and_then(Value::as_array).unwrap();
            assert!(
                projects
                    .iter()
                    .any(|p| p.get("name").and_then(Value::as_str) == Some("BeforeBackup"))
            );
            watermark
        }
        other => panic!("Attendu SnapshotLoaded après purge, reçu {other:?}"),
    };

    let p_post_purge = ProjectId::from_u128(0x2A02);
    let post_purge_rcpt = client_purged
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x112),
            command_id: CommandId::from_u128(0xCA02),
            authority_generation: client_purged.hello().authority_generation,
            actor_ref: None,
            project_id: Some(p_post_purge),
            aggregate_id: p_post_purge.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "AfterPurge", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();
    assert!(post_purge_rcpt.event_cursor.sequence > wm_after_purge.sequence);

    let (ev_purge_upd, _) = purge_sync
        .poll_next(
            &mut active_purge_reader,
            client_purged.connection_mut(),
            || RequestId::from_u128(0x113),
        )
        .await
        .unwrap();
    match ev_purge_upd {
        EventSyncUpdate::Events { events, cursor } => {
            assert_eq!(cursor.stream_id, stream_1);
            assert_eq!(
                events.last().unwrap().sequence,
                post_purge_rcpt.event_cursor.sequence
            );
        }
        other => panic!("Attendu événement continu après purge, reçu {other:?}"),
    }
    drop(client_purged);
    drop(active_purge_reader);
    handle_purged
        .shutdown_graceful("backup & restore")
        .await
        .unwrap();

    // Partie B : Sauvegarde -> restauration offline (nouveau stream_id et mode read_only_recovery)
    let restored_data = local_test_dir("f001-10-rst").unwrap();
    let backup_dir_tmp = local_test_dir("f001-10-bkp").unwrap();
    let backup_dir = backup_dir_tmp.path().join("snapshot-1");
    let src_data_path = data.path().to_path_buf();
    let restored_path = restored_data.path().to_path_buf();
    let backup_dir_c = backup_dir.clone();
    let clock_bkp = clock.clone();
    let restore_res = tokio::task::spawn_blocking(move || {
        {
            let mut sc = StoreConfig::new(&src_data_path);
            sc.clock = clock_bkp.clone();
            let store = UnleashStore::open(sc).unwrap();
            let _bkp = store
                .prepare_backup(PrepareBackupRequest {
                    destination_dir: backup_dir_c.clone(),
                    retention_tier: unleash_store::BackupRetentionTier::Manual,
                    coverage: String::from("db_and_objects"),
                })
                .unwrap();
            store.shutdown().unwrap();
        }

        let restore_cmd_id = CommandId::from_u128(0xCE99);
        let dummy_env = CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x99),
            command_id: restore_cmd_id,
            authority_generation: auth_1,
            actor_ref: None,
            project_id: None,
            aggregate_id: restore_cmd_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("RestoreBackup"),
            payload: Map::new(),
        };
        let restore_digest: Sha256Digest = compute_command_digest_v2(&dummy_env).unwrap();

        let mut target_sc = StoreConfig::new(&restored_path);
        target_sc.clock = clock_bkp;
        UnleashStore::restore_backup(
            RestoreBackupRequest {
                backup_dir: backup_dir_c,
                target_data_root: restored_path.clone(),
                command_id: restore_cmd_id,
                actor_id: ActorId::new("uid:restore").unwrap().to_string(),
                command_digest: restore_digest,
                source_authority_generation: auth_1,
                source_db_unreadable: false,
                source_db_path: Some(src_data_path.join("unleash-authority.db")),
                source_artifacts_dir: Some(src_data_path.join("artifacts")),
                neutralizer: Arc::new(EmptyExecutorNeutralizer),
            },
            target_sc,
        )
        .unwrap()
    })
    .await
    .unwrap();
    assert_ne!(restore_res.target_stream_id, stream_1);

    // Redémarrer le daemon sur la base restaurée (en read_only_recovery conformément à runtime-lifecycle.md)
    let rdv_2 = local_test_dir("f001-10-r2").unwrap();
    std::fs::set_permissions(
        rdv_2.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let DaemonStartOutcome::Started(handle_2) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(restored_data.path().to_path_buf()),
        rendezvous_dir: Some(rdv_2.path().to_path_buf()),
        store_overrides: StoreOverrides {
            clock: Some(clock),
            ..StoreOverrides::default()
        },
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("Daemon restauré attendu démarré");
    };
    let sock_2 = handle_2.socket_path().to_path_buf();
    assert_eq!(handle_2.mode(), DaemonMode::ReadOnlyRecovery);
    assert_eq!(handle_2.stream_id(), restore_res.target_stream_id);

    let mut client_2 =
        UnleashClient::connect(ClientConfig::new(sock_2.clone(), ClientKind::Cli, "0.1.0"))
            .await
            .unwrap();

    // Un synchroniseur gardant l'ancien curseur sur `stream_1` se réabonne sur le nouveau daemon -> ResyncRequired!
    let mut stale_stream = client_2
        .subscribe_events(&unleash_protocol::SubscribeEventsRequest {
            stream_id: old_cursor.stream_id,
            after_sequence: old_cursor.sequence,
            project_id: None,
            aggregate_id: None,
            batch_limit: 16,
        })
        .await
        .unwrap();

    let mut resync_sync = EventSynchronizer::new(
        ClientConfig::new(sock_2, ClientKind::Cli, "0.1.0"),
        EventFollowerConfig::default(),
    );
    let (upd, new_reader) = resync_sync
        .poll_next(&mut stale_stream, client_2.connection_mut(), || {
            RequestId::from_u128(0x888)
        })
        .await
        .unwrap();
    let mut active_reader = new_reader.expect("ResyncRequired doit produire un nouveau flux");
    let new_wm = match upd {
        EventSyncUpdate::SnapshotLoaded {
            snapshot,
            watermark,
            resync_reason,
        } => {
            assert!(resync_reason.is_some());
            assert_eq!(watermark.stream_id, handle_2.stream_id());
            let projects = snapshot.get("projects").and_then(Value::as_array).unwrap();
            assert!(
                projects
                    .iter()
                    .any(|p| p.get("name").and_then(Value::as_str) == Some("AfterPurge"))
            );
            watermark
        }
        other => panic!("Attendu SnapshotLoaded après ResyncRequired, reçu {other:?}"),
    };

    // En mode read_only_recovery, toute écriture métier ordinaire est refusée
    let p_after = ProjectId::from_u128(0x2A99);
    let ro_err = client_2
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x901),
            command_id: CommandId::from_u128(0xCA99),
            authority_generation: client_2.hello().authority_generation,
            actor_ref: None,
            project_id: Some(p_after),
            aggregate_id: p_after.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "ForbiddenInReadOnly", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(
        ro_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::ResourceBusy
    ));

    // En revanche StopDaemon est autorisé en read_only_recovery et est reçu sans trou après `new_wm`
    let status = client_2
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x902),
            query_type: String::from("GetDaemonStatus"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    let daemon_rev = status
        .result
        .get("revision")
        .and_then(Value::as_u64)
        .unwrap();
    let auth_2 = client_2.hello().authority_generation;

    let stop_rcpt = client_2
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x903),
            command_id: CommandId::from_u128(0xCA9A),
            authority_generation: auth_2,
            actor_ref: None,
            project_id: None,
            aggregate_id: auth_2.as_uuid(),
            expected_revision: Revision::new(daemon_rev),
            command_type: String::from("StopDaemon"),
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert!(stop_rcpt.event_cursor.sequence > new_wm.sequence);

    let (ev_upd, _) = resync_sync
        .poll_next(&mut active_reader, client_2.connection_mut(), || {
            RequestId::from_u128(0x904)
        })
        .await
        .unwrap();
    match ev_purge_or_stop(ev_upd) {
        Some((cursor, last_seq, has_stop)) => {
            assert_eq!(cursor.stream_id, handle_2.stream_id());
            assert!(last_seq >= stop_rcpt.event_cursor.sequence);
            assert!(has_stop);
        }
        None => panic!("Attendu lot d'événements StopDaemon post-resync"),
    }

    assert!(handle_2.wait_stopped().await.is_some());
}

fn ev_purge_or_stop(
    upd: EventSyncUpdate,
) -> Option<(
    unleash_protocol::EventCursor,
    unleash_domain::Sequence,
    bool,
)> {
    match upd {
        EventSyncUpdate::Events { events, cursor } => {
            let last_seq = events.last()?.sequence;
            let has_stop = events
                .iter()
                .any(|e| e.event_type == "DaemonStopRequested" || e.event_type == "DaemonStopped");
            Some((cursor, last_seq, has_stop))
        }
        _ => None,
    }
}

#[tokio::test]
async fn t_f001_010_concurrent_commits_deliver_all_live_events_in_sequence_order() {
    let rdv = local_test_dir("f001-10c-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-10c-dat").unwrap();

    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        max_in_flight_mutations: 32,
        subscriber_queue_capacity: 64,
        store_overrides: StoreOverrides {
            writer_queue_capacity: Some(32),
            ..StoreOverrides::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };
    let sock = handle.socket_path().to_path_buf();
    let stream_id = handle.stream_id();
    let auth = handle.authority_generation();

    let mut admin =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
            .await
            .unwrap();
    let status = admin
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xA00),
            query_type: String::from("GetDaemonStatus"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    let start_seq = status.watermark.unwrap().sequence;

    let mut reader = admin
        .subscribe_events(&SubscribeEventsRequest {
            stream_id,
            after_sequence: start_seq,
            project_id: None,
            aggregate_id: None,
            batch_limit: 4,
        })
        .await
        .unwrap();

    // Lancer 12 mutations concurrentes depuis des connexions parallèles synchronisées après Hello
    let total = 12_u128;
    let barrier = Arc::new(tokio::sync::Barrier::new(total as usize));
    let mut expected_pids = std::collections::HashSet::new();
    let mut join_set = tokio::task::JoinSet::new();
    for idx in 1..=total {
        let pid = ProjectId::from_u128(0xB000 + idx);
        expected_pids.insert(pid);
        let sock_c = sock.clone();
        let barrier_c = Arc::clone(&barrier);
        join_set.spawn(async move {
            let mut c = UnleashClient::connect(ClientConfig::new(sock_c, ClientKind::Cli, "0.1.0"))
                .await
                .unwrap();
            barrier_c.wait().await;
            c.send_command(&CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0xC000 + idx),
                command_id: CommandId::from_u128(0xD000 + idx),
                authority_generation: auth,
                actor_ref: None,
                project_id: Some(pid),
                aggregate_id: pid.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: String::from("CreateProject"),
                payload: match json!({ "name": format!("Conc-{idx}"), "weight": 1 }) {
                    Value::Object(m) => m,
                    _ => Map::new(),
                },
            })
            .await
            .unwrap()
        });
    }
    while let Some(res) = join_set.join_next().await {
        res.unwrap();
    }

    let mut received_pids = Vec::new();
    let mut prev_seq = start_seq.as_u64();
    tokio::time::timeout(Duration::from_secs(10), async {
        while received_pids.len() < total as usize {
            let Some(frame) = reader.next_frame().await.unwrap() else {
                panic!("Flux SubscribeEvents fermé avant réception complète");
            };
            if let SubscribeEventsFrame::Events { events, .. } = frame {
                for ev in events {
                    let seq = ev.sequence.as_u64();
                    assert!(
                        seq > prev_seq,
                        "Séquence non strictement croissante : {seq} après {prev_seq}"
                    );
                    prev_seq = seq;
                    if ev.event_type == "ProjectCreated" {
                        received_pids.push(ev.project_id.unwrap());
                    }
                }
            }
        }
    })
    .await
    .expect("timeout réception de tous les événements concurrents");

    let received_set: std::collections::HashSet<ProjectId> =
        received_pids.iter().copied().collect();
    assert_eq!(received_pids.len(), total as usize);
    assert_eq!(received_set, expected_pids);

    drop(reader);
    handle.shutdown_graceful("done-concurrent").await.unwrap();
}
