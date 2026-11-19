//! Façade principale `UnleashStore` coordonnant writer unique, lecteurs, objets, sauvegardes et restauration.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};
use unleash_domain::{
    AccountId, ActionId, ActorId, AuthorityGeneration, BackupId, CommandId, DaemonEpoch,
    DaemonInstanceId, LeaseId, MicrosAmount, PoolId, ProjectId, Sequence, Sha256Digest, StreamId,
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
    RetentionPurgeReport, RetentionPurgeRequest, read_dead_letters_conn, read_events_with_window,
    tx_ack_outbox, tx_apply_inbox_event, tx_execute_retention_purge, tx_lease_outbox_batch,
    tx_purge_expired_inbox, tx_purge_history, tx_record_non_transient_failure,
    tx_record_transient_failure,
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
use crate::policies_secrets::{
    AlertRecord, GuardConfigRecord, ListAlertsFilter, NewAlertSpec, PolicySummary,
    SecretReferenceStatus, SubjectKind, SubjectSuspensionRecord, get_secret_reference_status_conn,
    list_alerts_conn, list_secret_references_conn, read_guard_config_conn,
    read_policy_summary_conn, read_subject_suspension_conn, record_alert_tx,
};
use crate::projects_settings::{
    ProjectRecord, RetentionClass, SettingsRecord, SettingsScopeKind, get_project_conn,
    list_projects_conn, read_settings_conn,
};
use crate::qualification::{
    SqliteRuntimeManifest, apply_and_verify_writer_pragmas, inspect_and_verify_build,
};
use crate::recovery_ops::{
    ActionDispatchInput, RecoveryReport, inspect_recovery_conn, inspect_recovery_offline_cfg,
    lookup_restore_receipt_cfg, reconcile_recovered_actions_after_restart_tx,
    record_action_dispatch_tx,
};
use crate::restore::{
    MaintenanceRestoreManifest, RestoreActivationResult, RestoreBackupRequest,
    execute_restore_backup, read_maintenance_manifest_for_diagnostics,
};
use crate::writer::{
    HeldReaderSession, ReaderContext, ReaderRegistry, StoreWriterHandle, WalTelemetry,
    open_reader_connection, run_guarded_immediate_tx, spawn_held_reader_session,
};

// Le contrôle RO existant est exécuté une fois, avant toute initialisation de fichiers.
// La validation propre aux migrations après ouverture RW reste distincte et inchangée.
fn readonly_open_preflight(
    config: &StoreConfig,
    pre_migration_backup_id: Option<BackupId>,
) -> Result<(), StoreError> {
    // Pré-contrôle strictement en lecture seule sur base existante (zéro écriture si refus)
    if config.db_path.try_exists()? {
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

    Ok(())
}

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

/// Projection et curseur de la même vue `SQLite`, sans SQL dans les clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotProjection<T> {
    /// Projection typée.
    pub value: T,
    /// Watermark du journal dans la transaction de lecture.
    pub cursor: unleash_protocol::EventCursor,
}

/// Frontière atteinte lorsqu'une ouverture échoue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOpenStage {
    /// Contrôles existants RO, avant initialisation FS ou connexion `SQLite` RW.
    ReadOnlyPreflight,
    /// Initialisation, connexion RW, migrations ou démarrage du writer.
    AfterPreflight,
}

/// Échec d'ouverture attribué à une phase précise, sans relancer le préflight.
#[derive(Debug, thiserror::Error)]
#[error("Store open failed at {stage:?}: {error}")]
pub struct StoreOpenFailure {
    /// Phase attestée par le chemin réellement exécuté.
    pub stage: StoreOpenStage,
    /// Erreur d'origine conservée.
    #[source]
    pub error: StoreError,
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
    /// Lit la préparation terminal et le watermark dans la même vue.
    /// # Errors
    /// Retourne les erreurs de lecture du Store.
    pub fn inspect_terminal_allocation_snapshot(
        &self,
        terminal: unleash_domain::TerminalId,
    ) -> Result<SnapshotProjection<Option<crate::TerminalAllocationRecord>>, StoreError> {
        self.project_snapshot(|conn| crate::terminal_allocations::read_conn(conn, terminal))
    }

    /// Consomme une seule fois le dispatch courant sans lancer de processus.
    /// # Errors
    /// Refuse toute divergence, lease expirée ou résolution déjà engagée.
    pub fn resolve_terminal_launch(
        &self,
        request: crate::ResolveTerminalLaunchRequest,
    ) -> Result<crate::ResolvedTerminalLaunch, StoreError> {
        let cfg = self.config.clone();
        let result = self.writer.submit_immediate_blocking(
            WriteCriticality::Normal,
            true,
            move |tx, now| crate::terminal_allocations::resolve_tx(tx, &cfg, now, &request),
        )?;
        self.config
            .cutpoints
            .hit(StoreCutpoint::C2AfterCommandCommit)?;
        Ok(result)
    }
    /// Lit la demande réservée et l'état réel de son job dans le même snapshot.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture du Store.
    pub fn inspect_backup_request(
        &self,
        backup_id: BackupId,
    ) -> Result<Option<crate::BackupRequestRecord>, StoreError> {
        self.read_snapshot(move |conn, _| {
            crate::backup_requests::read_backup_request_conn(conn, backup_id)
        })
    }

    /// Demande, job, pins, manifeste attesté et watermark dans le même snapshot.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture ou d'intégrité de la projection.
    pub fn inspect_backup_request_snapshot(
        &self,
        backup_id: BackupId,
    ) -> Result<SnapshotProjection<Option<crate::BackupRequestRecord>>, StoreError> {
        self.project_snapshot(move |conn| {
            crate::backup_requests::read_backup_request_conn(conn, backup_id)
        })
    }

