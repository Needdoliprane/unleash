//! T-F012-014/018 : contexte de probe sans session, préparation durable avant admission.
#![expect(
    clippy::unwrap_used,
    reason = "Assertions et graphes relationnels synthétiques du banc SQLite"
)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use unleash_domain::{Sha256Digest, TimestampMs};
use unleash_store::{
    PrepareObjectRequest, PublishObjectReferenceRequest, StoreConfig, UnleashStore,
    WriteCriticality,
};
use unleash_testkit::local_test_dir;

const LEGACY_COLUMNS: &str = "manifest_id,model_call_id,manifest_kind,ordinal,parent_manifest_id,context_manifest_id,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,usage_object_hash,result_object_hash,content_object_hash,created_at_ms";

fn manifests(store: &UnleashStore) -> Vec<Vec<rusqlite::types::Value>> {
    store
        .read_snapshot(|conn, _| {
            let mut statement = conn.prepare(&format!(
                "SELECT {LEGACY_COLUMNS} FROM model_call_manifests ORDER BY manifest_id"
            ))?;
            let count = statement.column_count();
            let rows = statement.query_map([], |row| {
                (0..count)
                    .map(|index| row.get(index))
                    .collect::<Result<Vec<_>, _>>()
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .unwrap()
}

fn seed_legacy_manifest(store: &UnleashStore, ambiguous_probe: bool) {
    let hash = publish_fixture(store);
    store
        .register_principal("probe-operator", "user", "probe-operator")
        .unwrap();
    let daemon = store.daemon_state().unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
        tx.execute("INSERT INTO agents(agent_id,level,state,revision) VALUES('legacy-agent','N','eligible',0)",[])?;
        tx.execute("INSERT INTO sessions(session_id,agent_id,revision,state,created_at_ms) VALUES('legacy-session','legacy-agent',0,'open',?1)",[now.as_i64()])?;
        tx.execute("INSERT INTO context_manifests(manifest_id,session_id,base_session_revision,content_hash,schema_version,created_at_ms) VALUES('legacy-context','legacy-session',0,?1,2,?2)",rusqlite::params![hash.to_string(),now.as_i64()])?;
        tx.execute("INSERT INTO effective_bundles(bundle_hash,schema_version,role_version_id,requested_model_profile_version_id,allowed_model_policy_hash,initial_context_manifest_id,scope_id,scope_revision,content_object_hash) VALUES('legacy-bundle',1,'legacy-role','legacy-profile',?1,'legacy-context','global',0,?1)",[hash.to_string()])?;
        tx.execute("INSERT INTO command_receipts(actor_id,command_id,authority_generation,receipt_id,request_id,command_type,command_digest,aggregate_id,state,response_state,response_json,response_retained_until_ms,committed_at_ms) VALUES('probe-operator','legacy-command',?1,'legacy-receipt','legacy-request','TestProviderProfile',?2,'legacy-probe','accepted','full','{}',?3,?4)",rusqlite::params![daemon.authority_generation.to_string(),hash.to_string(),now.as_i64()+86_400_000,now.as_i64()])?;
        tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES('legacy-event',?1,'invocation','legacy-invocation',1,0,'InvocationQueued',1,'probe-operator','probe-operator',?2,?2,'{}','full',?3,'restricted','command','legacy-receipt','legacy-receipt','legacy-command')",rusqlite::params![daemon.stream_id.to_string(),now.as_i64(),hash.to_string()])?;
        tx.execute("INSERT INTO invocations(invocation_id,agent_id,session_id,trigger_event_id,effective_bundle_hash,context_manifest_id,state,admission_class,revision,created_at_ms) VALUES('legacy-invocation','legacy-agent','legacy-session','legacy-event','legacy-bundle','legacy-context','running','work',1,?1)",[now.as_i64()])?;
        tx.execute("INSERT INTO invocation_attempts(attempt_id,invocation_id,ordinal,authority_generation,daemon_epoch,fence,state,started_at_ms) VALUES('legacy-attempt','legacy-invocation',1,?1,?2,1,'running',?3)",rusqlite::params![daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;
        tx.execute("INSERT INTO actions(action_id,origin_kind,invocation_id,attempt_id,action_type,payload_hash,policy_revision,authority_generation,daemon_epoch,fence,state,reconcile_strategy,revision,created_at_ms) VALUES('legacy-action','invocation','legacy-invocation','legacy-attempt','model_dispatch',?1,0,?2,?3,1,'prepared','observe_then_decide',1,?4)",rusqlite::params![hash.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;
        tx.execute("INSERT INTO model_calls(model_call_id,action_id,owner_kind,invocation_id,trigger_event_id,state,revision) VALUES('legacy-call','legacy-action','invocation','legacy-invocation','legacy-event','prepared',1)",[])?;
        tx.execute("INSERT INTO model_call_manifests(manifest_id,model_call_id,manifest_kind,ordinal,parent_manifest_id,context_manifest_id,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,usage_object_hash,result_object_hash,content_object_hash,created_at_ms) VALUES('legacy-request-manifest','legacy-call','request',1,NULL,'legacy-context','provider-exact','model-exact',?1,?1,?1,NULL,NULL,?1,?2),('legacy-result-manifest','legacy-call','result',1,'legacy-request-manifest','legacy-context','provider-exact','reported-model',?1,?1,?1,?1,?1,?1,?2)",rusqlite::params![hash.to_string(),now.as_i64()])?;
        if ambiguous_probe {
            tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,source_kind,created_at_ms) VALUES('legacy-profile','provider-exact','local_only','none_local','user_declared',?1)",[now.as_i64()])?;
            tx.execute("INSERT INTO provider_probes(probe_id,command_receipt_id,principal_id,profile_version_ref,context_hash,phase,state,admission_class,authority_generation,daemon_epoch,fence,revision) VALUES('legacy-probe','legacy-receipt','probe-operator','legacy-profile',?1,'generation_probe','queued','work',?2,?3,0,1)",rusqlite::params![hash.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap()])?;
            tx.execute("INSERT INTO actions(action_id,origin_kind,control_principal_id,action_type,payload_hash,policy_revision,authority_generation,daemon_epoch,fence,state,reconcile_strategy,revision,created_at_ms) VALUES('old-probe-action','control_plane','probe-operator','model_dispatch',?1,0,?2,?3,0,'prepared','observe_then_decide',1,?4)",rusqlite::params![hash.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;
            tx.execute("INSERT INTO model_calls(model_call_id,action_id,owner_kind,provider_probe_id,trigger_event_id,state,revision) VALUES('old-probe-call','old-probe-action','provider_probe','legacy-probe','legacy-event','prepared',1)",[])?;
            tx.execute("INSERT INTO model_call_manifests(manifest_id,model_call_id,manifest_kind,ordinal,context_manifest_id,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,content_object_hash,created_at_ms) VALUES('ambiguous-manifest','old-probe-call','request',1,'legacy-context','provider-exact','model-exact',?1,?1,?1,?1,?2)",rusqlite::params![hash.to_string(),now.as_i64()])?;
        }
        Ok(())
    }).unwrap();
}

struct FailMigration(AtomicBool);
impl unleash_store::CutpointObserver for FailMigration {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), unleash_store::StoreError> {
        if point == unleash_store::StoreCutpoint::C11MigrationInProgress
            && self.0.swap(false, Ordering::AcqRel)
        {
            return Err(unleash_store::StoreError::CutpointInjected {
                cutpoint: "C11".to_owned(),
                message: "fixture V009 rollback".to_owned(),
            });
        }
        Ok(())
    }
}

#[test]
fn t_f012_009_v009_preserves_invocation_manifests_and_rolls_back_interrupted_rebuild() {
    use unleash_store::{BackupRetentionTier, PrepareBackupRequest, StoreError};
    let directory = local_test_dir("f012-v009-migration").unwrap();
    let mut old = StoreConfig::new(directory.path());
    old.max_supported_schema_version = 8;
    let store = UnleashStore::open(old.clone()).unwrap();
    seed_legacy_manifest(&store, false);
    let before = manifests(&store);
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: directory.path().join("migration-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let mut next = StoreConfig::new(directory.path());
    next.cutpoints = Arc::new(FailMigration(AtomicBool::new(true)));
    assert!(matches!(
        UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)),
        Err(StoreError::CutpointInjected { .. })
    ));
    let preserved = UnleashStore::open(old.clone()).unwrap();
    assert_eq!(preserved.daemon_state().unwrap().schema_version, 8);
    assert_eq!(manifests(&preserved), before);
    preserved.shutdown().unwrap();
    let upgraded = UnleashStore::open_with_migration_backup(
        StoreConfig::new(directory.path()),
        Some(backup.backup_id),
    )
    .unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 9);
    assert_eq!(manifests(&upgraded), before);
    let (foreign_keys,variants,triggers):(i64,i64,i64)=upgraded.read_snapshot(|conn,_|Ok((conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check",[],|row|row.get(0))?,conn.query_row("SELECT COUNT(*) FROM model_call_manifests WHERE context_kind='invocation_context' AND provider_probe_context_id IS NULL",[],|row|row.get(0))?,conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name LIKE 'admission_source_model_call_manifests_%'",[],|row|row.get(0))?))).unwrap();
    assert_eq!((foreign_keys, variants, triggers), (0, 2, 3));
    upgraded.shutdown().unwrap();
    assert!(matches!(
        UnleashStore::open(old),
        Err(StoreError::UnsupportedSchemaVersion { found: 9, .. })
    ));
}

#[test]
fn t_f012_009_v009_refuses_ambiguous_probe_history_without_reclassifying_it() {
    use unleash_store::{BackupRetentionTier, PrepareBackupRequest, StoreError};
    let directory = local_test_dir("f012-v009-ambiguous").unwrap();
    let mut old = StoreConfig::new(directory.path());
    old.max_supported_schema_version = 8;
    let store = UnleashStore::open(old.clone()).unwrap();
    seed_legacy_manifest(&store, true);
    let before = manifests(&store);
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: directory.path().join("migration-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    assert!(matches!(
        UnleashStore::open_with_migration_backup(
            StoreConfig::new(directory.path()),
            Some(backup.backup_id)
        ),
        Err(StoreError::InvariantViolation(_))
    ));
    let preserved = UnleashStore::open(old).unwrap();
    assert_eq!(manifests(&preserved), before);
    assert_eq!(preserved.daemon_state().unwrap().schema_version, 8);
}

fn legacy_ownership_links(store: &UnleashStore) -> Vec<Vec<rusqlite::types::Value>> {
    store.read_snapshot(|conn,_| {
        let mut statement=conn.prepare("SELECT m.manifest_id,m.model_call_id,c.invocation_id,i.agent_id,i.session_id,m.context_manifest_id,x.session_id,m.parent_manifest_id FROM model_call_manifests m JOIN model_calls c ON c.model_call_id=m.model_call_id JOIN invocations i ON i.invocation_id=c.invocation_id JOIN context_manifests x ON x.manifest_id=m.context_manifest_id ORDER BY m.manifest_id")?;
        let count=statement.column_count();
        Ok(statement.query_map([],|row|(0..count).map(|index|row.get(index)).collect::<Result<Vec<_>,_>>())?.collect::<Result<Vec<_>,_>>()?)
    }).unwrap()
}

#[test]
fn t_f012_009_v009_refuses_cross_session_invocation_history_atomically() {
    use unleash_store::{BackupRetentionTier, PrepareBackupRequest, StoreError};
    let directory = local_test_dir("f012-v009-cross-session").unwrap();
    let mut old = StoreConfig::new(directory.path());
    old.max_supported_schema_version = 8;
    let store = UnleashStore::open(old.clone()).unwrap();
    seed_legacy_manifest(&store, false);
    let hash = publish_fixture(&store);
    let daemon = store.daemon_state().unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now| {
        tx.execute("INSERT INTO agents(agent_id,level,state,revision) VALUES('other-agent','N','eligible',0)",[])?;
        tx.execute("INSERT INTO sessions(session_id,agent_id,revision,state,created_at_ms) VALUES('other-session','other-agent',0,'open',?1)",[now.as_i64()])?;
        tx.execute("INSERT INTO context_manifests(manifest_id,session_id,base_session_revision,content_hash,schema_version,created_at_ms) VALUES('other-context','other-session',0,?1,2,?2)",rusqlite::params![hash.to_string(),now.as_i64()])?;
        tx.execute("INSERT INTO effective_bundles(bundle_hash,schema_version,role_version_id,requested_model_profile_version_id,allowed_model_policy_hash,initial_context_manifest_id,scope_id,scope_revision,content_object_hash) VALUES('other-bundle',1,'legacy-role','legacy-profile',?1,'other-context','global',0,?1)",[hash.to_string()])?;
        tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES('other-event',?1,'invocation','other-invocation',1,0,'InvocationQueued',1,'probe-operator','probe-operator',?2,?2,'{}','full',?3,'restricted','command','legacy-receipt','legacy-receipt','legacy-command')",rusqlite::params![daemon.stream_id.to_string(),now.as_i64(),hash.to_string()])?;
        tx.execute("INSERT INTO invocations(invocation_id,agent_id,session_id,trigger_event_id,effective_bundle_hash,context_manifest_id,state,admission_class,revision,created_at_ms) VALUES('other-invocation','other-agent','other-session','other-event','other-bundle','other-context','queued','work',1,?1)",[now.as_i64()])?;
        // V008 accepts both FKs even though call A now refers to session B.
        // Leave the request valid and change only its child result manifest.
        tx.execute("UPDATE model_call_manifests SET context_manifest_id='other-context' WHERE manifest_id='legacy-result-manifest'",[])?;
        Ok(())
    }).unwrap();
    let before = manifests(&store);
    let links = legacy_ownership_links(&store);
    let (invocations, sessions, foreign_keys): (i64, i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM invocations", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })?,
            ))
        })
        .unwrap();
    assert_eq!((invocations, sessions, foreign_keys), (2, 2, 0));
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: directory.path().join("migration-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    assert!(
        matches!(
            UnleashStore::open_with_migration_backup(
                StoreConfig::new(directory.path()),
                Some(backup.backup_id)
            ),
            Err(StoreError::InvariantViolation(_))
        ),
        "V009 must reject the foreign-session result manifest before copying legacy rows"
    );
    let preserved = UnleashStore::open(old).unwrap();
    assert_eq!(preserved.daemon_state().unwrap().schema_version, 8);
    assert_eq!(manifests(&preserved), before);
    assert_eq!(legacy_ownership_links(&preserved), links);
    let (migration,objects,foreign_keys):(i64,i64,i64)=preserved.read_snapshot(|conn,_|Ok((conn.query_row("SELECT COUNT(*) FROM schema_migrations WHERE version=9",[],|row|row.get(0))?,conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('provider_probe_contexts','model_call_manifests_v009','model_call_context_owner')",[],|row|row.get(0))?,conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check",[],|row|row.get(0))?))).unwrap();
    assert_eq!((migration, objects, foreign_keys), (0, 0, 0));
}

