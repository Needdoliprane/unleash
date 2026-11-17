//! Façade principale `UnleashStore` coordonnant writer unique, lecteurs, objets, sauvegardes et restauration.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use unleash_domain::{
    AccountId, ActionId, ActorId, AuthorityGeneration, BackupId, CommandId, DaemonEpoch,
    DaemonInstanceId, LeaseId, MicrosAmount, PoolId, ReceiptId, Sequence, Sha256Digest, StreamId,
    TimestampMs,
};
use unleash_protocol::{CommandEnvelope, EventEnvelope};
use uuid::Uuid;

use crate::backup::{
    BackupManifest, BoundedExportOutcome, BoundedExportRequest, PrepareBackupRequest,
    ValidateBackupReport, prepare_backup_workflow, run_bounded_export, tx_abandon_backup_job,
    validate_backup_dir,
};
use crate::budgets::{
    BudgetAccountSnapshot, BudgetScopeType, CapacityPoolState, ExecutionPoolParams,
    tx_configure_execution_pool, tx_mark_action_budget_unknown, tx_read_budget_account,
    tx_read_execution_pool, tx_release_action_budget, tx_release_capacity_reservation,
    tx_settle_action_budget, tx_upsert_budget_account,
};
use crate::commands::{
    CommitCommandRequest, CommitOutcome, DaemonStateRow, ReceiptInspectionOutcome,
    ReceiptPurgeReport, compute_validated_command_params, decode_command_envelope_json,
    read_command_receipt_inspection, read_daemon_state_conn, read_daemon_state_tx,
    tx_commit_command, tx_purge_expired_command_receipts,
};
use crate::config::{
    FaultTarget, StoreConfig, StoreCutpoint, WriteCriticality, check_injected_fault,
};
use crate::error::StoreError;
use crate::events::{
    DeadLetterItem, InboxApplyOutcome, OutboxDispatchItem, PurgeStats, ReadEventsOutcome,
    read_dead_letters_conn, read_events_with_window, tx_ack_outbox, tx_apply_inbox_event,
    tx_lease_outbox_batch, tx_purge_expired_inbox, tx_purge_history,
    tx_record_non_transient_failure, tx_record_transient_failure,
};
use crate::fs_sync::{set_dir_permissions_0700, set_path_permissions};
use crate::leases::{
    AcquireLeaseRequest, FenceAssertion, LeaseLineageCtx, ResourceLeaseRecord, tx_acquire_lease,
    tx_release_lease, tx_revoke_and_reassign_lease, tx_validate_lease_fence,
};
use crate::migrations::{apply_pending_migrations, preflight_check_existing_db};
use crate::objects::{
    DeletionIntent, ObjectReconciliationReport, ObjectRecordView, PrepareObjectDeletionRequest,
    PrepareObjectRequest, PreparedObjectHandle, PublishObjectReferenceRequest,
    execute_unlink_and_finalize_deletion, read_object_record_conn, read_verified_object_bytes_conn,
    reconcile_objects_workflow, tx_acquire_read_pin, tx_prepare_object, tx_prepare_object_deletion,
    tx_publish_object_reference, tx_release_pin, tx_tombstone_object_references,
    verify_object_file_before_publish, write_prepared_object_bytes_to_disk,
};
use crate::qualification::{
    SqliteRuntimeManifest, apply_and_verify_writer_pragmas, inspect_and_verify_build,
};
use crate::restore::{
    MaintenanceRestoreManifest, RestoreActivationResult, RestoreBackupRequest,
    execute_restore_backup, read_maintenance_manifest,
};
use crate::writer::{
    HeldReaderSession, ReaderContext, ReaderRegistry, StoreWriterHandle, WalTelemetry,
    open_reader_connection, run_guarded_immediate_tx, spawn_held_reader_session,
};

struct SnapshotReaderGuard<'a> {
    conn: Option<Connection>,
    readers: &'a ReaderRegistry,
    reader_id: u64,
}

impl Drop for SnapshotReaderGuard<'_> {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            let _ = c.execute_batch("ROLLBACK;");
            drop(c);
        }
        self.readers.unregister(self.reader_id);
    }
}

/// Façade propriétaire de la persistance locale `SQLite` d'Unleash.
///
/// Possède l'unique thread writer derrière une file bornée à 8 demandes, coordonne les lecteurs
/// `query_only=ON`, les seuils WAL, les migrations, les objets adressés par contenu, les sauvegardes
/// et les restaurations offline.
pub struct UnleashStore {
    config: StoreConfig,
    manifest: SqliteRuntimeManifest,
    writer: StoreWriterHandle,
    readers: Arc<ReaderRegistry>,
}

impl std::fmt::Debug for UnleashStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnleashStore")
            .field("db_path", &self.config.db_path)
            .field("sqlite_version", &self.manifest.sqlite_version)
            .finish_non_exhaustive()
    }
}

fn enforce_db_sidecar_permissions(db_path: &Path) {
    if db_path.exists() {
        let _ = set_path_permissions(db_path, 0o600);
    }
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", db_path.display()));
        if sidecar.exists() {
            let _ = set_path_permissions(&sidecar, 0o600);
        }
    }
}