    /// Prend une demande une seule fois avant Guard ; une issue incertaine exige inspection.
    ///
    /// # Errors
    /// Refuse les lignées, états, principaux ou scopes incompatibles.
    pub fn claim_backup_request(
        &self,
        backup_id: BackupId,
        authority_generation: AuthorityGeneration,
    ) -> Result<crate::BackupRequestRecord, StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, true, move |tx, _| {
                crate::backup_requests::claim_backup_request_tx(tx, backup_id, authority_generation)
            })
    }

    /// Bloque une demande avec motif attesté uniquement avant tout marqueur d'effet.
    ///
    /// # Errors
    /// Refuse les autorités périmées, motifs non prouvés et tout effet potentiellement parti.
    pub fn block_claimed_backup(
        &self,
        backup_id: BackupId,
        authority_generation: AuthorityGeneration,
        reason: crate::BackupBlockReason,
    ) -> Result<crate::BackupRequestRecord, StoreError> {
        let config = self.config.clone();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, now| {
                crate::backup_requests::block_claimed_backup_tx(
                    tx,
                    &config,
                    now,
                    backup_id,
                    authority_generation,
                    reason,
                )
            })
    }

    /// Consomme une réservation une fois et exécute sa sauvegarde après le marqueur durable.
    ///
    /// # Errors
    /// Tout résultat incertain impose inspection ; aucun rejeu n'autorise un second effet.
    pub fn execute_reserved_backup(
        &self,
        request: crate::ExecuteReservedBackupRequest,
    ) -> Result<crate::ExecutedBackup, StoreError> {
        let config = self.config.clone();
        let consumed = self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now| {
                crate::backup_requests::consume_backup_request_tx(tx, &config, now, request)
            },
        )?;
        crate::backup::execute_reserved_backup_workflow(
            &self.config,
            &self.writer,
            self.config.cutpoints.as_ref(),
            request,
            consumed,
        )
    }
    /// Engage atomiquement une phase du broker et son événement, avant tout effet.
    ///
    /// # Errors
    /// Refuse les divergences, autorités périmées et tout dispatch déjà durable.
    pub fn commit_broker_phase(
        &self,
        request: crate::BrokerPhaseRequest,
    ) -> Result<crate::BrokerPhaseCommit, StoreError> {
        let cutpoint = match &request.phase {
            crate::BrokerPhase::Authorizing { .. } => {
                Some(StoreCutpoint::C3AfterBrokerIntentCommit)
            }
            crate::BrokerPhase::Dispatch { .. } => Some(StoreCutpoint::C4AfterBrokerDispatchCommit),
            _ => None,
        };
        let config = self.config.clone();
        let committed = self.writer.submit_immediate_blocking(
            WriteCriticality::Normal,
            true,
            move |tx, now| crate::broker_writer::commit_phase_tx(tx, &config, now, &request),
        )?;
        if let Some(point) = cutpoint {
            self.config.cutpoints.hit(point)?;
        }
        Ok(committed)
    }

    /// Lit l'identité et le CAS d'une action du journal broker.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture du Store.
    pub fn inspect_broker_action(
        &self,
        action_id: ActionId,
    ) -> Result<Option<crate::BrokerActionSnapshot>, StoreError> {
        self.read_snapshot(move |conn, _| {
            crate::broker_writer::inspect_action_conn(conn, action_id)
        })
    }

    /// Lit le contexte réel de préparation d'un probe, sans accès réseau ni secret.
    /// # Errors
    /// Refuse un document durable illisible ou un digest divergent.
    pub fn inspect_provider_probe(
        &self,
        probe_id: unleash_domain::ProviderProbeId,
    ) -> Result<Option<crate::PreparedProviderProbe>, StoreError> {
        self.read_snapshot(|conn, _| crate::provider_probes::read_context(conn, probe_id))
    }

    /// Résout uniquement la version configurée exacte, avec watermark du même snapshot.
    /// Les références historiques non attestées ne sont pas promues.
    /// # Errors
    /// Refuse un contenu incohérent ou une version hors plage `SQLite`.
    pub fn resolve_provider_profile_snapshot(
        &self,
        profile_id: uuid::Uuid,
        profile_version: u64,
    ) -> Result<SnapshotProjection<Option<crate::ConfiguredProviderProfile>>, StoreError> {
        self.project_snapshot(|conn| {
            crate::provider_profiles::resolve(conn, profile_id, profile_version)
        })
    }

    /// Recherche le reçu actif d'une commande F012 authentifiée, sans préparation.
    /// Un résultat absent n'accorde aucune autorité : le writer répète le lookup.
    /// # Errors
    /// Refuse acteur/rôle/enveloppe incohérents, lignée périmée, digest divergent ou tombstone.
    pub fn lookup_provider_profile_command(
        &self,
        envelope: &unleash_protocol::CommandEnvelope,
        caller: &crate::CallerAuthority,
    ) -> Result<Option<unleash_protocol::CommandReceipt>, StoreError> {
        crate::provider_profiles::validate_envelope_identity(envelope, caller)?;
        let digest = unleash_protocol::compute_command_digest_v2(envelope)?;
        self.read_snapshot(|conn, _| {
            let daemon = read_daemon_state_conn(conn)?;
            crate::commands::classify_command_replay(
                conn,
                &daemon,
                &caller.actor_id,
                envelope,
                digest,
            )
        })
    }

    /// Recherche le reçu d'une validation locale avant toute préparation dynamique.
    /// # Errors
    /// Refuse acteur/rôle/lignée incohérents, autre digest et réponse tombstonée.
    pub fn lookup_provider_validation_command(
        &self,
        envelope: &unleash_protocol::CommandEnvelope,
        caller: &crate::CallerAuthority,
    ) -> Result<Option<unleash_protocol::CommandReceipt>, StoreError> {
        crate::provider_profile_validation::validate_envelope_identity(envelope, caller)?;
        let digest = unleash_protocol::compute_command_digest_v2(envelope)?;
        self.read_snapshot(|conn, _| {
            let daemon = read_daemon_state_conn(conn)?;
            crate::commands::classify_command_replay(
                conn,
                &daemon,
                &caller.actor_id,
                envelope,
                digest,
            )
        })
    }

    /// Vérifie les liaisons puis les octets réels d'une configuration exacte, sans réseau/coffre.
    /// Cette lecture ne crée aucun reçu ni résultat ; le commit revalide les mêmes faits.
    /// # Errors
    /// Refuse causalité, autorité, portée, liaison ou contenu insuffisamment attestés.
    pub fn prepare_provider_profile_validation(
        &self,
        envelope: &unleash_protocol::CommandEnvelope,
        caller: &crate::CallerAuthority,
    ) -> Result<crate::provider_profile_validation::LocalProviderValidation, StoreError> {
        let envelope = envelope.clone();
        let caller = caller.clone();
        let (mut observation, profile) =
            self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
                let daemon = read_daemon_state_conn(tx)?;
                crate::provider_profile_validation::prepare_tx(tx, &daemon, now, &envelope, &caller)
            })?;
        crate::provider_profile_validation::verify_configuration_bytes(self, &profile)?;
        observation.observed_at_ms = self.config.clock.now_ms();
        Ok(observation)
    }

    /// Projette une version exacte et ses observations locales avec watermark commun.
    /// Aucun champ privé de liaison ou valeur de coffre n'est exposé.
    /// # Errors
    /// Refuse une version hors plage ou une projection durable incohérente.
    pub fn get_provider_profile_snapshot(
        &self,
        profile_id: uuid::Uuid,
        version: u64,
        project: Option<ProjectId>,
    ) -> Result<
        SnapshotProjection<Option<crate::provider_profile_validation::ProviderProfileView>>,
        StoreError,
    > {
        let projection = self.project_snapshot(|conn| {
            crate::provider_profile_validation::read_projection(conn, profile_id, version, project)
        })?;
        let value = projection
            .value
            .map(|value| crate::provider_profile_validation::finish_view(self, value))
            .transpose()?;
        Ok(SnapshotProjection {
            value,
            cursor: projection.cursor,
        })
    }
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
        Self::open_classified_with_backup(config, pre_migration_backup_id)
            .map_err(|failure| failure.error)
    }

    /// Ouvre en attribuant chaque échec à la frontière réellement franchie.
    ///
    /// # Errors
    /// Seul `ReadOnlyPreflight` atteste un refus avant toute initialisation FS/connexion RW.
    pub fn open_classified(config: StoreConfig) -> Result<Self, StoreOpenFailure> {
        Self::open_classified_with_backup(config, None)
    }

    fn open_classified_with_backup(
        config: StoreConfig,
        pre_migration_backup_id: Option<BackupId>,
    ) -> Result<Self, StoreOpenFailure> {
        readonly_open_preflight(&config, pre_migration_backup_id).map_err(|error| {
            StoreOpenFailure {
                stage: StoreOpenStage::ReadOnlyPreflight,
                error,
            }
        })?;
        Self::open_after_preflight(config, pre_migration_backup_id).map_err(|error| {
            StoreOpenFailure {
                stage: StoreOpenStage::AfterPreflight,
                error,
            }
        })
    }

    fn open_after_preflight(
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

    // F012 replays are reads of the authoritative wire receipt, not revalidation
    // of a newly resolved configuration. The writer repeats this classifier.
    fn preflight_provider_profile(
        &self,
        request: &CommitCommandRequest,
        digest: Sha256Digest,
    ) -> Result<Option<CommitOutcome>, StoreError> {
        let configuration = crate::provider_profiles::validate_identity(request)?;
        let validation = crate::provider_profile_validation::validate_identity(request)?;
        if !configuration && !validation {
            return Ok(None);
        }
        let lookup = || {
            self.read_snapshot(|conn, _| {
                let daemon = read_daemon_state_conn(conn)?;
                crate::commands::classify_command_replay(
                    conn,
                    &daemon,
                    &request.actor_id,
                    &request.envelope,
                    digest,
                )
            })
        };
        if let Some(receipt) = lookup()? {
            return Ok(Some(CommitOutcome::IdempotentReplay(receipt)));
        }
        let verified = if validation {
            crate::provider_profile_validation::verify_published(self, request)
        } else {
            crate::provider_profiles::verify_published(self, request)
        };
        if let Err(error) = verified {
            // A concurrent writer may have committed while object verification failed.
            if let Some(receipt) = lookup()? {
                return Ok(Some(CommitOutcome::IdempotentReplay(receipt)));
            }
            return Err(error);
        }
        Ok(None)
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
        let is_purge = matches!(
            req.mutation,
            crate::commands::CommandMutation::PurgeHistory { .. }
        );
        let (digest, retention_days) = self.run_command_prechecks(&req)?;
        if let Some(replay) = self.preflight_provider_profile(&req, digest)? {
            return Ok(replay);
        }
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
        if is_purge {
            finish_receipt_retention_purges(
                &self.config,
                &self.writer,
                outcome.receipt().receipt_id,
            )?;
        }
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
        let is_purge = matches!(
            req.mutation,
            crate::commands::CommandMutation::PurgeHistory { .. }
        );
        let (digest, retention_days) = self.run_command_prechecks(&req)?;
        if let Some(replay) = self.preflight_provider_profile(&req, digest)? {
            return Ok(replay);
        }
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
        if is_purge {
            let config = self.config.clone();
            let writer = self.writer.submission_client();
            let receipt = outcome.receipt().receipt_id;
            tokio::task::spawn_blocking(move || {
                finish_receipt_retention_purges(&config, &writer, receipt)
            })
            .await
            .map_err(|error| StoreError::StorageUnavailable(error.to_string()))??;
        }
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

    /// Projette les files et réservations réelles dans un seul snapshot `SQLite`.
    /// # Errors
    /// Refuse un propriétaire actif incomplet ou une lecture interrompue ou expirée.
    pub fn execution_admission_snapshot(
        &self,
        pool_id: PoolId,
    ) -> Result<crate::ExecutionAdmissionSnapshot, StoreError> {
        self.read_snapshot(|conn, _| {
            crate::admission::read_snapshot(conn, pool_id, self.config.clock.now_ms())
        })
    }

    /// Conserve l'observation privilégiée de préconditions exactes ; aucun accès client.
    /// # Errors
    /// Refuse ancienne incarnation, révision, claim ou observateur non attribuable.
    pub fn record_admission_eligibility(
        &self,
        observation: crate::AdmissionEligibilityObservation,
    ) -> Result<(), StoreError> {
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            crate::admission::record_eligibility(tx, now, &observation)
        })
    }

    /// Consulte la politique pure sous le writer, puis engage tous ses effets atomiquement.
    /// La closure ne fait aucune I/O et ne doit pas rappeler le Store.
    /// # Errors
    /// Refuse un snapshot périmé, une précondition absente, un fence ou des deltas invalides.
    pub fn commit_execution_admission<F>(
        &self,
        pool_id: PoolId,
        source_revision: crate::AdmissionSnapshotToken,
        decide: F,
    ) -> Result<crate::AdmissionOutcome, StoreError>
    where
        F: FnOnce(
                &crate::ExecutionAdmissionSnapshot,
            ) -> Result<crate::AdmissionDecision, StoreError>
            + Send
            + 'static,
    {
        self.commit_execution_admission_with_report(pool_id, source_revision, move |snapshot| {
            decide(snapshot).map(|decision| (decision, ()))
        })
        .map(|(outcome, ())| outcome)
    }

    /// Engage une décision d'admission et restitue son résultat de politique.
    ///
    /// La closure travaille sur le snapshot relu sous le writer, sans I/O et sans
    /// rappel au Store. Le rapport n'est rendu qu'en cas de succès du writer.
    ///
    /// # Errors
    ///
    /// Refuse un snapshot périmé, une précondition absente, un fence ou des deltas invalides.
    pub fn commit_execution_admission_with_report<F, R>(
        &self,
        pool_id: PoolId,
        source_revision: crate::AdmissionSnapshotToken,
        decide: F,
    ) -> Result<(crate::AdmissionOutcome, R), StoreError>
    where
        F: FnOnce(
                &crate::ExecutionAdmissionSnapshot,
            ) -> Result<(crate::AdmissionDecision, R), StoreError>
            + Send
            + 'static,
        R: Send + 'static,
    {
        let config = self.config.clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            crate::admission::commit(tx, &config, now, pool_id, source_revision, decide)
        })
    }

    /// Journalise une demande coopérative ; ne rend aucune capacité.
    /// # Errors
    /// Refuse contrôle, cible ou faisceau obsolète et demande déjà présente.
    pub fn request_execution_checkpoint(
        &self,
        waiting: unleash_domain::InvocationId,
        target: crate::ExecutionContinuation,
    ) -> Result<unleash_protocol::EventCursor, StoreError> {
        let config = self.config.clone();
        self.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
            crate::admission::request_checkpoint(tx, &config, now, waiting, &target)
        })
    }

    /// Lie quatre observations durables à une continuation exacte, sans la libérer.
    /// Le producteur interne doit réellement observer le host et le canal modèle.
    /// # Errors
    /// Refuse claim, référence absente, autre identité/fence ou rejeu divergent.
    pub fn record_execution_checkpoint(
        &self,
        observation: crate::ExecutionCheckpointObservation,
    ) -> Result<(), StoreError> {
        self.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
            crate::admission::record_checkpoint(tx, now, &observation)
        })
    }

    /// Termine la tentative, neutralise sa lease et rend son unité atomiquement.
    /// # Errors
    /// Sans preuves persistées et toujours valides, conserve l'unité intégralement.
    pub fn complete_execution_checkpoint(
        &self,
        reservation: unleash_domain::ReservationId,
        reason: unleash_domain::WaitReason,
    ) -> Result<unleash_protocol::EventCursor, StoreError> {
        let config = self.config.clone();
        self.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
            crate::admission::complete_checkpoint(tx, &config, now, reservation, reason)
        })
    }

    /// Remet en file une unité dont le checkpoint a été disposé dans cette incarnation.
    /// # Errors
    /// Refuse un terminal, une ancienne lignée ou une continuation encore active.
    pub fn requeue_checkpointed_execution(
        &self,
        unit: crate::AdmissionUnit,
        expected: unleash_domain::Revision,
    ) -> Result<unleash_protocol::EventCursor, StoreError> {
        let config = self.config.clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            crate::admission::requeue(tx, &config, now, unit, expected)
        })
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

    /// Publie une référence et rend sa poignée exacte pour le retrait du seul owner.
    /// Le producteur fournit une identité de tentative distincte ; aucune référence
    /// partageant seulement le hash ne sera retirée par cette poignée.
    /// # Errors
    /// Refuse octets, pin, génération ou lignée incohérents.
    pub fn publish_owned_object_reference(
        &self,
        request: PublishObjectReferenceRequest,
    ) -> Result<crate::objects::PublishedObjectReference, StoreError> {
        let record = self.object_record(request.object_hash)?;
        verify_object_file_before_publish(
            &self.config,
            &record.relative_path,
            request.object_hash,
        )?;
        let ids = Arc::clone(&self.config.id_generator);
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now| {
                let daemon = read_daemon_state_tx(tx)?;
                crate::objects::tx_publish_owned_reference(
                    tx,
                    ids.as_ref(),
                    now,
                    &request,
                    daemon.authority_generation,
                )
            })
    }

    /// Retire uniquement la référence identifiée par la poignée de son producteur.
    /// Ne supprime aucun octet et ne touche pas les autres références du même hash.
    /// # Errors
    /// Refuse une poignée d'ancienne lignée ou une erreur du writer.
    pub fn retire_owned_object_reference(
        &self,
        handle: crate::objects::PublishedObjectReference,
        reason: &str,
    ) -> Result<usize, StoreError> {
        let reason = reason.to_owned();
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _| {
                let daemon = read_daemon_state_tx(tx)?;
                crate::objects::tx_retire_owned_reference(
                    tx,
                    &handle,
                    daemon.authority_generation,
                    &reason,
                )
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
        let report =
            reconcile_objects_workflow(&self.config, &self.writer, self.config.cutpoints.as_ref())?;
        let purges =
            self.read_snapshot(|conn, _| crate::events::read_retention_purge_statuses_conn(conn))?;
        for purge in purges {
            // Le flag prouve un checkpoint post-suppression ; il ne permet pas de sauter
            // celui de reprise après un crash entre l'audit et le dernier TRUNCATE.
            if purge.state != "quarantined" {
                finish_retention_purge(&self.config, &self.writer, &purge.purge_id)?;
            }
        }
        Ok(report)
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
    /// Retourne [`StoreError`] si le manifeste n'est pas privé, borné ou conforme ; absent renvoie `None`.
    pub fn read_maintenance_restore_manifest(
        maintenance_dir: &Path,
    ) -> Result<Option<MaintenanceRestoreManifest>, StoreError> {
        read_maintenance_manifest_for_diagnostics(maintenance_dir)
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

    /// Engage atomiquement une décision interne dédiée et lève son seul gel de restauration.
    ///
    /// Aucun reçu d'une autre opération n'autorise cette décision. Les effets et coûts restent historiques.
    ///
    /// # Errors
    /// Refuse les autorités, politiques, restaurations ou reçus sans lien exact avec la décision.
    #[expect(
        clippy::too_many_lines,
        reason = "Décision de scope, reçu canonique et levée du gel atomiques sous le writer"
    )]
    pub fn dispose_frozen_scope(
        &self,
        disposition: crate::recovery_ops::ScopeFreezeDisposition,
    ) -> Result<usize, StoreError> {
        crate::policies_secrets::require_human_decider(
            &disposition.caller,
            "recovery scope disposition",
        )?;
        if disposition.reason.trim().is_empty()
            || disposition.dependency_budget_impact.trim().is_empty()
            || disposition.policy_revision < 0
        {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: "disposition requires reason, policy and dependency/budget impact"
                    .to_owned(),
            });
        }
        let decision_json = serde_json::to_string(&disposition)?;
        let retention_ms =
            i64::from(self.config.default_receipt_retention_days).saturating_mul(86_400_000);
        self.writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, now_ms| {
            let current: Option<(String, String)> = tx.query_row(
                "SELECT f.state, i.target_authority_generation FROM recovery_scope_freezes f
                 JOIN restore_imports i ON i.restore_id = f.restore_id WHERE f.scope_id = ?1 AND f.restore_id = ?2",
                rusqlite::params![disposition.scope_id, disposition.restore_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(StoreError::from_rusqlite)?;
            let Some((state, target_authority)) = current else {
                return Err(StoreError::RecoveryInProgress("scope/restore mismatch".to_owned()));
            };
            let daemon = read_daemon_state_conn(tx)?;
            if target_authority != daemon.authority_generation.to_string() {
                return Err(StoreError::RecoveryInProgress("restore is not the active lineage".to_owned()));
            }
            let command_id = CommandId::from_uuid(disposition.decision_receipt_id.as_uuid());
            let payload = serde_json::to_value(&disposition)?.as_object().cloned()
                .ok_or_else(|| StoreError::InvariantViolation("scope decision must be an object".to_owned()))?;
            let envelope = CommandEnvelope {
                protocol_version: unleash_protocol::ProtocolVersion,
                request_id: unleash_domain::RequestId::from_uuid(disposition.decision_receipt_id.as_uuid()),
                command_id, authority_generation: daemon.authority_generation,
                actor_ref: Some(disposition.caller.actor_id.to_string()), project_id: None,
                aggregate_id: disposition.restore_id.as_uuid(), expected_revision: unleash_domain::Revision::INITIAL,
                command_type: "DisposeRecoveryScope".to_owned(), payload,
            };
            let digest = unleash_protocol::compute_command_digest_v2(&envelope)?;
            let existing: Option<(String, String, String, String, String)> = tx.query_row(
                "SELECT actor_id, authority_generation, command_type, command_digest, aggregate_id FROM command_receipts
                 WHERE receipt_id = ?1 OR (actor_id = ?2 AND command_id = ?3)",
                rusqlite::params![disposition.decision_receipt_id.to_string(), disposition.caller.actor_id.to_string(), command_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            ).optional().map_err(StoreError::from_rusqlite)?;
            if let Some((actor, authority, command_type, existing_digest, aggregate)) = &existing
                && (actor != disposition.caller.actor_id.as_str()
                    || authority != &daemon.authority_generation.to_string()
                    || command_type != "DisposeRecoveryScope" || existing_digest != &digest.as_prefixed_str()
                    || aggregate != &disposition.restore_id.to_string())
            {
                return Err(StoreError::Unauthorized("receipt is not bound to this explicit scope decision".to_owned()));
            }
            if state != "frozen" {
                let previous: Option<String> = tx.query_row(
                    "SELECT decision_json FROM recovery_scope_dispositions WHERE scope_id = ?1 AND restore_id = ?2",
                    rusqlite::params![disposition.scope_id, disposition.restore_id.to_string()], |row| row.get(0),
                ).optional().map_err(StoreError::from_rusqlite)?;
                return if existing.is_some() && previous.as_deref() == Some(&decision_json) { Ok(0) }
                    else { Err(StoreError::ContractViolation { reason: None, message: "scope already disposed by another decision".to_owned() }) };
            }
            let policy_applies = (disposition.policy_scope_kind == "global" && disposition.policy_scope_id == "global")
                || (disposition.policy_scope_kind == "project" && disposition.policy_scope_id == disposition.scope_id);
            let revision: i64 = tx.query_row(
                "SELECT COALESCE(MAX(revision), 0) FROM security_policies WHERE scope_kind = ?1 AND scope_id = ?2",
                rusqlite::params![disposition.policy_scope_kind, disposition.policy_scope_id], |row| row.get(0),
            ).map_err(StoreError::from_rusqlite)?;
            if !policy_applies || revision != disposition.policy_revision {
                return Err(StoreError::Unauthorized("scope disposition policy mismatch".to_owned()));
            }
            if existing.is_some() {
                return Err(StoreError::ContractViolation { reason: None, message: "decision receipt without atomic disposition".to_owned() });
            }
            let sequence: i64 = tx.query_row("SELECT COALESCE(MAX(sequence), 0) FROM events WHERE stream_id = ?1",
                [daemon.stream_id.to_string()], |row| row.get(0)).map_err(StoreError::from_rusqlite)?;
            let receipt = unleash_protocol::CommandReceipt {
                receipt_id: disposition.decision_receipt_id, command_id,
                accepted_at: chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms.as_i64()).unwrap_or(chrono::DateTime::UNIX_EPOCH),
                aggregate_id: disposition.restore_id.as_uuid(), new_revision: unleash_domain::Revision::new(1),
                operation_id: None, state: unleash_protocol::ReceiptState::Completed,
                event_cursor: unleash_protocol::EventCursor::new(daemon.stream_id, Sequence::new(u64::try_from(sequence).unwrap_or(0))),
                result_ref: Some(format!("recovery-scope:{}", disposition.scope_id)),
            };
            tx.execute(
                "INSERT INTO command_receipts
                 (actor_id, command_id, authority_generation, receipt_id, request_id, command_type, command_digest,
                  aggregate_id, state, response_state, response_json, response_retained_until_ms, first_event_seq, last_event_seq, committed_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'DisposeRecoveryScope', ?6, ?7, 'completed', 'full', ?8, ?9, NULL, NULL, ?10)",
                rusqlite::params![disposition.caller.actor_id.to_string(), command_id.to_string(), daemon.authority_generation.to_string(),
                    disposition.decision_receipt_id.to_string(), envelope.request_id.to_string(), digest.as_prefixed_str(),
                    disposition.restore_id.to_string(), serde_json::to_string(&receipt)?, now_ms.as_i64().saturating_add(retention_ms), now_ms.as_i64()],
            ).map_err(StoreError::from_rusqlite)?;
            tx.execute(
                "INSERT INTO recovery_scope_dispositions (scope_id, restore_id, decision_receipt_id, decided_by_actor, decision_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![disposition.scope_id, disposition.restore_id.to_string(), disposition.decision_receipt_id.to_string(),
                    disposition.caller.actor_id.to_string(), decision_json, now_ms.as_i64()],
            ).map_err(StoreError::from_rusqlite)?;
            tx.execute(
                "UPDATE recovery_scope_freezes SET state = 'disposed', disposed_at_ms = ?1, disposition_receipt_id = ?2
                 WHERE scope_id = ?3 AND restore_id = ?4 AND state = 'frozen'",
                rusqlite::params![now_ms.as_i64(), disposition.decision_receipt_id.to_string(), disposition.scope_id, disposition.restore_id.to_string()],
            ).map_err(StoreError::from_rusqlite)
        })
    }

    fn project_snapshot<T>(
        &self,
        read: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<SnapshotProjection<T>, StoreError> {
        self.read_snapshot(|conn, _ctx| {
            let daemon = read_daemon_state_conn(conn)?;
            let high: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE stream_id = ?1",
                    [daemon.stream_id.to_string()],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            let value = read(conn)?;
            Ok(SnapshotProjection {
                value,
                cursor: unleash_protocol::EventCursor::new(
                    daemon.stream_id,
                    Sequence::new(u64::try_from(high).unwrap_or(0)),
                ),
            })
        })
    }

    /// Liste projets et watermark dans une seule transaction lectrice.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture du store.
    pub fn list_projects_snapshot(
        &self,
    ) -> Result<SnapshotProjection<Vec<ProjectRecord>>, StoreError> {
        self.project_snapshot(list_projects_conn)
    }

    /// Lit projet et watermark dans une seule transaction lectrice.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture du store.
    pub fn get_project_snapshot(
        &self,
        project_id: ProjectId,
    ) -> Result<SnapshotProjection<ProjectRecord>, StoreError> {
        self.project_snapshot(|conn| get_project_conn(conn, project_id))
    }

    /// Lit réglages et watermark dans une seule transaction lectrice.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture du store.
    pub fn read_settings_snapshot(
        &self,
        scope_kind: SettingsScopeKind,
        scope_id: &str,
    ) -> Result<SnapshotProjection<SettingsRecord>, StoreError> {
        let now_ms = self.config.clock.now_ms();
        self.project_snapshot(|conn| read_settings_conn(conn, now_ms, scope_kind, scope_id))
    }

    /// Lit résumé de politique et watermark dans une seule transaction lectrice.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture du store.
    pub fn read_policy_summary_snapshot(
        &self,
        project_id: Option<ProjectId>,
    ) -> Result<SnapshotProjection<PolicySummary>, StoreError> {
        self.project_snapshot(|conn| read_policy_summary_conn(conn, project_id))
    }

    /// Lit récupération et watermark dans une seule transaction lectrice.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture du store.
    pub fn inspect_recovery_snapshot(
        &self,
    ) -> Result<SnapshotProjection<RecoveryReport>, StoreError> {
        self.project_snapshot(|conn| inspect_recovery_conn(conn, &self.config))
    }
    /// Liste tous les projets persistés (`ListProjects`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn list_projects(&self) -> Result<Vec<ProjectRecord>, StoreError> {
        self.read_snapshot(|conn, _ctx| list_projects_conn(conn))
    }

    /// Lit un projet par identifiant (`GetProject`).
    ///
    /// # Errors
    /// Retourne [`StoreError::NotFound`] si absent ou une erreur de lecture.
    pub fn get_project(&self, project_id: ProjectId) -> Result<ProjectRecord, StoreError> {
        self.read_snapshot(|conn, _ctx| get_project_conn(conn, project_id))
    }

    /// Lit les paramètres typés d'une portée (`ReadSettings`), synthétisant les défauts Q-15/Q-12 si vierge.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn read_settings(
        &self,
        scope_kind: SettingsScopeKind,
        scope_id: &str,
    ) -> Result<SettingsRecord, StoreError> {
        let now_ms = self.config.clock.now_ms();
        self.read_snapshot(|conn, _ctx| read_settings_conn(conn, now_ms, scope_kind, scope_id))
    }

    /// Lit le résumé agrégé de politique d'isolation et de configuration AI-Guard (`ReadPolicySummary`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn read_policy_summary(
        &self,
        project_id: Option<ProjectId>,
    ) -> Result<PolicySummary, StoreError> {
        self.read_snapshot(|conn, _ctx| read_policy_summary_conn(conn, project_id))
    }

    /// Lit une configuration AI-Guard par identifiant (`"default"`, etc.).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn read_guard_config(
        &self,
        config_id: &str,
    ) -> Result<Option<GuardConfigRecord>, StoreError> {
        self.read_snapshot(|conn, _ctx| read_guard_config_conn(conn, config_id))
    }

    /// Lit l'état courant de suspension/révocation d'un sujet typé.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn get_subject_suspension(
        &self,
        kind: SubjectKind,
        subject_id: &str,
        scope_kind: &str,
        scope_id: &str,
    ) -> Result<Option<SubjectSuspensionRecord>, StoreError> {
        self.read_snapshot(|conn, _ctx| {
            read_subject_suspension_conn(conn, kind, subject_id, scope_kind, scope_id)
        })
    }

    /// Retourne les métadonnées d'une référence secrète opaque (`GetSecretReferenceStatus`, zéro octet secret).
    ///
    /// # Errors
    /// Retourne [`StoreError::NotFound`] si absente ou une erreur de lecture.
    pub fn get_secret_reference_status(
        &self,
        secret_ref_id: &str,
    ) -> Result<SecretReferenceStatus, StoreError> {
        self.read_snapshot(|conn, _ctx| get_secret_reference_status_conn(conn, secret_ref_id))
    }

    /// Liste les références secrètes enregistrées (métadonnées uniquement, zéro octet secret).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn list_secret_references(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<SecretReferenceStatus>, StoreError> {
        self.read_snapshot(|conn, _ctx| list_secret_references_conn(conn, namespace))
    }

    /// Enregistre ou met à jour une alerte durable dans `alerts`.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL ou de contrainte.
    pub fn record_alert(&self, spec: NewAlertSpec) -> Result<AlertRecord, StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                record_alert_tx(tx, now_ms, &spec)
            })
    }

    /// Liste les alertes selon filtre, en remontant les alertes bloquantes actives si demandé (`ListAlerts`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn list_alerts(&self, filter: &ListAlertsFilter) -> Result<Vec<AlertRecord>, StoreError> {
        self.read_snapshot(|conn, _ctx| list_alerts_conn(conn, filter))
    }

    /// Enregistre une tentative de dispatch d'action dans `action_dispatches`.
    ///
    /// # Errors
    /// Retourne [`StoreError::NotFound`] si l'action n'existe pas ou une erreur SQL.
    pub fn record_action_dispatch(&self, input: ActionDispatchInput) -> Result<(), StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, now_ms| {
                record_action_dispatch_tx(tx, now_ms, &input)
            })
    }

    /// Classe en `outcome_unknown` (sans retry automatique) les actions laissées `dispatching`/`in_flight`
    /// sous une époque antérieure après redémarrage dans la même lignée.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL.
    pub fn reconcile_recovered_actions_after_restart(&self) -> Result<Vec<ActionId>, StoreError> {
        self.writer
            .submit_immediate_blocking(WriteCriticality::Critical, false, |tx, now_ms| {
                reconcile_recovered_actions_after_restart_tx(tx, now_ms)
            })
    }

    /// Inspecte l'inventaire et l'état de récupération complet (`InspectRecovery`).
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur de lecture.
    pub fn inspect_recovery(&self) -> Result<RecoveryReport, StoreError> {
        self.read_snapshot(|conn, _ctx| inspect_recovery_conn(conn, &self.config))
    }

    /// Inspecte l'état de récupération hors-ligne, y compris quand la base source est corrompue (`0600` manifest).
    ///
    /// # Errors
    /// Retourne [`StoreError`] si la lecture du manifeste privé échoue.
    pub fn inspect_recovery_offline(config: &StoreConfig) -> Result<RecoveryReport, StoreError> {
        inspect_recovery_offline_cfg(config)
    }

    /// Recherche un reçu `RestoreBackup` par `(actor_id, command_id)` via manifeste privé `0600`
    /// avant bascule ou `restore_manifest_records` après bascule.
    ///
    /// # Errors
    /// Retourne [`StoreError`] si le manifeste ou la base est illisible de manière inattendue.
    pub fn lookup_restore_receipt(
        config: &StoreConfig,
        actor_id: &str,
        command_id: CommandId,
    ) -> Result<Option<MaintenanceRestoreManifest>, StoreError> {
        lookup_restore_receipt_cfg(config, actor_id, command_id)
    }

    /// Attribue une classe de rétention Q-12 et un éventuel verrou `keep_by_policy` à un objet live.
    ///
    /// # Errors
    /// Retourne [`StoreError::NotFound`] si l'objet est absent ou une erreur SQL.
    pub fn tag_object_retention(
        &self,
        object_hash: Sha256Digest,
        retention_class: RetentionClass,
        keep_by_policy: bool,
        project_id: Option<ProjectId>,
    ) -> Result<(), StoreError> {
        let hash_s = object_hash.as_prefixed_str();
        let cls_s = retention_class.as_str().to_owned();
        let keep_i = i64::from(keep_by_policy);
        let proj_s = project_id.map(|p| p.to_string());
        self.writer
            .submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, _now| {
                let updated = tx
                    .execute(
                        "UPDATE object_records
                         SET retention_class = ?1, keep_by_policy = ?2, project_id = COALESCE(?3, project_id)
                         WHERE object_hash = ?4",
                        rusqlite::params![cls_s, keep_i, proj_s, hash_s],
                    )
                    .map_err(StoreError::from_rusqlite)?;
                if updated == 0 {
                    return Err(StoreError::NotFound(format!("object {hash_s}")));
                }
                Ok(())
            })
    }

    /// Exécute une purge complète sous politique de rétention Q-12 (`unleash.retention.default.v1`),
    /// respecte les préavis >= 24 h et les protections actives, journalise dans `retention_purges`
    /// puis exécute `PRAGMA wal_checkpoint(TRUNCATE)`.
    ///
    /// # Errors
    /// Retourne [`StoreError`] en cas d'erreur SQL ou I/O.
    pub fn execute_retention_purge(
        &self,
        req: RetentionPurgeRequest,
    ) -> Result<RetentionPurgeReport, StoreError> {
        check_injected_fault(
            self.config.fault_injector.as_ref(),
            FaultTarget::Maintenance,
        )?;
        let cfg = self.config.clone();
        let mut report = self.writer.submit_immediate_blocking(
            WriteCriticality::Critical,
            false,
            move |tx, now_ms| tx_execute_retention_purge(tx, &cfg, now_ms, None, None, &req),
        )?;
        if !report.suspended_clock_uncertain {
            let (truncated, bytes) =
                finish_retention_purge(&self.config, &self.writer, &report.purge_id)?;
            report.wal_checkpoint_truncated = truncated;
            report.wal_bytes_after = bytes;
        }
        Ok(report)
    }
}