fn publish_fixture(store: &UnleashStore) -> Sha256Digest {
    let bytes = br#"{"context_mode":"explicit_unleash_history","fixture":"synthetic-probe-v1"}"#;
    let hash = Sha256Digest::from_bytes(bytes);
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "probe-fixture".to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&handle, bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id,
            object_hash: hash,
            object_generation: handle.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "probe-fixture".to_owned(),
            source_field: "context".to_owned(),
        })
        .unwrap();
    hash
}

#[test]
fn t_f012_014_generation_manifest_accepts_a_probe_without_agent_session() {
    let directory = local_test_dir("f012-probe-context").unwrap();
    let store = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    store
        .register_principal("probe-operator", "user", "probe-operator")
        .unwrap();
    let hash = publish_fixture(&store);
    let daemon = store.daemon_state().unwrap();
    let result=store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now:TimestampMs|{
        tx.execute("INSERT INTO command_receipts(actor_id,command_id,authority_generation,receipt_id,request_id,command_type,command_digest,aggregate_id,state,response_state,response_json,response_retained_until_ms,committed_at_ms) VALUES('probe-operator','fixture-command',?1,'fixture-receipt','fixture-request','TestProviderProfile',?2,'fixture-probe','accepted','full','{}',?3,?4)",rusqlite::params![daemon.authority_generation.to_string(),hash.to_string(),now.as_i64()+86_400_000,now.as_i64()])?;
        tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES('fixture-event',?1,'provider_probe','fixture-probe',1,0,'ProviderProbePrepared',1,'probe-operator','probe-operator',?2,?2,'{}','full',?3,'restricted','command','fixture-receipt','fixture-receipt','fixture-command')",rusqlite::params![daemon.stream_id.to_string(),now.as_i64(),hash.to_string()])?;
        tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,source_kind,created_at_ms) VALUES('fixture-profile@1','fixture-provider','local_only','none_local','user_declared',?1)",[now.as_i64()])?;
        tx.execute("INSERT INTO provider_probes(probe_id,command_receipt_id,principal_id,profile_version_ref,context_hash,phase,state,admission_class,authority_generation,daemon_epoch,fence,revision,queued_at_ms) VALUES('fixture-probe','fixture-receipt','probe-operator','fixture-profile@1',?1,'generation_probe','queued','work',?2,?3,0,1,?4)",rusqlite::params![hash.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;
        tx.execute("INSERT INTO actions(action_id,origin_kind,control_principal_id,action_type,payload_hash,policy_revision,authority_generation,daemon_epoch,fence,state,reconcile_strategy,revision,created_at_ms) VALUES('fixture-action','control_plane','probe-operator','model_dispatch',?1,0,?2,?3,0,'prepared','observe_then_decide',1,?4)",rusqlite::params![hash.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;
        tx.execute("INSERT INTO model_calls(model_call_id,action_id,owner_kind,provider_probe_id,trigger_event_id,state,revision) VALUES('fixture-call','fixture-action','provider_probe','fixture-probe','fixture-event','prepared',1)",[])?;
        tx.execute("INSERT INTO provider_probe_contexts(probe_id,receipt_id,actor_id,principal_id,profile_version_ref,authority_generation,daemon_epoch,action_id,model_call_id,manifest_id,preparation_digest,context_digest,context_json,created_at_ms) VALUES('fixture-probe','fixture-receipt','probe-operator','probe-operator','fixture-profile@1',?1,?2,'fixture-action','fixture-call','fixture-manifest',?3,?3,'{}',?4)",rusqlite::params![daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),hash.to_string(),now.as_i64()])?;
        // The NULL is valid only together with the explicit probe variant.
        tx.execute("INSERT INTO model_call_manifests(manifest_id,model_call_id,manifest_kind,ordinal,context_manifest_id,context_kind,provider_probe_context_id,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,content_object_hash,created_at_ms) VALUES('fixture-manifest','fixture-call','request',1,NULL,'provider_probe_context','fixture-probe','fixture-provider','fixture-model',?1,?1,?1,?1,?2)",rusqlite::params![hash.to_string(),now.as_i64()])?;
        Ok(())
    });
    assert!(
        result.is_ok(),
        "provider_probe_context must not require an invocation session: {result:?}"
    );
    let counts: (i64, i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })?,
            ))
        })
        .unwrap();
    assert_eq!(counts, (0, 0, 0));
    let naked=store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_|{
        tx.execute("INSERT INTO model_call_manifests(manifest_id,model_call_id,manifest_kind,ordinal,context_manifest_id,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,content_object_hash,created_at_ms) SELECT 'naked',model_call_id,'request',2,NULL,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,content_object_hash,created_at_ms FROM model_call_manifests WHERE manifest_id='fixture-manifest'",[])?;Ok(())});
    assert!(naked.is_err());
}
