//! Spike 1 `SQLite` M0 : qualification build, latences `fullfsync=ON` sur APFS local,
//! pression WAL 16/32 MiB, expiration lecteur 5 s et export borné 8 MiB.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use unleash_domain::{ActorId, IdGeneratorPort, RequestId, Revision};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProtocolVersion, ReceiptState, Sensitivity,
};
use unleash_store::{
    AggregateKind, BackupRetentionTier, BoundedExportRequest, CommandMutation,
    CommitCommandRequest, DEFAULT_EXPORT_CHUNK_BYTES, DEFAULT_EXPORT_MAX_BYTES,
    DEFAULT_WAL_PRESSURE_BYTES, DEFAULT_WAL_STOP_BYTES, EventAppendSpec, ExportCancellation,
    MonotonicClockPort, PrepareBackupRequest, StoreConfig, StoreError, UnleashStore,
    WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};
use uuid::Uuid;

struct ClockAdapter(Arc<ManualClock>);

impl MonotonicClockPort for ClockAdapter {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

fn percentile_us(sorted: &[u128], num: usize, den: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = sorted.len().saturating_sub(1).saturating_mul(num) / den.max(1);
    sorted.get(idx).copied().unwrap_or(0)
}

fn build_spike_noop_cmd(
    ids: &DeterministicIdGenerator,
    authority: unleash_domain::AuthorityGeneration,
    expected_revision: Revision,
    event_type: &str,
    payload: Map<String, Value>,
    outbox_destinations: Vec<String>,
) -> Result<CommitCommandRequest, Box<dyn std::error::Error>> {
    Ok(CommitCommandRequest {
        actor_id: ActorId::new("user:louis")?,
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(ids.new_uuid()),
            command_id: ids.new_command_id(),
            authority_generation: authority,
            actor_ref: Some("user:louis".to_owned()),
            project_id: None,
            aggregate_id: authority.as_uuid(),
            expected_revision,
            command_type: "NoopCommand".to_owned(),
            payload: payload.clone(),
        },
        target_aggregate: AggregateKind::Daemon,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::NoopDaemonBump,
        events: vec![EventAppendSpec {
            event_type: event_type.to_owned(),
            aggregate_type: AggregateKind::Daemon,
            aggregate_id: authority.as_uuid(),
            aggregate_revision: Revision::new(expected_revision.as_u64() + 1),
            project_id: None,
            correlation_id: Uuid::from_u128(u128::from(expected_revision.as_u64()) + 1),
            causation_kind: EventCausationKind::Command,
            causation_id: None,
            payload,
            sensitivity: Sensitivity::Project,
            outbox_destinations,
        }],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    })
}

fn print_build_manifest(
    out: &mut io::StdoutLock<'_>,
    bench_path: &std::path::Path,
    manifest: &unleash_store::SqliteRuntimeManifest,
) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(out, "=== SQLite Build & PRAGMA Manifest ===")?;
    writeln!(out, "bench_path={}", bench_path.display())?;
    writeln!(out, "sqlite_version={}", manifest.sqlite_version)?;
    writeln!(out, "sqlite_source_id={}", manifest.sqlite_source_id)?;
    writeln!(out, "expected_source_id={}", manifest.expected_source_id)?;
    writeln!(
        out,
        "compile_options_count={}",
        manifest.compile_options.len()
    )?;
    for opt in &manifest.compile_options {
        writeln!(out, "compile_option={opt}")?;
    }
    writeln!(
        out,
        "pragmas=journal_mode:{} synchronous:{} foreign_keys:{} trusted_schema:{} secure_delete:{} busy_timeout_ms:{} wal_autocheckpoint:{} fullfsync:{} checkpoint_fullfsync:{} page_size:{}",
        manifest.effective_pragmas.journal_mode,
        manifest.effective_pragmas.synchronous,
        manifest.effective_pragmas.foreign_keys,
        manifest.effective_pragmas.trusted_schema,
        manifest.effective_pragmas.secure_delete,
        manifest.effective_pragmas.busy_timeout_ms,
        manifest.effective_pragmas.wal_autocheckpoint,
        manifest.effective_pragmas.fullfsync,
        manifest.effective_pragmas.checkpoint_fullfsync,
        manifest.effective_pragmas.page_size,
    )?;
    Ok(())
}