fn finish_retention_purge(
    config: &StoreConfig,
    writer: &StoreWriterHandle,
    purge_id: &str,
) -> Result<(bool, u64), StoreError> {
    let reason = format!("retention_q12:{purge_id}");
    let intents = writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _| {
        let mut statement = tx.prepare("SELECT delete_operation_id, object_hash, object_generation, relative_path, state
            FROM object_deletions WHERE reason = ?1 AND state IN ('prepared', 'unlinked') ORDER BY delete_operation_id")
            .map_err(StoreError::from_rusqlite)?;
        let rows = statement.query_map([reason], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?, row.get::<_, String>(4)?,
        ))).map_err(StoreError::from_rusqlite)?;
        let mut intents = Vec::new();
        for row in rows {
            let (operation, hash, generation, path, state) = row.map_err(StoreError::from_rusqlite)?;
            intents.push(DeletionIntent { delete_operation_id: operation, object_hash: hash.parse()?,
                object_generation: generation, relative_path: path, state });
        }
        Ok(intents)
    })?;
    for intent in intents {
        execute_unlink_and_finalize_deletion(
            config,
            writer,
            &intent,
            config.cutpoints.as_ref(),
            true,
        )?;
    }
    let id = purge_id.to_owned();
    let incomplete: i64 = writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _| {
        tx.query_row("SELECT COUNT(*) FROM object_deletions d JOIN object_records o ON o.object_hash = d.object_hash
             WHERE d.reason = ?1 AND (d.state != 'completed' OR (o.object_generation = d.object_generation AND o.state != 'tombstone'))",
             [format!("retention_q12:{id}")], |row| row.get(0)).map_err(StoreError::from_rusqlite)
    })?;
    if incomplete != 0 {
        return Err(StoreError::StorageCorrupt(
            "retention deletion remains incomplete or quarantined".to_owned(),
        ));
    }
    check_injected_fault(config.fault_injector.as_ref(), FaultTarget::Maintenance)?;
    let before = writer.telemetry().truncate_checkpoints;
    let initial = writer.maintenance_tick(true)?;
    if initial.truncate_checkpoints <= before || initial.wal_bytes != 0 {
        return Err(StoreError::StorageUnavailable(
            "retention checkpoint remains blocked".to_owned(),
        ));
    }
    let id = purge_id.to_owned();
    writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _| {
        tx.execute(
            "UPDATE retention_purges SET wal_checkpoint_truncated = 1 WHERE purge_id = ?1",
            [id],
        )
        .map_err(StoreError::from_rusqlite)?;
        Ok(())
    })?;
    let final_checkpoint = writer.maintenance_tick(true);
    match final_checkpoint {
        Ok(result)
            if result.truncate_checkpoints > initial.truncate_checkpoints
                && result.wal_bytes == 0 =>
        {
            Ok((true, 0))
        }
        result => {
            let id = purge_id.to_owned();
            writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _| {
                tx.execute(
                    "UPDATE retention_purges SET wal_checkpoint_truncated = 0 WHERE purge_id = ?1",
                    [id],
                )
                .map_err(StoreError::from_rusqlite)?;
                Ok(())
            })?;
            match result {
                Err(error) => Err(error),
                Ok(_) => Err(StoreError::StorageUnavailable(
                    "retention final checkpoint remains blocked".to_owned(),
                )),
            }
        }
    }
}

fn finish_receipt_retention_purges(
    config: &StoreConfig,
    writer: &StoreWriterHandle,
    receipt_id: unleash_domain::ReceiptId,
) -> Result<(), StoreError> {
    let ids =
        writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _| {
            let mut statement = tx
                .prepare("SELECT purge_id FROM retention_purges WHERE command_receipt_id = ?1")
                .map_err(StoreError::from_rusqlite)?;
            let rows = statement
                .query_map([receipt_id.to_string()], |row| row.get::<_, String>(0))
                .map_err(StoreError::from_rusqlite)?;
            rows.map(|row| row.map_err(StoreError::from_rusqlite))
                .collect::<Result<Vec<_>, _>>()
        })?;
    for id in ids {
        finish_retention_purge(config, writer, &id)?;
    }
    Ok(())
}
