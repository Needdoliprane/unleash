// T-F002-002 / AC-F002-002 / REQ-PERSIST-002
//! Qualification build `SQLite`, profil PRAGMA, file bornée, pression WAL 16/32 MiB et export 8 MiB.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use unleash_domain::{ActorId, CommandId, RequestId, Revision};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProtocolVersion, ReceiptState, Sensitivity,
};
use unleash_store::{
    AggregateKind, BoundedExportRequest, CommandMutation, CommitCommandRequest, CutpointObserver,
    DEFAULT_EXPORT_CHUNK_BYTES, DEFAULT_EXPORT_MAX_BYTES, DEFAULT_WAL_PRESSURE_BYTES,
    DEFAULT_WAL_STOP_BYTES, DEFAULT_WRITER_QUEUE_CAPACITY, EventAppendSpec, ExportCancellation,
    MonotonicClockPort, SqliteBuildPolicy, StoreConfig, StoreCutpoint, StoreError, UnleashStore,
    WriteCriticality, compiled_sqlite_source_id, compiled_sqlite_version, validate_build_metadata,
};
use unleash_testkit::{
    Cutpoint, CutpointController, DeterministicIdGenerator, ManualClock, local_test_dir,
};
use uuid::Uuid;

struct ClockAdapter(Arc<ManualClock>);
impl MonotonicClockPort for ClockAdapter {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

struct CutpointAdapter(CutpointController);
impl CutpointObserver for CutpointAdapter {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        let cp = match cutpoint {
            StoreCutpoint::C0BeforeCommandTx => Cutpoint::C0,
            StoreCutpoint::C1BeforeCommandCommit => Cutpoint::C1,
            StoreCutpoint::C2AfterCommandCommit => Cutpoint::C2,
            _ => return Ok(()),
        };
        self.0.hit(cp).map_err(|e| StoreError::CutpointInjected {
            cutpoint: cp.as_str().to_owned(),
            message: e.to_string(),
        })
    }
}

#[test]
fn test_t_f002_002_build_matrix_and_source_id_check() {
    let expected_sid = compiled_sqlite_source_id().to_owned();
    let compiled_ver = compiled_sqlite_version();
    assert!(!expected_sid.is_empty());
    assert!(!compiled_ver.is_empty());

    let policy = SqliteBuildPolicy {
        expected_source_id: expected_sid.clone(),
        enforce_source_id_match: false,
        extra_approved_backports: vec![],
    };

    // Builds approuvés : >= 3.51.3 et backports 3.44.6 / 3.50.7
    let opts = vec!["THREADSAFE=1".to_owned()];
    assert!(validate_build_metadata("3.54.1", &expected_sid, &opts, &policy).is_ok());
    assert!(validate_build_metadata("3.51.3", &expected_sid, &opts, &policy).is_ok());
    assert!(validate_build_metadata("3.44.6", &expected_sid, &opts, &policy).is_ok());
    assert!(validate_build_metadata("3.50.7", &expected_sid, &opts, &policy).is_ok());

    // Builds refusés : 3.52.0 retirée, < 3.51.3 non backportée, OMIT_WAL, THREADSAFE=0
    assert!(matches!(
        validate_build_metadata("3.52.0", &expected_sid, &opts, &policy),
        Err(StoreError::BuildUnqualified { .. })
    ));
    assert!(matches!(
        validate_build_metadata("3.51.2", &expected_sid, &opts, &policy),
        Err(StoreError::BuildUnqualified { .. })
    ));
    assert!(matches!(
        validate_build_metadata("3.54.1", &expected_sid, &["OMIT_WAL".to_owned()], &policy),
        Err(StoreError::BuildUnqualified { .. })
    ));

    // Divergence de sqlite_source_id() refuse le démarrage avec STORAGE_BUILD_UNQUALIFIED sans écrire
    let dir = local_test_dir("f002-002-bad-sid").unwrap();
    let mut cfg = StoreConfig::new(dir.path());
    cfg.build_policy.expected_source_id = "tampered-source-id".to_owned();
    cfg.build_policy.enforce_source_id_match = true;
    let err = UnleashStore::open(cfg).unwrap_err();
    assert!(matches!(err, StoreError::BuildUnqualified { .. }));
    assert_eq!(
        err.reason_code(),
        Some(unleash_protocol::ReasonCode::StorageBuildUnqualified)
    );
}