fn run_latency_benchmark(out: &mut io::StdoutLock<'_>) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Qualification build & benchmark latences durable CommitCommand (fullfsync=ON sur APFS local)
    let bench_dir = local_test_dir("spike-sqlite-bench")?;
    let bench_store = UnleashStore::open(StoreConfig::new(bench_dir.path()))?;
    let manifest = bench_store.runtime_manifest().clone();
    let auth_b = bench_store.active_authority_generation()?;
    let ids_b = DeterministicIdGenerator::new(0x511_CAFE);
    print_build_manifest(out, bench_dir.path(), &manifest)?;

    let mut commit_latencies_us = Vec::with_capacity(200);
    for i in 0..200_u64 {
        let mut p = Map::new();
        p.insert("seq".into(), Value::from(i));
        let req = build_spike_noop_cmd(
            &ids_b,
            auth_b,
            Revision::new(i),
            "SpikePing",
            p,
            vec!["spike-dest".to_owned()],
        )?;
        let t0 = Instant::now();
        let _ = bench_store.commit_command(req)?;
        commit_latencies_us.push(t0.elapsed().as_micros());
    }
    commit_latencies_us.sort_unstable();
    let total_us: u128 = commit_latencies_us.iter().copied().sum();
    let mean_us = total_us / u128::try_from(commit_latencies_us.len()).unwrap_or(1);

    let bkp_t0 = Instant::now();
    let bkp = bench_store.prepare_backup(PrepareBackupRequest {
        destination_dir: bench_dir.path().join("backups/spike-bkp"),
        retention_tier: BackupRetentionTier::Manual,
        coverage: "db_and_objects".to_owned(),
    })?;
    let bkp_us = bkp_t0.elapsed().as_micros();
    let val_t0 = Instant::now();
    let val = UnleashStore::validate_backup(
        &bench_dir.path().join("backups/spike-bkp"),
        bench_store.config(),
    )?;
    let val_us = val_t0.elapsed().as_micros();

    writeln!(out, "=== APFS fullfsync=ON CommitCommand (N=200) ===")?;
    writeln!(
        out,
        "commit_us min={} mean={} p50={} p95={} p99={} max={}",
        commit_latencies_us.first().copied().unwrap_or(0),
        mean_us,
        percentile_us(&commit_latencies_us, 50, 100),
        percentile_us(&commit_latencies_us, 95, 100),
        percentile_us(&commit_latencies_us, 99, 100),
        commit_latencies_us.last().copied().unwrap_or(0),
    )?;
    writeln!(
        out,
        "backup_prepare_us={} backup_validate_us={} integrity_ok={} fk_violations={}",
        bkp_us, val_us, val.integrity_check_ok, val.foreign_key_violations
    )?;
    writeln!(out, "backup_id={}", bkp.backup_id)?;
    bench_store.shutdown()?;
    Ok(())
}

fn run_wal_and_export_spike(
    out: &mut io::StdoutLock<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    // 2. Pression WAL 16 MiB PASSIVE / 32 MiB STOP, readers 5 s, cap tx 1 MiB, export 8 MiB
    let wal_dir = local_test_dir("spike-sqlite-wal")?;
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0x511_BEEF));
    let mut cfg = StoreConfig::new(wal_dir.path());
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg.id_generator = ids.clone();

    let store = UnleashStore::open(cfg)?;
    let authority = store.active_authority_generation()?;

    let cap_err = store.execute_in_writer_tx(WriteCriticality::Normal, |tx, _| {
        tx.execute_batch("CREATE TABLE IF NOT EXISTS _cap(v TEXT);")?;
        tx.execute("INSERT INTO _cap(v) VALUES (?1)", ["Y".repeat(1_200_000)])?;
        Ok(())
    });
    writeln!(
        out,
        "tx_wal_1mib_cap_rejected={}",
        matches!(cap_err, Err(StoreError::TxWalGrowthExceeded { .. }))
    )?;

    let held = store.begin_held_reader()?;
    let blob = "B".repeat(128 * 1024);
    let mut committed = 0_u64;
    let mut stop_wal_bytes = 0_u64;
    let mut stop_hit = false;

    for i in 0..400_u64 {
        let mut p = Map::new();
        p.insert("blob".into(), Value::String(blob.clone()));
        let req =
            build_spike_noop_cmd(&ids, authority, Revision::new(i), "WalPressure", p, vec![])?;
        match store.commit_command(req) {
            Ok(_) => committed += 1,
            Err(StoreError::WalPressureStop { wal_bytes, .. }) => {
                stop_hit = true;
                stop_wal_bytes = wal_bytes;
                break;
            }
            Err(other) => return Err(Box::new(other)),
        }
    }

    let before_reap = store.wal_telemetry();
    clock.advance(Duration::from_secs(6));
    let after_reap = store.run_wal_maintenance_tick()?;

    let export_res = store.export_bounded(BoundedExportRequest {
        destination_path: wal_dir.path().join("spike-export.ndjson"),
        staging_dir: wal_dir.path().join("spike-staging"),
        max_bytes_override: None,
        chunk_bytes_override: None,
        cancellation: ExportCancellation::new(),
    });
    let export_emitted = match export_res {
        Err(StoreError::ExportQuotaExceeded {
            bytes_written,
            max_bytes,
        }) => (bytes_written, max_bytes),
        other => {
            return Err(format!("export attendu en dépassement quota : {other:?}").into());
        }
    };

    writeln!(out, "=== WAL Pressure & Bounded Export ===")?;
    writeln!(
        out,
        "wal_pressure_threshold={} wal_stop_threshold={} commits_before_stop={} wal_stop_hit={} stop_wal_bytes={} passive_ckpts_before_reap={}",
        DEFAULT_WAL_PRESSURE_BYTES,
        DEFAULT_WAL_STOP_BYTES,
        committed,
        stop_hit,
        stop_wal_bytes,
        before_reap.passive_checkpoints,
    )?;
    writeln!(
        out,
        "after_reap reader_aborted={} reader_released={} expired_terminated={} passive_ckpts={} active_wal_after_reap={}",
        held.is_aborted(),
        held.is_released(),
        after_reap.expired_readers_terminated,
        after_reap.passive_checkpoints,
        after_reap.wal_bytes,
    )?;
    writeln!(
        out,
        "export_quota_exceeded bytes_written={} max_bytes={} chunk_limit={}",
        export_emitted.0, export_emitted.1, DEFAULT_EXPORT_CHUNK_BYTES,
    )?;
    writeln!(
        out,
        "export_max_default={DEFAULT_EXPORT_MAX_BYTES} export_chunk_default={DEFAULT_EXPORT_CHUNK_BYTES}"
    )?;

    store.shutdown()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out = io::stdout().lock();
    run_latency_benchmark(&mut out)?;
    run_wal_and_export_spike(&mut out)?;
    Ok(())
}