fn apply_startup_state(
    conn: &mut Connection,
    config: &StoreConfig,
    manifest: &SqliteRuntimeManifest,
    schema_version: i64,
    now_ms: TimestampMs,
) -> Result<(AuthorityGeneration, DaemonEpoch), StoreError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StoreError::from_rusqlite)?;

    let existing_count: i64 = tx
        .query_row("SELECT COUNT(*) FROM daemon_state", [], |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;

    let (authority_generation, daemon_epoch) = if existing_count == 0 {
        let auth = AuthorityGeneration::from_uuid(config.id_generator.new_uuid());
        let stream_id = StreamId::from_uuid(config.id_generator.new_uuid());
        let inst_id = DaemonInstanceId::from_uuid(config.id_generator.new_uuid());
        let boot_id = config.id_generator.new_uuid().to_string();
        let uid = i64::from(config.owner_uid);
        let epoch = DaemonEpoch::new(1);

        tx.execute(
            "INSERT INTO daemon_state
             (singleton, owner_uid, authority_generation, daemon_epoch, daemon_instance_id,
              boot_identity, stream_id, schema_version, mode, revision, started_at_ms)
             VALUES (1, ?1, ?2, 1, ?3, ?4, ?5, ?6, 'ready', 0, ?7)",
            rusqlite::params![
                uid,
                auth.to_string(),
                inst_id.to_string(),
                boot_id,
                stream_id.to_string(),
                schema_version,
                now_ms.as_i64()
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
        (auth, epoch)
    } else {
        tx.execute(
            "UPDATE daemon_state SET schema_version = ?1 WHERE singleton = 1",
            [schema_version],
        )
        .map_err(StoreError::from_rusqlite)?;

        // Libérer les leases outbox expirées lors d'un arrêt/redémarrage
        tx.execute(
            "UPDATE outbox SET state = 'pending', lease_epoch = NULL WHERE state = 'leased'",
            [],
        )
        .map_err(StoreError::from_rusqlite)?;

        // Réarmement unique dead_letter -> pending uniquement si consumer_version a changé
        tx.execute(
            "UPDATE outbox
             SET state = 'pending', rearm_count = 1, delivery_attempt = 0, consumer_version = ?1
             WHERE state = 'dead_letter' AND rearm_count = 0 AND consumer_version != ?1",
            [config.consumer_version.as_str()],
        )
        .map_err(StoreError::from_rusqlite)?;

        let row = read_daemon_state_tx(&tx)?;
        (row.authority_generation, row.daemon_epoch)
    };

    let opts_json = serde_json::to_string(&manifest.compile_options)?;
    let pragmas_json = serde_json::to_string(&manifest.effective_pragmas)?;
    let epoch_i64 = i64::try_from(daemon_epoch.as_u64()).unwrap_or(1);

    tx.execute(
        "INSERT INTO store_runtime_manifests
         (sqlite_version, sqlite_source_id, expected_source_id, compile_options_json,
          effective_pragmas_json, authority_generation, daemon_epoch, qualified_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            manifest.sqlite_version,
            manifest.sqlite_source_id,
            manifest.expected_source_id,
            opts_json,
            pragmas_json,
            authority_generation.to_string(),
            epoch_i64,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    tx.commit().map_err(StoreError::from_rusqlite)?;
    Ok((authority_generation, daemon_epoch))
}

impl UnleashStore {
    /// Ouvre ou initialise la base `SQLite` qualifiée avec vérification de build, PRAGMAs et migrations.
    ///
    /// # Errors
    /// Retourne [`StoreError`] si la qualification échoue, la version de schéma est supérieure,
    /// ou une migration est interrompue/corrompue.
    pub fn open(config: StoreConfig) -> Result<Self, StoreError> {
        Self::open_with_migration_backup(config, None)
    }

    /// Ouvre en fournissant une sauvegarde préalable vérifiée si une montée de version est nécessaire.
    ///
    /// # Errors
    /// Retourne [`StoreError`] si l'ouverture ou la migration échoue.
    pub fn open_with_migration_backup(
        config: StoreConfig,
        pre_migration_backup_id: Option<BackupId>,
    ) -> Result<Self, StoreError> {
        fs::create_dir_all(&config.data_root)?;
        set_dir_permissions_0700(&config.data_root)?;
        if let Some(db_parent) = config.db_path.parent() {
            fs::create_dir_all(db_parent)?;
            set_dir_permissions_0700(db_parent)?;
        }
        fs::create_dir_all(&config.artifacts_dir)?;
        set_dir_permissions_0700(&config.artifacts_dir)?;

        // Pré-contrôle strictement en lecture seule sur base existante (zéro écriture si refus)
        if config.db_path.exists() {
            let ro_conn = Connection::open_with_flags(
                &config.db_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(StoreError::from_rusqlite)?;
            ro_conn
                .busy_timeout(std::time::Duration::from_millis(u64::from(
                    config.busy_timeout_ms,
                )))
                .map_err(StoreError::from_rusqlite)?;
            let _ = inspect_and_verify_build(&ro_conn, &config.build_policy)?;
            let _ = preflight_check_existing_db(
                &ro_conn,
                config.max_supported_schema_version,
                pre_migration_backup_id,
            )?;
            drop(ro_conn);
        }

        let mut rw_conn = Connection::open_with_flags(
            &config.db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(StoreError::from_rusqlite)?;

        let (sqlite_version, sqlite_source_id, compile_options) =
            inspect_and_verify_build(&rw_conn, &config.build_policy)?;
        let effective_pragmas = apply_and_verify_writer_pragmas(&rw_conn, config.busy_timeout_ms)?;

        let now_ms = config.clock.now_ms();
        let mig_report = apply_pending_migrations(
            &mut rw_conn,
            config.max_supported_schema_version,
            pre_migration_backup_id,
            now_ms,
            config.cutpoints.as_ref(),
        )?;

        let manifest = SqliteRuntimeManifest {
            sqlite_version,
            sqlite_source_id,
            expected_source_id: config.build_policy.expected_source_id.clone(),
            compile_options,
            effective_pragmas,
            qualified_at_ms: now_ms,
        };

        let (startup_auth, startup_epoch) = apply_startup_state(
            &mut rw_conn,
            &config,
            &manifest,
            mig_report.current_version,
            now_ms,
        )?;
        enforce_db_sidecar_permissions(&config.db_path);

        let readers = ReaderRegistry::new();
        let writer = StoreWriterHandle::start(config.clone(), rw_conn, Arc::clone(&readers))?;

        tracing::info!(
            sqlite_version = %manifest.sqlite_version,
            schema_version = mig_report.current_version,
            authority_generation = %startup_auth,
            daemon_epoch = startup_epoch.as_u64(),
            owner_uid = config.owner_uid,
            "UnleashStore ouvert et qualifié"
        );

        Ok(Self {
            config,
            manifest,
            writer,
            readers,
        })
    }

    /// Renvoie le manifeste runtime `SQLite` vérifié au démarrage.
    #[must_use]
    pub const fn runtime_manifest(&self) -> &SqliteRuntimeManifest {
        &self.manifest
    }

    /// Renvoie la configuration active.
    #[must_use]
    pub const fn config(&self) -> &StoreConfig {
        &self.config
    }

    /// Inspecte l'état courant de `daemon_state` via une connexion lectrice `query_only=ON`.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture ou de dépassement du délai de 5 s.
    pub fn daemon_state(&self) -> Result<DaemonStateRow, StoreError> {
        self.read_snapshot(|conn, _ctx| read_daemon_state_conn(conn))
    }

    /// Déclare l'état `read_only_recovery` (diagnostic) après détection d'une corruption.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de persistance.
    pub fn enter_read_only_recovery(&self) -> Result<DaemonStateRow, StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, |tx, _now| {
                tx.execute(
                    "UPDATE daemon_state SET mode = 'read_only_recovery', revision = revision + 1
                     WHERE singleton = 1",
                    [],
                )
                .map_err(StoreError::from_rusqlite)?;
                read_daemon_state_tx(tx)
            })
    }

    /// Simule/exécute un redémarrage ordinaire dans la même lignée (`daemon_epoch += 1`,
    /// réinitialisation des leases outbox d'époques antérieures, passage des `resource_leases` en `suspect`,
    /// réarmement unique éventuel des `dead_letter` si `consumer_version` a changé).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'échec de réouverture ou de qualification.
    pub fn restart_same_lineage(mut self) -> Result<Self, StoreError> {
        let cfg = self.config.clone();
        self.writer.shutdown();

        let reopened = Self::open(cfg)?;
        let new_inst = DaemonInstanceId::from_uuid(reopened.config.id_generator.new_uuid());
        let new_boot = reopened.config.id_generator.new_uuid().to_string();

        reopened.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| {
                let d = read_daemon_state_tx(tx)?;
                let next_epoch = i64::try_from(d.daemon_epoch.as_u64())
                    .unwrap_or(1)
                    .saturating_add(1);
                tx.execute(
                    "UPDATE daemon_state
                     SET daemon_epoch = ?1, daemon_instance_id = ?2, boot_identity = ?3, started_at_ms = ?4
                     WHERE singleton = 1",
                    rusqlite::params![
                        next_epoch,
                        new_inst.to_string(),
                        new_boot,
                        now_ms.as_i64()
                    ],
                )
                .map_err(StoreError::from_rusqlite)?;

                tx.execute(
                    "UPDATE resource_leases SET state = 'suspect'
                     WHERE authority_generation = ?1 AND state = 'held'",
                    [d.authority_generation.to_string()],
                )
                .map_err(StoreError::from_rusqlite)?;
                tracing::info!(
                    authority_generation = %d.authority_generation,
                    daemon_epoch = next_epoch,
                    "redémarrage même lignée : époque incrémentée et leases classées suspect"
                );
                Ok(())
            },
        )?;

        let _ = reopened.reconcile_objects_and_deletions()?;
        Ok(reopened)
    }

    /// Valide et décode une trame JSON brute de commande selon `unleash.ipc.v2`.
    ///
    /// # Errors
    /// Retourne [`StoreError::ProtocolIncompatible`] pour `unleash.ipc.v1` et [`StoreError::Protocol`]
    /// pour toute clé dupliquée ou champ inconnu/invalide.
    pub fn decode_command_envelope_json(raw_json: &str) -> Result<CommandEnvelope, StoreError> {
        decode_command_envelope_json(raw_json)
    }

    fn run_command_prechecks(
        &self,
        req: &CommitCommandRequest,
    ) -> Result<(Sha256Digest, u32), StoreError> {
        if let Some(available_bytes) = self
            .config
            .disk_space_probe
            .available_bytes(&self.config.data_root)
            && available_bytes < self.config.min_free_space_bytes
        {
            tracing::warn!(
                available_bytes,
                required_bytes = self.config.min_free_space_bytes,
                "espace disque critique insuffisant avant soumission de commande"
            );
            return Err(StoreError::StorageLowSpace {
                available_bytes,
                required_bytes: self.config.min_free_space_bytes,
            });
        }

        check_injected_fault(
            self.config.fault_injector.as_ref(),
            FaultTarget::CommandWrite,
        )?;

        let validated = compute_validated_command_params(&self.config, req)?;
        self.config
            .cutpoints
            .hit(StoreCutpoint::C0BeforeCommandTx)?;
        Ok(validated)
    }

    /// Exécute l'algorithme transactionnel canonique `CommitCommand` sur le thread writer unique.
    ///
    /// # Errors
    /// Retourne [`StoreError`] (`StaleAuthorityGeneration`, `IdempotencyConflict`,
    /// `IdempotencyWindowExpired`, `RevisionConflict`, `Backpressure`, `StorageBusy`, etc.).
    pub fn commit_command(&self, req: CommitCommandRequest) -> Result<CommitOutcome, StoreError> {
        let _span = tracing::info_span!(
            "commit_command",
            command_id = %req.envelope.command_id,
            authority_generation = %req.envelope.authority_generation,
            command_type = %req.envelope.command_type,
            aggregate_id = %req.envelope.aggregate_id
        )
        .entered();
        let (digest, retention_days) = self.run_command_prechecks(&req)?;
        let cfg = self.config.clone();
        let cutpoints = Arc::clone(&self.config.cutpoints);
        let outcome = self.writer.submit_immediate_blocking(
            WriteCriticality::Normal,
            true,
            move |tx, now_ms| {
                tx_commit_command(
                    tx,
                    &cfg,
                    now_ms,
                    &req,
                    digest,
                    retention_days,
                    cutpoints.as_ref(),
                )
            },
        )?;
        self.config
            .cutpoints
            .hit(StoreCutpoint::C2AfterCommandCommit)?;
        Ok(outcome)
    }

    /// Variante asynchrone de [`Self::commit_command`] attendant sur `tokio::sync::oneshot`.
    ///
    /// # Errors
    /// Identique à [`Self::commit_command`].
    pub async fn commit_command_async(
        &self,
        req: CommitCommandRequest,
    ) -> Result<CommitOutcome, StoreError> {
        let _span = tracing::info_span!(
            "commit_command_async",
            command_id = %req.envelope.command_id,
            authority_generation = %req.envelope.authority_generation,
            command_type = %req.envelope.command_type,
            aggregate_id = %req.envelope.aggregate_id
        )
        .entered();
        let (digest, retention_days) = self.run_command_prechecks(&req)?;
        let cfg = self.config.clone();
        let cutpoints = Arc::clone(&self.config.cutpoints);
        let outcome = self
            .writer
            .submit_immediate_async(WriteCriticality::Normal, true, move |tx, now_ms| {
                tx_commit_command(
                    tx,
                    &cfg,
                    now_ms,
                    &req,
                    digest,
                    retention_days,
                    cutpoints.as_ref(),
                )
            })
            .await?;
        self.config
            .cutpoints
            .hit(StoreCutpoint::C2AfterCommandCommit)?;
        Ok(outcome)
    }

    /// Inspecte un reçu par `(actor_id, command_id)` (`GetCommandReceipt`).
    ///
    /// # Errors
    /// Retourne [`StoreError::NotFound`] si la clé est absente ou une erreur de lecture.
    pub fn inspect_command_receipt(
        &self,
        actor_id: &ActorId,
        command_id: CommandId,
    ) -> Result<ReceiptInspectionOutcome, StoreError> {
        self.read_snapshot(|conn, _ctx| read_command_receipt_inspection(conn, actor_id, command_id))
    }

    /// Purge les réponses complètes expirées vers le statut `tombstone` (suspendu si horloge incertaine).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de stockage.
    pub fn purge_expired_command_receipts(&self) -> Result<ReceiptPurgeReport, StoreError> {
        check_injected_fault(
            self.config.fault_injector.as_ref(),
            FaultTarget::Maintenance,
        )?;
        let clock_uncertain = self.config.clock_uncertain.load(Ordering::Acquire);
        self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| tx_purge_expired_command_receipts(tx, now_ms, clock_uncertain),
        )
    }

    /// Lit un snapshot cohérent sur une connexion lectrice `query_only=ON` bornée à `read_timeout` (5 s).
    ///
    /// # Errors
    /// Retourne [`StoreError::ReadTimeout`] si la lecture dépasse 5 s ou est interrompue.
    pub fn read_snapshot<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&Connection, &ReaderContext) -> Result<T, StoreError>,
    {
        let conn = open_reader_connection(&self.config)?;
        let started_at = self.config.monotonic_clock.monotonic_now();
        let control = self.readers.register(
            started_at,
            self.config.read_timeout,
            conn.get_interrupt_handle(),
            None,
        );
        let reader_id = control.reader_id();
        conn.execute_batch("BEGIN DEFERRED;")
            .map_err(StoreError::from_rusqlite)?;

        let guard = SnapshotReaderGuard {
            conn: Some(conn),
            readers: self.readers.as_ref(),
            reader_id,
        };
        let Some(conn_ref) = guard.conn.as_ref() else {
            return Err(StoreError::StorageUnavailable(
                "connexion lectrice indisponible".to_owned(),
            ));
        };

        let ctx = ReaderContext {
            control: Arc::clone(&control),
        };
        let result = f(conn_ref, &ctx);
        let elapsed = self
            .config
            .monotonic_clock
            .monotonic_now()
            .saturating_sub(started_at);
        let aborted = control.is_aborted();
        drop(guard);

        if aborted || elapsed > self.config.read_timeout {
            return Err(StoreError::ReadTimeout {
                elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                max_ms: u64::try_from(self.config.read_timeout.as_millis()).unwrap_or(5_000),
            });
        }
        result
    }

    /// Lit les événements du journal à partir de `after_sequence` (`SubscribeEvents` / `ReadEvents`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] si la requête `SQLite` échoue.
    pub fn read_events(
        &self,
        stream_id: StreamId,
        after_sequence: Sequence,
        limit: u32,
        cursor_recorded_at_ms: Option<TimestampMs>,
    ) -> Result<ReadEventsOutcome, StoreError> {
        let now_ms = self.config.clock.now_ms();
        self.read_snapshot(|conn, _ctx| {
            read_events_with_window(
                conn,
                now_ms,
                stream_id,
                after_sequence,
                limit,
                cursor_recorded_at_ms,
            )
        })
    }

    /// Loue le prochain lot ordonné d'entrées `outbox` admissibles (1 en vol par `(destination, aggregate_id)`,
    /// parallélisme global <= 32, exclusion des scopes gelés après restore).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur transactionnelle.
    pub fn lease_outbox_batch(
        &self,
        destination: &str,
        limit: usize,
    ) -> Result<Vec<OutboxDispatchItem>, StoreError> {
        let dest = destination.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                let daemon = read_daemon_state_tx(tx)?;
                tx_lease_outbox_batch(
                    tx,
                    daemon.authority_generation,
                    daemon.daemon_epoch,
                    now_ms,
                    &dest,
                    limit,
                )
            })
    }

    /// Applique un événement dans `inbox` et exécute la mutation métier dans la même transaction (`ApplyInboxEvent`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de transaction.
    pub fn apply_inbox_event<F>(
        &self,
        consumer_id: &str,
        event: &EventEnvelope,
        result_hash: Sha256Digest,
        mutation_fn: F,
    ) -> Result<InboxApplyOutcome, StoreError>
    where
        F: FnOnce(&Transaction<'_>) -> Result<(), StoreError> + Send + 'static,
    {
        let c_id = consumer_id.to_owned();
        let ev_id = event.event_id;
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                tx_apply_inbox_event(tx, now_ms, &c_id, ev_id, result_hash, mutation_fn)
            })
    }

    /// Acquitte une entrée `outbox` après traitement confirmé.
    ///
    /// # Errors
    /// Retourne [`StoreError`] si l'entrée n'existe pas ou en cas d'erreur de stockage.
    pub fn ack_outbox_delivery(&self, event_id: Uuid, destination: &str) -> Result<(), StoreError> {
        let dest = destination.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                tx_ack_outbox(tx, now_ms, event_id, &dest)
            })
    }

    /// Enregistre un échec transitoire outbox avec backoff exponentiel `200ms * 2^k` plafonné à 60 s (±20 % gigue).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn record_outbox_transient_failure(
        &self,
        event_id: Uuid,
        destination: &str,
        error_message: &str,
        jitter_bps: i16,
    ) -> Result<TimestampMs, StoreError> {
        let dest = destination.to_owned();
        let err_msg = error_message.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                tx_record_transient_failure(tx, now_ms, event_id, &dest, &err_msg, jitter_bps)
            })
    }

    /// Enregistre un échec non transitoire ; passe en `dead_letter` au 8e échec et bloque l'agrégat si conditionnant.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn record_outbox_non_transient_failure(
        &self,
        event_id: Uuid,
        destination: &str,
        error_message: &str,
        aggregate_blocking_target: Option<(&str, Uuid)>,
    ) -> Result<bool, StoreError> {
        let cfg = self.config.clone();
        let dest = destination.to_owned();
        let err_msg = error_message.to_owned();
        let target_owned = aggregate_blocking_target.map(|(k, id)| (k.to_owned(), id));
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, _now| {
                let target_ref = target_owned.as_ref().map(|(k, id)| (k.as_str(), *id));
                tx_record_non_transient_failure(tx, &cfg, event_id, &dest, &err_msg, target_ref)
            })
    }

    /// Liste les entrées actuellement en `dead_letter` (pour `GetHealth` et `GetDaemonStatus`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn list_dead_letters(&self) -> Result<Vec<DeadLetterItem>, StoreError> {
        self.read_snapshot(|conn, _ctx| read_dead_letters_conn(conn))
    }

    /// Purge les entrées `inbox` dont l'outbox est `acked` depuis au moins 7 jours.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn purge_expired_inbox(&self) -> Result<PurgeStats, StoreError> {
        check_injected_fault(
            self.config.fault_injector.as_ref(),
            FaultTarget::Maintenance,
        )?;
        let clock_uncertain = self.config.clock_uncertain.load(Ordering::Acquire);
        let affected_rows = self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| tx_purge_expired_inbox(tx, now_ms, clock_uncertain),
        )?;
        Ok(PurgeStats {
            affected_rows,
            wal_truncated: false,
            completed_at_ms: self.config.clock.now_ms(),
        })
    }

    /// Purge explicite `PurgeHistory` : remplace le payload d'événements éligibles (>= 7 j, outbox acked) par
    /// un tombstone sans jamais supprimer physiquement la ligne `events`, puis exécute `wal_checkpoint(TRUNCATE)`.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn purge_history(&self, older_than_ms: TimestampMs) -> Result<PurgeStats, StoreError> {
        check_injected_fault(
            self.config.fault_injector.as_ref(),
            FaultTarget::Maintenance,
        )?;
        let clock_uncertain = self.config.clock_uncertain.load(Ordering::Acquire);
        let affected_rows = self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| tx_purge_history(tx, now_ms, older_than_ms, clock_uncertain),
        )?;
        let _ = self.writer.maintenance_tick(true)?;
        tracing::info!(
            affected_rows,
            older_than_ms = older_than_ms.as_i64(),
            "purge historique effectuée avec WAL TRUNCATE"
        );
        Ok(PurgeStats {
            affected_rows,
            wal_truncated: true,
            completed_at_ms: self.config.clock.now_ms(),
        })
    }

    /// Configure ou initialise le pool d'exécution global (1..=50, soft/hard, drainage si baisse sous occupation).
    ///
    /// # Errors
    /// Retourne [`StoreError::ControlReserveIncoherent`] si les paramètres de réserve sont incohérents.
    pub fn configure_execution_pool(
        &self,
        params: ExecutionPoolParams,
    ) -> Result<CapacityPoolState, StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _now| {
                tx_configure_execution_pool(tx, &params)
            })
    }

    /// Lit l'état courant du pool de capacité.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn execution_pool_state(&self, pool_id: PoolId) -> Result<CapacityPoolState, StoreError> {
        self.read_snapshot(|conn, _ctx| tx_read_execution_pool(conn, pool_id))
    }

    /// Libère une réservation de capacité et lève le gel de drainage si `occupation <= requested_limit`.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn release_capacity_reservation(
        &self,
        reservation_id: unleash_domain::ReservationId,
    ) -> Result<(), StoreError> {
        self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| tx_release_capacity_reservation(tx, now_ms, reservation_id),
        )
    }

    /// Crée ou met à jour un compte budgétaire (`global`, `project`, `provider`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn upsert_budget_account(
        &self,
        account_id: AccountId,
        scope_type: BudgetScopeType,
        scope_id: &str,
        currency: &str,
        limit_micros: MicrosAmount,
    ) -> Result<BudgetAccountSnapshot, StoreError> {
        let s_id = scope_id.to_owned();
        let curr = currency.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, _now| {
                tx_upsert_budget_account(tx, account_id, scope_type, &s_id, &curr, limit_micros)
            })
    }

    /// Lit l'état d'un compte budgétaire.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn budget_account(
        &self,
        account_id: AccountId,
    ) -> Result<BudgetAccountSnapshot, StoreError> {
        self.read_snapshot(|conn, _ctx| tx_read_budget_account(conn, account_id))
    }

    /// Marque les réservations budgétaires d'une action en `unknown` (aucune libération sur timeout).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn mark_action_budget_unknown(&self, action_id: ActionId) -> Result<(), StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _now| {
                tx_mark_action_budget_unknown(tx, action_id)
            })
    }

    /// Règle le coût réel d'une action (même s'il dépasse l'estimation ou le plafond ; bloque les futures admissions).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn settle_action_budget(
        &self,
        action_id: ActionId,
        actual_micros: MicrosAmount,
    ) -> Result<(), StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _now| {
                tx_settle_action_budget(tx, action_id, actual_micros)
            })
    }

    /// Libère une réservation budgétaire après échec confirmé sans effet.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn release_action_budget(&self, action_id: ActionId) -> Result<(), StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _now| {
                tx_release_action_budget(tx, action_id)
            })
    }

    /// Acquiert une lease exclusive en incrémentant durablement `resource_fences.last_fence` (`AcquireLease`).
    ///
    /// # Errors
    /// Retourne [`StoreError::CapacityBlocked`] ou [`StoreError`] si la ressource est déjà détenue.
    pub fn acquire_lease(
        &self,
        req: AcquireLeaseRequest,
    ) -> Result<ResourceLeaseRecord, StoreError> {
        let lease_id = LeaseId::from_uuid(self.config.id_generator.new_uuid());
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                let d = read_daemon_state_tx(tx)?;
                let ctx = LeaseLineageCtx {
                    authority: d.authority_generation,
                    daemon_epoch: d.daemon_epoch,
                    boot_identity: &d.boot_identity,
                    now_ms,
                };
                tx_acquire_lease(tx, &ctx, lease_id, &req)
            })
    }

    /// Révoque une lease active et réattribue immédiatement à un nouveau propriétaire avec un fence supérieur.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn revoke_and_reassign_lease(
        &self,
        old_lease_id: LeaseId,
        new_req: AcquireLeaseRequest,
    ) -> Result<ResourceLeaseRecord, StoreError> {
        let new_lease_id = LeaseId::from_uuid(self.config.id_generator.new_uuid());
        self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| {
                let d = read_daemon_state_tx(tx)?;
                let ctx = LeaseLineageCtx {
                    authority: d.authority_generation,
                    daemon_epoch: d.daemon_epoch,
                    boot_identity: &d.boot_identity,
                    now_ms,
                };
                tx_revoke_and_reassign_lease(tx, &ctx, old_lease_id, new_lease_id, &new_req)
            },
        )
    }

    /// Libère une lease sans jamais décrémenter `last_fence`.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn release_lease(&self, lease_id: LeaseId) -> Result<(), StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _now| {
                tx_release_lease(tx, lease_id)
            })
    }

    /// Valide qu'une assertion de fence (`authority_generation`, `daemon_epoch`, `fence`, `owner_id`) est active.
    ///
    /// # Errors
    /// Retourne [`StoreError::StaleAuthorityGeneration`] ou [`StoreError::StaleFence`] si périmée.
    pub fn validate_lease_fence(&self, assertion: &FenceAssertion) -> Result<(), StoreError> {
        let assertion_c = assertion.clone();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, _now| {
                let d = read_daemon_state_tx(tx)?;
                tx_validate_lease_fence(tx, d.authority_generation, d.daemon_epoch, &assertion_c)
            })
    }

    /// Prépare un objet sous pin durable avant toute écriture disque (`PrepareObject`).
    ///
    /// # Errors
    /// Retourne [`StoreError::ObjectDeleting`] si l'objet est en cours de suppression.
    pub fn prepare_object(
        &self,
        req: PrepareObjectRequest,
    ) -> Result<PreparedObjectHandle, StoreError> {
        let cfg = self.config.clone();
        let handle = self.writer.submit_immediate_blocking(
            WriteCriticality::Normal,
            false,
            move |tx, now_ms| {
                let d = read_daemon_state_tx(tx)?;
                tx_prepare_object(tx, &cfg, d.authority_generation, now_ms, &req)
            },
        )?;
        self.config.cutpoints.hit(StoreCutpoint::C10aAfterPrepPin)?;
        Ok(handle)
    }

    /// Écrit physiquement les octets hors transaction, vérifie SHA-256, synchronise et renomme atomiquement.
    ///
    /// # Errors
    /// Retourne [`StoreError`] si le digest ou la synchronisation échoue.
    pub fn write_prepared_object_bytes(
        &self,
        handle: &PreparedObjectHandle,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        write_prepared_object_bytes_to_disk(&self.config, handle, bytes)
    }

    /// Passe l'objet à `live`, publie atomiquement `object_references.live` et libère le pin (`PublishObjectReference`).
    ///
    /// # Errors
    /// Retourne [`StoreError::ObjectDeleting`] ou [`StoreError::InvariantViolation`] si l'état/génération diverge.
    pub fn publish_object_reference(
        &self,
        req: PublishObjectReferenceRequest,
    ) -> Result<(), StoreError> {
        let rec = self.object_record(req.object_hash)?;
        verify_object_file_before_publish(&self.config, &rec.relative_path, req.object_hash)?;
        let id_gen = Arc::clone(&self.config.id_generator);
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                tx_publish_object_reference(tx, id_gen.as_ref(), now_ms, &req)
            })
    }

    /// Acquiert un pin de lecture sur un objet `live`.
    ///
    /// # Errors
    /// Retourne [`StoreError::ObjectDeleting`] ou [`StoreError::NotFound`] si indisponible.
    pub fn acquire_read_pin(
        &self,
        object_hash: Sha256Digest,
        owner_kind: &str,
        owner_id: &str,
    ) -> Result<String, StoreError> {
        let id_gen = Arc::clone(&self.config.id_generator);
        let okind = owner_kind.to_owned();
        let oid = owner_id.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                let d = read_daemon_state_tx(tx)?;
                tx_acquire_read_pin(
                    tx,
                    id_gen.as_ref(),
                    d.authority_generation,
                    now_ms,
                    object_hash,
                    &okind,
                    &oid,
                )
            })
    }

    /// Libère un pin de lecture ou de préparation.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn release_pin(&self, pin_id: &str) -> Result<(), StoreError> {
        let pid = pin_id.to_owned();
        self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| tx_release_pin(tx, now_ms, &pid),
        )
    }

    /// Passe les références d'un propriétaire en `tombstone` (lors d'une purge autorisée).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn tombstone_object_references(
        &self,
        object_hash: Sha256Digest,
        reason: &str,
    ) -> Result<usize, StoreError> {
        let rsn = reason.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, _now| {
                tx_tombstone_object_references(tx, object_hash, &rsn)
            })
    }

    /// Réserve exclusivement une suppression d'objet (`PrepareObjectDeletion`) sans unlink dans la transaction.
    ///
    /// # Errors
    /// Retourne [`StoreError::ObjectReferenced`], [`StoreError::ObjectPinned`] ou [`StoreError::BackupBarrierActive`].
    pub fn prepare_object_deletion(
        &self,
        req: PrepareObjectDeletionRequest,
    ) -> Result<DeletionIntent, StoreError> {
        self.config
            .cutpoints
            .hit(StoreCutpoint::C10bBeforeDeletingReserve)?;
        let id_gen = Arc::clone(&self.config.id_generator);
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                tx_prepare_object_deletion(tx, id_gen.as_ref(), now_ms, &req)
            })
    }

    /// Exécute `unlink` + `sync_dir` hors transaction, puis finalise `tombstone`/`completed` (`FinalizeObjectDeletion`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] si la génération ou le chemin a changé ou en cas d'anomalie disque.
    pub fn execute_and_finalize_object_deletion(
        &self,
        intent: &DeletionIntent,
    ) -> Result<(), StoreError> {
        execute_unlink_and_finalize_deletion(
            &self.config,
            &self.writer,
            intent,
            self.config.cutpoints.as_ref(),
            true,
        )
    }

    /// Réconcilie au redémarrage pins suspects, objets préparés et suppressions interrompues (`C10a..d`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL ou I/O.
    pub fn reconcile_objects_and_deletions(
        &self,
    ) -> Result<ObjectReconciliationReport, StoreError> {
        reconcile_objects_workflow(&self.config, &self.writer, self.config.cutpoints.as_ref())
    }

    /// Lit l'enregistrement `object_records` d'un hash.
    ///
    /// # Errors
    /// Retourne [`StoreError::NotFound`] si absent.
    pub fn object_record(&self, object_hash: Sha256Digest) -> Result<ObjectRecordView, StoreError> {
        self.read_snapshot(|conn, _ctx| read_object_record_conn(conn, object_hash))
    }

    /// Lit et vérifie les octets d'une référence publiée `live` en recalculant leur SHA-256.
    ///
    /// # Errors
    /// Retourne [`StoreError::EvidenceUnavailable`] si manquant, non `live` ou corrompu.
    pub fn read_verified_object_bytes(
        &self,
        object_hash: Sha256Digest,
    ) -> Result<Vec<u8>, StoreError> {
        self.read_snapshot(|conn, _ctx| {
            read_verified_object_bytes_conn(conn, &self.config, object_hash)
        })
    }

    /// Produit une sauvegarde complète vérifiée (`PrepareBackup`: barrière durable, `step(-1)`, pins, copies,
    /// `integrity_check`, `foreign_key_check`, manifeste synchronisé, rotation 7+4).
    ///
    /// # Errors
    /// Retourne [`StoreError`] si la copie ou la vérification échoue.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "API publique orientée requête : consomme PrepareBackupRequest par valeur pour appel ergonomique"
    )]
    pub fn prepare_backup(&self, req: PrepareBackupRequest) -> Result<BackupManifest, StoreError> {
        prepare_backup_workflow(
            &self.config,
            &self.writer,
            self.config.cutpoints.as_ref(),
            &req,
        )
    }

    /// Abandonne explicitement une sauvegarde interrompue et libère sa barrière/ses pins associés (`storage-model.md`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] si la sauvegarde est introuvable, déjà complète ou en erreur SQL.
    pub fn abandon_backup_job(&self, backup_id: BackupId) -> Result<(), StoreError> {
        self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| tx_abandon_backup_job(tx, now_ms, backup_id),
        )
    }

    /// Valide une sauvegarde existante (`ValidateBackup` : manifeste `complete`, `integrity_check`,
    /// `foreign_key_check` et recalcul SHA-256 de chaque fichier d'objet).
    ///
    /// # Errors
    /// Retourne [`StoreError::RestoreIncomplete`] ou [`StoreError::EvidenceUnavailable`] en cas de défaut.
    pub fn validate_backup(
        backup_dir: &Path,
        config: &StoreConfig,
    ) -> Result<ValidateBackupReport, StoreError> {
        validate_backup_dir(backup_dir, config)
    }

    /// Exécute un export borné par quota (8 MiB max, fragments 256 KiB) sur copie contrôlée.
    ///
    /// # Errors
    /// Retourne [`StoreError::ExportQuotaExceeded`] ou [`StoreError::ReadTimeout`] en nettoyant le staging.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "API publique orientée requête : consomme BoundedExportRequest par valeur à l'appel"
    )]
    pub fn export_bounded(
        &self,
        req: BoundedExportRequest,
    ) -> Result<BoundedExportOutcome, StoreError> {
        run_bounded_export(&self.config, &self.readers, &req)
    }

    /// Exécute le protocole offline conservateur de restauration (`RestoreBackup`).
    ///
    /// # Errors
    /// Retourne [`StoreError::RestorationBlocked`] si un ancien exécuteur n'est pas neutralisé ou
    /// [`StoreError::RestoreIncomplete`] si la sauvegarde est invalide.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "API publique de bascule offline : consomme RestoreBackupRequest et StoreConfig cible"
    )]
    pub fn restore_backup(
        req: RestoreBackupRequest,
        target_config: StoreConfig,
    ) -> Result<RestoreActivationResult, StoreError> {
        execute_restore_backup(&req, &target_config)
    }

    /// Lit le manifeste privé de maintenance `0600` (`InspectRecovery` lorsque source corrompue).
    ///
    /// # Errors
    /// Retourne [`StoreError`] si le manifeste est absent ou corrompu.
    pub fn read_maintenance_restore_manifest(
        maintenance_dir: &Path,
    ) -> Result<Option<MaintenanceRestoreManifest>, StoreError> {
        read_maintenance_manifest(maintenance_dir)
    }

    /// Déclenche une vérification et expiration des lecteurs dépassant 5 s ainsi qu'un checkpoint WAL.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de checkpoint.
    pub fn run_wal_maintenance_tick(&self) -> Result<WalTelemetry, StoreError> {
        self.writer.maintenance_tick(false)
    }

    /// Exécute un `PRAGMA wal_checkpoint(TRUNCATE)` explicite de maintenance.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur `SQLite`.
    pub fn checkpoint_truncate(&self) -> Result<WalTelemetry, StoreError> {
        self.writer.maintenance_tick(true)
    }

    /// Renvoie la télémétrie courante du writer, du WAL et des lecteurs.
    #[must_use]
    pub fn wal_telemetry(&self) -> WalTelemetry {
        self.writer.telemetry()
    }

    /// Ferme proprement le thread writer et les ressources du store.
    ///
    /// # Errors
    /// Retourne [`StoreError`] si l'arrêt rencontre une erreur.
    pub fn shutdown(mut self) -> Result<(), StoreError> {
        tracing::info!("arrêt du thread writer UnleashStore");
        self.writer.shutdown();
        Ok(())
    }

    /// Helper d'inspection directe des scopes gelés dans une base restaurée.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn list_frozen_scopes(&self) -> Result<Vec<(String, bool, String)>, StoreError> {
        self.read_snapshot(|conn, _ctx| {
            let mut stmt = conn
                .prepare(
                    "SELECT scope_id, global_freeze, state
                     FROM recovery_scope_freezes ORDER BY scope_id ASC",
                )
                .map_err(StoreError::from_rusqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    let sid: String = r.get(0)?;
                    let gf: i64 = r.get(1)?;
                    let st: String = r.get(2)?;
                    Ok((sid, gf != 0, st))
                })
                .map_err(StoreError::from_rusqlite)?;
            let mut out = Vec::new();
            for item in rows {
                out.push(item.map_err(StoreError::from_rusqlite)?);
            }
            Ok(out)
        })
    }

    /// Helper d'inspection de la lignée active attendue.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn active_authority_generation(&self) -> Result<AuthorityGeneration, StoreError> {
        self.daemon_state().map(|d| d.authority_generation)
    }

    /// Ouvre une transaction de lecture `query_only=ON` maintenue ouverte jusqu'à libération ou expiration de 5 s.
    ///
    /// # Errors
    /// Retourne [`StoreError`] si l'ouverture échoue.
    pub fn begin_held_reader(&self) -> Result<HeldReaderSession, StoreError> {
        spawn_held_reader_session(&self.config, &self.readers)
    }

    /// Exécute une closure atomique dans une transaction `BEGIN IMMEDIATE` sur le writer unique.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'échec ou de pression WAL/file.
    pub fn execute_in_writer_tx<T, F>(
        &self,
        criticality: WriteCriticality,
        f: F,
    ) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>, TimestampMs) -> Result<T, StoreError> + Send + 'static,
    {
        let cfg = self.config.clone();
        let enforce_cap = criticality == WriteCriticality::Normal;
        self.writer.submit_blocking(criticality, move |conn, now| {
            run_guarded_immediate_tx(conn, &cfg, criticality, enforce_cap, now, f)
        })
    }

    /// Enregistre une entrée `principals` pour satisfaire les FKs lors des tests et de l'amorçage.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn register_principal(
        &self,
        principal_id: &str,
        principal_kind: &str,
        actor_id: &str,
    ) -> Result<(), StoreError> {
        let p_id = principal_id.to_owned();
        let p_kind = principal_kind.to_owned();
        let a_id = actor_id.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                tx.execute(
                    "INSERT INTO principals (principal_id, principal_kind, actor_id, state, created_at_ms)
                     VALUES (?1, ?2, ?3, 'active', ?4)
                     ON CONFLICT(principal_id) DO UPDATE SET
                       principal_kind = excluded.principal_kind,
                       actor_id = excluded.actor_id",
                    rusqlite::params![p_id, p_kind, a_id, now_ms.as_i64()],
                )
                .map_err(StoreError::from_rusqlite)?;
                Ok(())
            })
    }

    /// Enregistre une version de workflow pour satisfaire les FKs de `workflow_runs`.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn register_workflow_version(
        &self,
        workflow_version_id: &str,
        visibility: &str,
        content_hash: Sha256Digest,
    ) -> Result<(), StoreError> {
        let wv_id = workflow_version_id.to_owned();
        let vis = visibility.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                tx.execute(
                    "INSERT INTO workflow_versions
                     (workflow_version_id, registry_version_id, schema_version, visibility, content_hash, created_at_ms)
                     VALUES (?1, NULL, 1, ?2, ?3, ?4)
                     ON CONFLICT(workflow_version_id) DO NOTHING",
                    rusqlite::params![wv_id, vis, content_hash.as_prefixed_str(), now_ms.as_i64()],
                )
                .map_err(StoreError::from_rusqlite)?;
                Ok(())
            })
    }

    /// Lève explicitement le gel d'un scope après disposition autorisée (F018).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn dispose_frozen_scope(
        &self,
        scope_id: &str,
        disposition_receipt_id: ReceiptId,
    ) -> Result<usize, StoreError> {
        let sid = scope_id.to_owned();
        let rcpt = disposition_receipt_id.to_string();
        self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| {
                let updated = tx
                    .execute(
                        "UPDATE recovery_scope_freezes
                         SET state = 'disposed', disposed_at_ms = ?1, disposition_receipt_id = ?2
                         WHERE scope_id = ?3 AND state = 'frozen'",
                        rusqlite::params![now_ms.as_i64(), rcpt, sid],
                    )
                    .map_err(StoreError::from_rusqlite)?;
                Ok(updated)
            },
        )
    }
}