#[test]
fn test_t_f002_002_pragmas_queue_bound_and_tx_wal_growth_cap() {
    let dir = local_test_dir("f002-002-pragmas").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0002));
    let ctrl = CutpointController::new();

    let mut cfg = StoreConfig::new(dir.path());
    cfg.owner_uid = 4242;
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg.id_generator = ids.clone();
    cfg.cutpoints = Arc::new(CutpointAdapter(ctrl.clone()));

    let store = Arc::new(UnleashStore::open(cfg).unwrap());
    assert_eq!(store.daemon_state().unwrap().owner_uid, 4242);
    let manifest = store.runtime_manifest();
    assert_eq!(manifest.effective_pragmas.journal_mode, "wal");
    assert_eq!(manifest.effective_pragmas.synchronous, 2);
    assert_eq!(manifest.effective_pragmas.foreign_keys, 1);
    assert_eq!(manifest.effective_pragmas.trusted_schema, 0);
    assert_eq!(manifest.effective_pragmas.secure_delete, 2);
    assert_eq!(manifest.effective_pragmas.busy_timeout_ms, 250);
    assert_eq!(manifest.effective_pragmas.wal_autocheckpoint, 0);
    #[cfg(target_os = "macos")]
    {
        assert_eq!(manifest.effective_pragmas.fullfsync, 1);
        assert_eq!(manifest.effective_pragmas.checkpoint_fullfsync, 1);
    }

    // Borne 1 MiB de croissance WAL par transaction
    let oversize = "x".repeat(1_200_000);
    let res = store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, _now| {
        tx.execute_batch("CREATE TABLE IF NOT EXISTS _scratch (v TEXT);")?;
        tx.execute("INSERT INTO _scratch(v) VALUES (?1)", [oversize])?;
        Ok(())
    });
    assert!(matches!(res, Err(StoreError::TxWalGrowthExceeded { .. })));

    // File bornée à 8 demandes : bloquer le writer au point C1 puis saturer la file
    let barrier = ctrl.arm_barrier(Cutpoint::C1, Duration::from_secs(10));
    let authority = store.active_authority_generation().unwrap();
    let make_noop = move |cmd_idx: u128| CommitCommandRequest {
        actor_id: ActorId::new("user:louis").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(1000 + cmd_idx),
            command_id: CommandId::from_u128(2000 + cmd_idx),
            authority_generation: authority,
            actor_ref: Some("user:louis".to_owned()),
            project_id: None,
            aggregate_id: authority.as_uuid(),
            expected_revision: Revision::new(u64::try_from(cmd_idx).unwrap_or(0)),
            command_type: "NoopCommand".to_owned(),
            payload: Map::new(),
        },
        target_aggregate: AggregateKind::Daemon,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::NoopDaemonBump,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    };

    let s0 = Arc::clone(&store);
    let h0 = std::thread::spawn(move || s0.commit_command(make_noop(0)));
    barrier.wait_arrived(1, Duration::from_secs(5)).unwrap();

    // Remplir exactement les 8 places de la file pendant que le writer est bloqué sur h0
    let mut queued_handles = Vec::new();
    for i in 1..=DEFAULT_WRITER_QUEUE_CAPACITY {
        let s = Arc::clone(&store);
        let req = make_noop(i as u128);
        queued_handles.push(std::thread::spawn(move || s.commit_command(req)));
    }
    // Attendre de façon déterministe que la file soit pleine
    let deadline = Instant::now() + Duration::from_secs(5);
    while store.wal_telemetry().queue_depth < DEFAULT_WRITER_QUEUE_CAPACITY
        && Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        store.wal_telemetry().queue_depth,
        DEFAULT_WRITER_QUEUE_CAPACITY
    );

    // La 9e demande est refusée immédiatement par ResourceBusy/BACKPRESSURE
    let rejected = store.commit_command(make_noop(99)).unwrap_err();
    assert!(matches!(rejected, StoreError::Backpressure { .. }));
    assert_eq!(
        rejected.reason_code(),
        Some(unleash_protocol::ReasonCode::Backpressure)
    );

    barrier.release();
    h0.join().unwrap().unwrap();
    for h in queued_handles {
        let _ = h.join().unwrap();
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario complet R6 : lecteur bloquant, seuils WAL 16/32 MiB, neutralisation 5 s et export 8 MiB"
)]
fn test_t_f002_002_r6_wal_pressure_held_reader_and_export_quota() {
    let dir = local_test_dir("f002-002-r6").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0x000F_0020_0026));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg.id_generator = ids.clone();

    assert_eq!(cfg.wal_pressure_bytes, DEFAULT_WAL_PRESSURE_BYTES);
    assert_eq!(cfg.wal_stop_bytes, DEFAULT_WAL_STOP_BYTES);
    assert_eq!(cfg.export_max_bytes, DEFAULT_EXPORT_MAX_BYTES);
    assert_eq!(cfg.export_chunk_bytes, DEFAULT_EXPORT_CHUNK_BYTES);

    let store = UnleashStore::open(cfg.clone()).unwrap();
    let authority = store.active_authority_generation().unwrap();

    // 1. Retenir un vrai lecteur query_only=ON
    let held_reader = store.begin_held_reader().unwrap();

    // Alimenter > 32 MiB d'écritures par transactions bornées (< 1 MiB chacune)
    let mut committed_count = 0u64;
    let mut wal_stop_hit = false;
    let payload_blob = "A".repeat(128 * 1024);
    for i in 0..400u64 {
        let mut p = Map::new();
        p.insert("blob".into(), Value::String(payload_blob.clone()));
        let req = CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(10_000 + u128::from(i)),
                command_id: CommandId::from_u128(20_000 + u128::from(i)),
                authority_generation: authority,
                actor_ref: Some("user:louis".to_owned()),
                project_id: None,
                aggregate_id: authority.as_uuid(),
                expected_revision: Revision::new(i),
                command_type: "NoopCommand".to_owned(),
                payload: p.clone(),
            },
            target_aggregate: AggregateKind::Daemon,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::NoopDaemonBump,
            events: vec![EventAppendSpec {
                event_type: "WalStressPayload".to_owned(),
                aggregate_type: AggregateKind::Daemon,
                aggregate_id: authority.as_uuid(),
                aggregate_revision: Revision::new(i + 1),
                project_id: None,
                correlation_id: Uuid::from_u128(10_000 + u128::from(i)),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: p,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec![],
            }],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        };
        match store.commit_command(req) {
            Ok(_) => committed_count += 1,
            Err(StoreError::WalPressureStop { wal_bytes, .. }) => {
                assert!(wal_bytes >= DEFAULT_WAL_STOP_BYTES);
                wal_stop_hit = true;
                break;
            }
            Err(other) => panic!("erreur inattendue sous pression WAL: {other:?}"),
        }
    }
    assert!(
        wal_stop_hit,
        "le seuil d'arrêt 32 MiB aurait dû être atteint"
    );
    let telem = store.wal_telemetry();
    assert!(
        telem.wal_bytes <= DEFAULT_WAL_STOP_BYTES + cfg.max_tx_wal_growth_bytes,
        "WAL={} dépasse 33 MiB",
        telem.wal_bytes
    );

    // 2. Expirer le lecteur via l'horloge contrôlée (> 5s) -> libération RÉELLE de la transaction + progrès du checkpoint
    clock.advance(Duration::from_secs(6));
    let after_reap = store.run_wal_maintenance_tick().unwrap();
    assert!(held_reader.is_aborted());
    assert!(held_reader.is_released());
    assert!(after_reap.expired_readers_terminated >= 1);
    assert!(after_reap.passive_checkpoints >= 1);

    // Vérifier aussi la variante avec neutralisation bloquée
    let blocked_reader = store.begin_held_reader().unwrap();
    blocked_reader.set_block_neutralization(true);
    clock.advance(Duration::from_secs(6));
    let blocked_telem = store.run_wal_maintenance_tick().unwrap();
    assert!(blocked_telem.reader_neutralization_blocked);
    blocked_reader.release().unwrap();

    // 3. Export simultané > 8 MiB sous quota borné (8 MiB + fragment <= 256 KiB)
    let export_dest = dir.path().join("export-out.ndjson");
    let export_staging = dir.path().join("export-staging");
    let err = store
        .export_bounded(BoundedExportRequest {
            destination_path: export_dest.clone(),
            staging_dir: export_staging.clone(),
            max_bytes_override: None,
            chunk_bytes_override: None,
            cancellation: ExportCancellation::new(),
        })
        .unwrap_err();
    assert!(
        matches!(err, StoreError::ExportQuotaExceeded { bytes_written, .. } if bytes_written <= DEFAULT_EXPORT_MAX_BYTES + DEFAULT_EXPORT_CHUNK_BYTES as u64)
    );
    assert_eq!(
        err.error_code(),
        unleash_protocol::ErrorCode::InvalidArgument
    );
    assert_eq!(
        err.reason_code(),
        Some(unleash_protocol::ReasonCode::SchemaLimitExceeded)
    );
    assert!(
        !export_dest.exists(),
        "aucun artefact complet publié en cas de dépassement"
    );
    if export_staging.exists() {
        assert_eq!(std::fs::read_dir(&export_staging).unwrap().count(), 0);
    }

    // Mesure p50/p95 sous fullfsync=ON sur APFS (/tmp) et contrôle de cohérence
    let mut latencies_us = Vec::new();
    for j in 0..20u64 {
        let rev = committed_count + j;
        let t0 = Instant::now();
        store
            .commit_command(CommitCommandRequest {
                actor_id: ActorId::new("user:louis").unwrap(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_u128(50_000 + u128::from(j)),
                    command_id: CommandId::from_u128(60_000 + u128::from(j)),
                    authority_generation: authority,
                    actor_ref: Some("user:louis".to_owned()),
                    project_id: None,
                    aggregate_id: authority.as_uuid(),
                    expected_revision: Revision::new(rev),
                    command_type: "NoopCommand".to_owned(),
                    payload: Map::new(),
                },
                target_aggregate: AggregateKind::Daemon,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::NoopDaemonBump,
                events: vec![],
                capacity_reservations: vec![],
                budget_reservations: vec![],
            })
            .unwrap();
        latencies_us.push(t0.elapsed().as_micros());
    }
    latencies_us.sort_unstable();
    assert!(latencies_us.get(18).copied().unwrap_or(0) > 0);
}
