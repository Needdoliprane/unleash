//! Migrations versionnées `STRICT`, sommes de contrôle SHA-256 et vérifications d'ouverture (`REQ-PERSIST-005`).

use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unleash_domain::{BackupId, TimestampMs};

use crate::config::{CutpointObserver, StoreCutpoint};
use crate::error::StoreError;

/// Version courante maximale du schéma gérée par cette version de `unleash-store`.
pub const CURRENT_SCHEMA_VERSION: i64 = 20;

/// État persisté d'une migration dans `schema_migrations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationState {
    /// Migration en cours (doit bloquer une ouverture ordinaire si retrouvée au démarrage).
    Applying,
    /// Migration terminée et vérifiée.
    Applied,
    /// Migration en échec.
    Failed,
}

impl MigrationState {
    fn from_sql(s: &str) -> Option<Self> {
        match s {
            "applying" => Some(Self::Applying),
            "applied" => Some(Self::Applied),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Enregistrement d'une migration appliquée ou interrompue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRecord {
    /// Numéro de version croissant (>= 1).
    pub version: i64,
    /// Nom symbolique de la migration.
    pub name: String,
    /// Somme de contrôle SHA-256 (`sha256:<hex>`) du DDL exact.
    pub checksum_sha256: String,
    /// État de la migration.
    pub state: MigrationState,
    /// Sauvegarde préalable vérifiée obligatoire lors d'une montée de version.
    pub pre_migration_backup_id: Option<BackupId>,
    /// Horodatage de début.
    pub started_at_ms: i64,
    /// Horodatage de fin si terminée.
    pub completed_at_ms: Option<i64>,
}

/// Résultat d'inspection ou d'exécution des migrations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Version de schéma atteinte après migration.
    pub current_version: i64,
    /// Migrations exécutées lors de cette passe.
    pub newly_applied: Vec<i64>,
}

struct MigrationDef {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[MigrationDef] = &[
    MigrationDef {
        version: 1,
        name: "v001_core_authority_events_objects_execution",
        sql: V001_SQL,
    },
    MigrationDef {
        version: 2,
        name: "v002_workflow_automation_registries_and_probes",
        sql: V002_SQL,
    },
    MigrationDef {
        version: 3,
        name: "v003_m1_projects_policies_secrets_recovery_retention",
        sql: V003_SQL,
    },
    MigrationDef {
        version: 4,
        name: "v004_recovery_scope_dispositions",
        sql: V004_SQL,
    },
    MigrationDef {
        version: 5,
        name: "v005_durable_broker_phases",
        sql: V005_SQL,
    },
    MigrationDef {
        version: 6,
        name: "v006_reserved_backup_requests",
        sql: V006_SQL,
    },
    MigrationDef {
        version: 7,
        name: "v007_pre_dispatch_backup_block_reason",
        sql: V007_SQL,
    },
    MigrationDef {
        version: 8,
        name: "v008_transactional_execution_admission",
        sql: V008_SQL,
    },
    MigrationDef {
        version: 9,
        name: "v009_provider_probe_context_union",
        sql: V009_SQL,
    },
    MigrationDef {
        version: 10,
        name: "v010_terminal_allocation_preparation",
        sql: V010_SQL,
    },
    MigrationDef {
        version: 11,
        name: "v011_immutable_configured_provider_profiles",
        sql: V011_SQL,
    },
    MigrationDef {
        version: 12,
        name: "v012_terminal_control_transfer_intentions",
        sql: V012_SQL,
    },
    MigrationDef {
        version: 13,
        name: "v013_policy_provider_probe_mandates",
        sql: V013_SQL,
    },
    MigrationDef {
        version: 14,
        name: "v014_terminal_input_intentions",
        sql: V014_SQL,
    },
    MigrationDef {
        version: 15,
        name: "v015_provider_probe_transport_contexts",
        sql: include_str!("migrations/v015_provider_probe_transport_contexts.sql"),
    },
    MigrationDef {
        version: 16,
        name: "v016_terminal_renewal_intents",
        sql: include_str!("migrations/v016_terminal_renewal_intents.sql"),
    },
    MigrationDef {
        version: 17,
        name: "v017_terminal_control_refusals",
        sql: include_str!("migrations/v017_terminal_control_refusals.sql"),
    },
    MigrationDef {
        version: 18,
        name: "v018_terminal_release_intents",
        sql: include_str!("migrations/v018_terminal_release_intents.sql"),
    },
    MigrationDef {
        version: crate::provider_service_launch::SCHEMA_VERSION,
        name: "v019_provider_service_launches",
        sql: include_str!("migrations/v019_provider_service_launches.sql"),
    },
    MigrationDef {
        version: 20,
        name: "v020_workflow_activation_budget",
        sql: include_str!("migrations/v020_workflow_activation_budget.sql"),
    },
];

const V014_SQL: &str = r"
CREATE TABLE terminal_input_intents (
 action_id TEXT PRIMARY KEY,
 terminal_id TEXT NOT NULL REFERENCES terminal_sessions(terminal_id),
 receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 command_id TEXT NOT NULL,
 actor_id TEXT NOT NULL,
 principal_id TEXT NOT NULL REFERENCES principals(principal_id),
 authority_generation TEXT NOT NULL,
 daemon_epoch INTEGER NOT NULL CHECK(daemon_epoch>=0),
 lease_id TEXT NOT NULL REFERENCES resource_leases(lease_id),
 resource_fence INTEGER NOT NULL CHECK(resource_fence BETWEEN 1 AND 9007199254740991),
 input_seq INTEGER NOT NULL CHECK(input_seq BETWEEN 1 AND 9007199254740991),
 payload_digest TEXT NOT NULL,
 payload_byte_length INTEGER NOT NULL CHECK(payload_byte_length BETWEEN 1 AND 65536),
 context_digest TEXT NOT NULL,
 context_json TEXT NOT NULL CHECK(json_valid(context_json)),
 state TEXT NOT NULL CHECK(state IN ('prepared','write_resolved','observed','outcome_unknown','not_dispatched','disposed')),
 revision INTEGER NOT NULL CHECK(revision>=1),
 dispatch_id TEXT UNIQUE REFERENCES action_dispatches(dispatch_id),
 observation_object_hash TEXT REFERENCES object_records(object_hash),
 renewal TEXT CHECK(renewal IS NULL OR renewal IN ('renewed','lease_expired','controller_changed','authority_changed','clock_uncertain','authority_blocked')),
 renewed_lease_expires_at INTEGER,
 disposition_evaluation_id TEXT,
 CHECK(COALESCE(json_extract(context_json,'$.schema_version')=1
  AND json_extract(context_json,'$.action_id')=action_id
  AND json_extract(context_json,'$.initial_action_id')!=action_id
  AND json_extract(context_json,'$.terminal_id')=terminal_id
  AND json_extract(context_json,'$.receipt_id')=receipt_id
  AND json_extract(context_json,'$.command_id')=command_id
  AND json_extract(context_json,'$.actor_id')=actor_id
  AND json_extract(context_json,'$.principal_id')=principal_id
  AND json_extract(context_json,'$.authority_generation')=authority_generation
  AND json_extract(context_json,'$.daemon_epoch')=daemon_epoch
  AND json_extract(context_json,'$.lease_id')=lease_id
  AND json_extract(context_json,'$.resource_fence')=resource_fence
  AND json_extract(context_json,'$.input_seq')=input_seq
  AND json_extract(context_json,'$.payload_digest')=payload_digest
  AND json_extract(context_json,'$.payload_byte_length')=payload_byte_length,0)),
 CHECK(json_type(context_json,'$.payload_bytes') IS NULL),
 CHECK((state='prepared' AND dispatch_id IS NULL AND observation_object_hash IS NULL AND disposition_evaluation_id IS NULL)
  OR(state IN ('write_resolved','outcome_unknown') AND dispatch_id IS NOT NULL AND observation_object_hash IS NULL AND disposition_evaluation_id IS NULL)
  OR(state='observed' AND dispatch_id IS NOT NULL AND observation_object_hash IS NOT NULL AND renewal IS NOT NULL AND disposition_evaluation_id IS NULL)
  OR(state='not_dispatched' AND dispatch_id IS NULL AND observation_object_hash IS NULL AND disposition_evaluation_id IS NOT NULL)
  OR(state='disposed' AND observation_object_hash IS NOT NULL)),
 CHECK((state='observed' AND renewal IS NOT NULL) OR (state!='observed' AND renewal IS NULL AND renewed_lease_expires_at IS NULL)),
 CHECK((renewal IS 'renewed' AND renewed_lease_expires_at IS NOT NULL) OR (renewal IS NOT 'renewed' AND renewed_lease_expires_at IS NULL))
) STRICT;
CREATE UNIQUE INDEX terminal_input_one_pending ON terminal_input_intents(terminal_id)
 WHERE state IN ('prepared','write_resolved','outcome_unknown');
CREATE UNIQUE INDEX terminal_input_sequence_reserved ON terminal_input_intents(terminal_id,input_seq)
 WHERE state!='not_dispatched';
CREATE TRIGGER terminal_input_initial BEFORE INSERT ON terminal_input_intents
 WHEN NEW.state!='prepared' OR NEW.revision!=1
 BEGIN SELECT RAISE(ABORT,'terminal input must start prepared'); END;
CREATE TRIGGER terminal_input_immutable BEFORE UPDATE ON terminal_input_intents
 WHEN NEW.action_id IS NOT OLD.action_id OR NEW.terminal_id IS NOT OLD.terminal_id
 OR NEW.receipt_id IS NOT OLD.receipt_id OR NEW.command_id IS NOT OLD.command_id
 OR NEW.actor_id IS NOT OLD.actor_id OR NEW.principal_id IS NOT OLD.principal_id
 OR NEW.authority_generation IS NOT OLD.authority_generation OR NEW.daemon_epoch IS NOT OLD.daemon_epoch
 OR NEW.lease_id IS NOT OLD.lease_id OR NEW.resource_fence IS NOT OLD.resource_fence
 OR NEW.input_seq IS NOT OLD.input_seq OR NEW.payload_digest IS NOT OLD.payload_digest
 OR NEW.payload_byte_length IS NOT OLD.payload_byte_length
 OR NEW.context_digest IS NOT OLD.context_digest OR NEW.context_json IS NOT OLD.context_json
 BEGIN SELECT RAISE(ABORT,'terminal input intention immutable'); END;
CREATE TRIGGER terminal_input_transition BEFORE UPDATE ON terminal_input_intents
 WHEN NOT(NEW.revision=OLD.revision+1 AND (
  (OLD.state='prepared' AND NEW.state='write_resolved' AND NEW.dispatch_id IS NOT NULL)
  OR(OLD.state='prepared' AND NEW.state='not_dispatched' AND NEW.dispatch_id IS NULL)
  OR(OLD.state='write_resolved' AND NEW.state IN ('observed','outcome_unknown') AND NEW.dispatch_id IS OLD.dispatch_id)
  OR(OLD.state='outcome_unknown' AND NEW.state IN ('observed','disposed') AND NEW.dispatch_id IS OLD.dispatch_id)))
 BEGIN SELECT RAISE(ABORT,'terminal input transition invalid'); END;
CREATE TRIGGER terminal_input_durable BEFORE DELETE ON terminal_input_intents
 BEGIN SELECT RAISE(ABORT,'terminal input intention durable'); END;
";

const V013_SQL: &str = r"
CREATE TABLE provider_probe_mandates (
 mandate_ref TEXT PRIMARY KEY REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 scope_kind TEXT NOT NULL CHECK(scope_kind IN ('global','project')),
 scope_id TEXT NOT NULL CHECK(length(scope_id)>0 AND (scope_kind!='global' OR scope_id='global')),
 authority_generation TEXT NOT NULL,
 principal_id TEXT NOT NULL REFERENCES principals(principal_id),
 profile_version_ref TEXT NOT NULL REFERENCES provider_profiles(profile_version_ref),
 profile_id TEXT NOT NULL,
 profile_version INTEGER NOT NULL CHECK(profile_version>=1),
 configuration_digest TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('connect','inventory','generation_probe')),
 expires_at_ms INTEGER NOT NULL,
 issued_by_actor TEXT NOT NULL,
 issued_at_ms INTEGER NOT NULL CHECK(expires_at_ms>issued_at_ms),
 mandate_digest TEXT NOT NULL,
 grant_json TEXT NOT NULL CHECK(json_valid(grant_json)),
 revocation_receipt_id TEXT UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 revocation_digest TEXT,
 revocation_json TEXT CHECK(revocation_json IS NULL OR json_valid(revocation_json)),
 CHECK(revocation_receipt_id IS NULL OR revocation_receipt_id!=mandate_ref),
 CHECK(COALESCE(json_extract(grant_json,'$.schema_version')=1
   AND json_extract(grant_json,'$.mandate_ref')=mandate_ref
   AND json_extract(grant_json,'$.scope_kind')=scope_kind
   AND json_extract(grant_json,'$.scope_id')=scope_id
   AND json_extract(grant_json,'$.authority_generation')=authority_generation
   AND json_extract(grant_json,'$.permission.principal_id')=principal_id
   AND json_extract(grant_json,'$.profile_version_ref')=profile_version_ref
   AND json_extract(grant_json,'$.permission.profile_id')=profile_id
   AND json_extract(grant_json,'$.permission.profile_version')=profile_version
   AND json_extract(grant_json,'$.configuration_digest')=configuration_digest
   AND json_extract(grant_json,'$.permission.phase')=phase
   AND json_extract(grant_json,'$.permission.expires_at_ms')=expires_at_ms
   AND json_extract(grant_json,'$.issued_by_actor')=issued_by_actor
   AND json_extract(grant_json,'$.issued_at_ms')=issued_at_ms,0)),
 CHECK((revocation_receipt_id IS NULL AND revocation_digest IS NULL AND revocation_json IS NULL)
   OR (revocation_receipt_id IS NOT NULL AND revocation_digest IS NOT NULL AND revocation_json IS NOT NULL
     AND COALESCE(json_extract(revocation_json,'$.schema_version')=1
       AND json_extract(revocation_json,'$.mandate_ref')=mandate_ref
       AND json_extract(revocation_json,'$.mandate_digest')=mandate_digest
       AND json_extract(revocation_json,'$.revocation_receipt_id')=revocation_receipt_id
       AND json_extract(revocation_json,'$.authority_generation')=authority_generation
       AND json_extract(revocation_json,'$.scope_kind')=scope_kind
       AND json_extract(revocation_json,'$.scope_id')=scope_id,0))),
 FOREIGN KEY(scope_kind,scope_id) REFERENCES security_policies(scope_kind,scope_id)
) STRICT;
CREATE INDEX provider_probe_mandates_scope ON provider_probe_mandates(scope_kind,scope_id);
CREATE TRIGGER provider_probe_mandate_initial BEFORE INSERT ON provider_probe_mandates
WHEN NEW.revocation_receipt_id IS NOT NULL
 OR EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=NEW.mandate_ref)
 OR EXISTS(SELECT 1 FROM provider_probe_mandates WHERE revocation_receipt_id=NEW.mandate_ref)
BEGIN SELECT RAISE(ABORT,'provider probe mandate must precede its new receipt'); END;
CREATE TRIGGER provider_probe_mandate_profile_binding BEFORE INSERT ON provider_probe_mandates
WHEN NOT EXISTS(SELECT 1 FROM provider_profiles p WHERE p.profile_version_ref=NEW.profile_version_ref
 AND p.profile_id=NEW.profile_id AND p.profile_version=NEW.profile_version
 AND p.config_digest=NEW.configuration_digest
 AND ((NEW.scope_kind='global' AND p.scope_kind='user' AND p.scope_owner_key='default')
   OR (NEW.scope_kind='project' AND p.scope_kind='project' AND p.scope_owner_key=NEW.scope_id)))
BEGIN SELECT RAISE(ABORT,'provider probe mandate profile binding invalid'); END;
CREATE TRIGGER provider_probe_mandate_immutable BEFORE UPDATE ON provider_probe_mandates
WHEN NEW.mandate_ref IS NOT OLD.mandate_ref OR NEW.scope_kind IS NOT OLD.scope_kind
 OR NEW.scope_id IS NOT OLD.scope_id OR NEW.authority_generation IS NOT OLD.authority_generation
 OR NEW.principal_id IS NOT OLD.principal_id OR NEW.profile_version_ref IS NOT OLD.profile_version_ref
 OR NEW.profile_id IS NOT OLD.profile_id OR NEW.profile_version IS NOT OLD.profile_version
 OR NEW.configuration_digest IS NOT OLD.configuration_digest OR NEW.phase IS NOT OLD.phase
 OR NEW.expires_at_ms IS NOT OLD.expires_at_ms OR NEW.issued_by_actor IS NOT OLD.issued_by_actor
 OR NEW.issued_at_ms IS NOT OLD.issued_at_ms OR NEW.mandate_digest IS NOT OLD.mandate_digest
 OR NEW.grant_json IS NOT OLD.grant_json
BEGIN SELECT RAISE(ABORT,'provider probe mandate immutable'); END;
CREATE TRIGGER provider_probe_mandate_revocation_once
BEFORE UPDATE OF revocation_receipt_id,revocation_digest,revocation_json ON provider_probe_mandates
WHEN NOT(OLD.revocation_receipt_id IS NULL AND NEW.revocation_receipt_id IS NOT NULL
 AND NEW.revocation_digest IS NOT NULL AND NEW.revocation_json IS NOT NULL)
 OR EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=NEW.revocation_receipt_id)
 OR EXISTS(SELECT 1 FROM provider_probe_mandates WHERE mandate_ref=NEW.revocation_receipt_id)
BEGIN SELECT RAISE(ABORT,'provider probe mandate revocation immutable'); END;
CREATE TRIGGER provider_probe_mandate_durable BEFORE DELETE ON provider_probe_mandates
BEGIN SELECT RAISE(ABORT,'provider probe mandate durable'); END;
CREATE TRIGGER provider_probe_mandate_receipt_binding BEFORE INSERT ON command_receipts
WHEN EXISTS(SELECT 1 FROM provider_probe_mandates m WHERE m.mandate_ref=NEW.receipt_id
 AND NOT COALESCE(NEW.command_type='SetPolicy' AND NEW.actor_id=m.issued_by_actor
   AND NEW.command_id=json_extract(m.grant_json,'$.command_id')
   AND NEW.authority_generation=m.authority_generation AND NEW.state IN ('accepted','completed')
   AND NEW.aggregate_id=CASE WHEN m.scope_kind='global' THEN m.authority_generation ELSE m.scope_id END,0))
 OR EXISTS(SELECT 1 FROM provider_probe_mandates m WHERE m.revocation_receipt_id=NEW.receipt_id
 AND NOT COALESCE(NEW.command_type='SetPolicy' AND NEW.actor_id=json_extract(m.revocation_json,'$.revoked_by_actor')
   AND NEW.command_id=json_extract(m.revocation_json,'$.command_id')
   AND NEW.authority_generation=m.authority_generation AND NEW.state IN ('accepted','completed')
   AND NEW.aggregate_id=CASE WHEN m.scope_kind='global' THEN m.authority_generation ELSE m.scope_id END,0))
BEGIN SELECT RAISE(ABORT,'provider probe mandate receipt binding invalid'); END;
CREATE TRIGGER admission_source_mandate_insert AFTER INSERT ON provider_probe_mandates
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_mandate_update AFTER UPDATE ON provider_probe_mandates
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_mandate_delete AFTER DELETE ON provider_probe_mandates
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
";

const V012_SQL: &str = r"
ALTER TABLE terminal_sessions ADD COLUMN control_revision INTEGER NOT NULL DEFAULT 0 CHECK(control_revision>=0);
CREATE TRIGGER terminal_control_revision_monotone BEFORE UPDATE OF control_revision ON terminal_sessions
WHEN NEW.control_revision!=OLD.control_revision AND NEW.control_revision!=OLD.control_revision+1
BEGIN SELECT RAISE(ABORT,'terminal control revision must advance once'); END;
CREATE TABLE terminal_control_intents (
 action_id TEXT PRIMARY KEY,
 terminal_id TEXT NOT NULL REFERENCES terminal_sessions(terminal_id),
 receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 command_id TEXT NOT NULL,
 actor_id TEXT NOT NULL,
 principal_id TEXT NOT NULL REFERENCES principals(principal_id),
 authority_generation TEXT NOT NULL,
 daemon_epoch INTEGER NOT NULL CHECK(daemon_epoch>=0),
 context_digest TEXT NOT NULL,
 context_json TEXT NOT NULL CHECK(json_valid(context_json)),
 previous_lease_id TEXT NOT NULL REFERENCES resource_leases(lease_id),
 next_lease_id TEXT NOT NULL UNIQUE CHECK(next_lease_id!=previous_lease_id),
 next_fence INTEGER NOT NULL CHECK(next_fence>0),
 state TEXT NOT NULL CHECK(state IN ('prepared','handshake_resolved','observed','outcome_unknown','disposed')),
 revision INTEGER NOT NULL CHECK(revision>=1),
 dispatch_id TEXT UNIQUE REFERENCES action_dispatches(dispatch_id),
 observation_object_hash TEXT REFERENCES object_records(object_hash),
 CHECK(COALESCE(json_extract(context_json,'$.schema_version')=1
   AND json_extract(context_json,'$.action_id')=action_id
   AND json_extract(context_json,'$.initial_action_id')!=action_id
   AND json_extract(context_json,'$.terminal_id')=terminal_id
   AND json_extract(context_json,'$.receipt_id')=receipt_id
   AND json_extract(context_json,'$.command_id')=command_id
   AND json_extract(context_json,'$.actor_id')=actor_id
   AND json_extract(context_json,'$.principal_id')=principal_id
   AND json_extract(context_json,'$.authority_generation')=authority_generation
   AND json_extract(context_json,'$.daemon_epoch')=daemon_epoch
   AND json_extract(context_json,'$.previous_lease_id')=previous_lease_id
   AND json_extract(context_json,'$.next_lease_id')=next_lease_id
   AND json_extract(context_json,'$.next_fence')=next_fence
   AND json_extract(context_json,'$.previous_fence')<next_fence,0)),
 CHECK((state='prepared' AND dispatch_id IS NULL AND observation_object_hash IS NULL)
    OR (state IN ('handshake_resolved','outcome_unknown') AND dispatch_id IS NOT NULL AND observation_object_hash IS NULL)
    OR (state='observed' AND dispatch_id IS NOT NULL AND observation_object_hash IS NOT NULL)
    OR (state='disposed' AND observation_object_hash IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX terminal_control_one_pending ON terminal_control_intents(terminal_id)
WHERE state IN ('prepared','handshake_resolved','outcome_unknown');
CREATE TRIGGER terminal_control_initial BEFORE INSERT ON terminal_control_intents
WHEN NEW.state!='prepared' OR NEW.revision!=1
BEGIN SELECT RAISE(ABORT,'terminal control must start prepared'); END;
CREATE TRIGGER terminal_control_immutable BEFORE UPDATE ON terminal_control_intents
WHEN NEW.action_id IS NOT OLD.action_id OR NEW.terminal_id IS NOT OLD.terminal_id
 OR NEW.receipt_id IS NOT OLD.receipt_id OR NEW.command_id IS NOT OLD.command_id
 OR NEW.actor_id IS NOT OLD.actor_id OR NEW.principal_id IS NOT OLD.principal_id
 OR NEW.authority_generation IS NOT OLD.authority_generation OR NEW.daemon_epoch IS NOT OLD.daemon_epoch
 OR NEW.context_digest IS NOT OLD.context_digest OR NEW.context_json IS NOT OLD.context_json
 OR NEW.previous_lease_id IS NOT OLD.previous_lease_id OR NEW.next_lease_id IS NOT OLD.next_lease_id
 OR NEW.next_fence IS NOT OLD.next_fence
BEGIN SELECT RAISE(ABORT,'terminal control intention immutable'); END;
CREATE TRIGGER terminal_control_transition BEFORE UPDATE ON terminal_control_intents
WHEN NOT(NEW.revision=OLD.revision+1 AND (
 (OLD.state='prepared' AND NEW.state='handshake_resolved' AND NEW.dispatch_id IS NOT NULL)
 OR (OLD.state='handshake_resolved' AND NEW.state IN ('observed','outcome_unknown') AND NEW.dispatch_id IS OLD.dispatch_id)
 OR (OLD.state='outcome_unknown' AND NEW.state='observed' AND NEW.dispatch_id IS OLD.dispatch_id)
 OR (OLD.state IN ('prepared','handshake_resolved','outcome_unknown') AND NEW.state='disposed' AND NEW.dispatch_id IS OLD.dispatch_id)))
BEGIN SELECT RAISE(ABORT,'terminal control transition invalid'); END;
CREATE TRIGGER terminal_control_durable BEFORE DELETE ON terminal_control_intents
BEGIN SELECT RAISE(ABORT,'terminal control intention durable'); END;
";

const V011_SQL: &str = r"
ALTER TABLE provider_profiles ADD COLUMN profile_id TEXT;
ALTER TABLE provider_profiles ADD COLUMN profile_version INTEGER CHECK(profile_version IS NULL OR profile_version>=1);
ALTER TABLE provider_profiles ADD COLUMN config_object_hash TEXT REFERENCES object_records(object_hash);
ALTER TABLE provider_profiles ADD COLUMN config_digest TEXT;
ALTER TABLE provider_profiles ADD COLUMN config_json TEXT;
ALTER TABLE provider_profiles ADD COLUMN preparation_digest TEXT;
ALTER TABLE provider_profiles ADD COLUMN scope_kind TEXT CHECK(scope_kind IS NULL OR scope_kind IN ('user','project'));
ALTER TABLE provider_profiles ADD COLUMN scope_owner_key TEXT;
ALTER TABLE provider_profiles ADD COLUMN principal_id TEXT REFERENCES principals(principal_id);
ALTER TABLE provider_profiles ADD COLUMN actor_id TEXT;
ALTER TABLE provider_profiles ADD COLUMN receipt_id TEXT REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE provider_profiles ADD COLUMN catalog_id TEXT;
ALTER TABLE provider_profiles ADD COLUMN catalog_version TEXT;
CREATE UNIQUE INDEX configured_provider_profile_version ON provider_profiles(profile_id,profile_version) WHERE profile_id IS NOT NULL;
CREATE UNIQUE INDEX configured_provider_profile_receipt ON provider_profiles(receipt_id) WHERE receipt_id IS NOT NULL;
CREATE TRIGGER configured_provider_profile_complete BEFORE INSERT ON provider_profiles
WHEN NOT((NEW.profile_id IS NULL AND NEW.profile_version IS NULL AND NEW.config_object_hash IS NULL AND NEW.config_digest IS NULL AND NEW.config_json IS NULL AND NEW.preparation_digest IS NULL AND NEW.scope_kind IS NULL AND NEW.scope_owner_key IS NULL AND NEW.principal_id IS NULL AND NEW.actor_id IS NULL AND NEW.receipt_id IS NULL AND NEW.catalog_id IS NULL AND NEW.catalog_version IS NULL)
 OR (NEW.profile_id IS NOT NULL AND NEW.profile_version IS NOT NULL AND NEW.config_object_hash IS NOT NULL AND NEW.config_digest IS NOT NULL AND NEW.config_json IS NOT NULL AND NEW.preparation_digest IS NOT NULL AND NEW.scope_kind IS NOT NULL AND NEW.scope_owner_key IS NOT NULL AND length(NEW.scope_owner_key)>0 AND NEW.principal_id IS NOT NULL AND NEW.actor_id IS NOT NULL AND NEW.receipt_id IS NOT NULL AND ((NEW.catalog_id IS NULL AND NEW.catalog_version IS NULL) OR (NEW.catalog_id IS NOT NULL AND NEW.catalog_version IS NOT NULL)) AND (NEW.scope_kind!='user' OR NEW.scope_owner_key='default')))
BEGIN SELECT RAISE(ABORT,'configured provider profile metadata incomplete'); END;
CREATE TRIGGER configured_provider_profile_sequence BEFORE INSERT ON provider_profiles
WHEN NEW.profile_id IS NOT NULL AND (NEW.profile_version!=(SELECT COALESCE(MAX(profile_version),0)+1 FROM provider_profiles WHERE profile_id=NEW.profile_id)
 OR EXISTS(SELECT 1 FROM provider_profiles WHERE profile_id=NEW.profile_id AND (scope_kind!=NEW.scope_kind OR scope_owner_key!=NEW.scope_owner_key)))
BEGIN SELECT RAISE(ABORT,'configured provider profile version or scope invalid'); END;
CREATE TRIGGER configured_provider_profile_immutable BEFORE UPDATE ON provider_profiles
WHEN OLD.profile_id IS NOT NULL OR NEW.profile_id IS NOT NULL OR NEW.profile_version IS NOT NULL OR NEW.config_object_hash IS NOT NULL OR NEW.config_digest IS NOT NULL OR NEW.config_json IS NOT NULL OR NEW.preparation_digest IS NOT NULL OR NEW.scope_kind IS NOT NULL OR NEW.scope_owner_key IS NOT NULL OR NEW.principal_id IS NOT NULL OR NEW.actor_id IS NOT NULL OR NEW.receipt_id IS NOT NULL OR NEW.catalog_id IS NOT NULL OR NEW.catalog_version IS NOT NULL
BEGIN SELECT RAISE(ABORT,'configured provider profile immutable; legacy cannot be promoted'); END;
CREATE TRIGGER configured_provider_profile_no_delete BEFORE DELETE ON provider_profiles WHEN OLD.profile_id IS NOT NULL
BEGIN SELECT RAISE(ABORT,'configured provider profile immutable'); END;
CREATE TRIGGER configured_provider_catalog_immutable BEFORE UPDATE ON model_catalog_snapshots
WHEN EXISTS(SELECT 1 FROM provider_profiles WHERE catalog_id=OLD.catalog_id AND catalog_version=OLD.catalog_version)
BEGIN SELECT RAISE(ABORT,'configured provider catalog immutable'); END;
CREATE TRIGGER configured_provider_catalog_no_delete BEFORE DELETE ON model_catalog_snapshots
WHEN EXISTS(SELECT 1 FROM provider_profiles WHERE catalog_id=OLD.catalog_id AND catalog_version=OLD.catalog_version)
BEGIN SELECT RAISE(ABORT,'configured provider catalog referenced'); END;
";

const V010_SQL: &str = r"
CREATE TABLE terminal_allocations (
 terminal_id TEXT PRIMARY KEY,
 receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 action_id TEXT NOT NULL UNIQUE,
 process_id TEXT NOT NULL UNIQUE,
 process_run_uuid TEXT NOT NULL UNIQUE,
 principal_id TEXT NOT NULL REFERENCES principals(principal_id),
 mission_id TEXT NOT NULL REFERENCES missions(mission_id),
 project_id TEXT NOT NULL REFERENCES projects(project_id),
 purpose_key TEXT NOT NULL,
 lease_id TEXT NOT NULL UNIQUE REFERENCES resource_leases(lease_id),
 preparation_digest TEXT NOT NULL,
 context_digest TEXT NOT NULL UNIQUE,
 context_json TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('allocated','launch_resolved','retired')),
 revision INTEGER NOT NULL CHECK(revision>=1),
 dispatch_id TEXT UNIQUE REFERENCES action_dispatches(dispatch_id),
 resolved_at_ms INTEGER,
 retirement_evidence_id TEXT REFERENCES evidence(evidence_id),
 retired_at_ms INTEGER,
 CHECK((state='allocated' AND revision=1 AND dispatch_id IS NULL AND resolved_at_ms IS NULL AND retirement_evidence_id IS NULL AND retired_at_ms IS NULL)
    OR (state='launch_resolved' AND revision=2 AND dispatch_id IS NOT NULL AND resolved_at_ms IS NOT NULL AND retirement_evidence_id IS NULL AND retired_at_ms IS NULL)
    OR (state='retired' AND revision IN (2,3) AND retirement_evidence_id IS NOT NULL AND retired_at_ms IS NOT NULL AND ((dispatch_id IS NULL AND resolved_at_ms IS NULL) OR (dispatch_id IS NOT NULL AND resolved_at_ms IS NOT NULL))))
) STRICT;
CREATE UNIQUE INDEX terminal_allocations_active_property ON terminal_allocations(mission_id,purpose_key) WHERE state IN ('allocated','launch_resolved');
CREATE TRIGGER terminal_allocation_initial_state BEFORE INSERT ON terminal_allocations
WHEN NEW.state!='allocated' OR NEW.revision!=1 BEGIN SELECT RAISE(ABORT,'terminal preparation must start allocated'); END;
CREATE TRIGGER terminal_allocation_immutable BEFORE UPDATE ON terminal_allocations
WHEN NEW.terminal_id IS NOT OLD.terminal_id OR NEW.receipt_id IS NOT OLD.receipt_id OR NEW.action_id IS NOT OLD.action_id OR NEW.process_id IS NOT OLD.process_id OR NEW.process_run_uuid IS NOT OLD.process_run_uuid OR NEW.principal_id IS NOT OLD.principal_id OR NEW.mission_id IS NOT OLD.mission_id OR NEW.project_id IS NOT OLD.project_id OR NEW.purpose_key IS NOT OLD.purpose_key OR NEW.lease_id IS NOT OLD.lease_id OR NEW.preparation_digest IS NOT OLD.preparation_digest OR NEW.context_digest IS NOT OLD.context_digest OR NEW.context_json IS NOT OLD.context_json
BEGIN SELECT RAISE(ABORT,'terminal preparation immutable'); END;
CREATE TRIGGER terminal_allocation_transition BEFORE UPDATE ON terminal_allocations
WHEN NOT((OLD.state='allocated' AND NEW.state='launch_resolved' AND NEW.revision=OLD.revision+1) OR (OLD.state IN ('allocated','launch_resolved') AND NEW.state='retired' AND NEW.revision=OLD.revision+1 AND NEW.dispatch_id IS OLD.dispatch_id AND NEW.resolved_at_ms IS OLD.resolved_at_ms))
BEGIN SELECT RAISE(ABORT,'terminal preparation transition invalid'); END;
CREATE TABLE terminal_disposition_observations (
 evidence_id TEXT PRIMARY KEY REFERENCES evidence(evidence_id) DEFERRABLE INITIALLY DEFERRED,
 terminal_id TEXT NOT NULL REFERENCES terminal_allocations(terminal_id),
 context_digest TEXT NOT NULL,
 observation_object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
 dispatch_id TEXT REFERENCES action_dispatches(dispatch_id)
) STRICT;
CREATE TRIGGER terminal_disposition_causality BEFORE INSERT ON terminal_disposition_observations
WHEN EXISTS(SELECT 1 FROM evidence WHERE evidence_id=NEW.evidence_id) OR NOT EXISTS(
 SELECT 1 FROM terminal_allocations t WHERE t.terminal_id=NEW.terminal_id AND t.context_digest=NEW.context_digest
 AND ((NEW.dispatch_id IS NULL AND t.state='allocated' AND NOT EXISTS(SELECT 1 FROM action_dispatches WHERE action_id=t.action_id))
 OR (NEW.dispatch_id IS NOT NULL AND t.state='launch_resolved' AND t.dispatch_id=NEW.dispatch_id
 AND EXISTS(SELECT 1 FROM action_dispatches d WHERE d.dispatch_id=NEW.dispatch_id AND d.action_id=t.action_id))))
BEGIN SELECT RAISE(ABORT,'terminal disposition must precede its new evidence after the exact marker'); END;
CREATE TRIGGER terminal_disposition_evidence BEFORE INSERT ON evidence
WHEN EXISTS(SELECT 1 FROM terminal_disposition_observations o WHERE o.evidence_id=NEW.evidence_id) AND NOT EXISTS(
 SELECT 1 FROM terminal_disposition_observations o JOIN terminal_allocations t ON t.terminal_id=o.terminal_id
 JOIN principals p ON p.principal_id=NEW.producer_id JOIN object_records r ON r.object_hash=NEW.object_hash
 WHERE o.evidence_id=NEW.evidence_id AND NEW.kind='observation' AND NEW.schema_version=1 AND NEW.availability='available'
 AND p.state='active' AND p.principal_kind IN ('service','system') AND r.state='live' AND NEW.object_hash=o.observation_object_hash
 AND (o.dispatch_id IS NULL OR EXISTS(SELECT 1 FROM action_dispatches d WHERE d.dispatch_id=o.dispatch_id AND NEW.observed_at_ms>=d.started_at_ms))
 AND NEW.snapshot_ref='terminal-disposition:v1:'||o.context_digest
 AND (NEW.action_id=t.action_id OR (NEW.action_id IS NULL AND NOT EXISTS(SELECT 1 FROM actions WHERE action_id=t.action_id))))
BEGIN SELECT RAISE(ABORT,'terminal disposition evidence is not an attributable observation'); END;
CREATE TRIGGER terminal_disposition_immutable BEFORE UPDATE ON terminal_disposition_observations BEGIN SELECT RAISE(ABORT,'terminal disposition immutable'); END;
CREATE TRIGGER terminal_disposition_durable BEFORE DELETE ON terminal_disposition_observations BEGIN SELECT RAISE(ABORT,'terminal disposition durable'); END;
CREATE TRIGGER terminal_disposition_evidence_immutable BEFORE UPDATE ON evidence
WHEN EXISTS(SELECT 1 FROM terminal_disposition_observations WHERE evidence_id=OLD.evidence_id)
AND (NEW.evidence_id IS NOT OLD.evidence_id OR NEW.kind IS NOT OLD.kind OR NEW.action_id IS NOT OLD.action_id OR NEW.snapshot_ref IS NOT OLD.snapshot_ref OR NEW.object_hash IS NOT OLD.object_hash OR NEW.producer_id IS NOT OLD.producer_id OR NEW.observed_at_ms IS NOT OLD.observed_at_ms OR NEW.schema_version IS NOT OLD.schema_version)
BEGIN SELECT RAISE(ABORT,'terminal disposition evidence binding immutable'); END;
CREATE TRIGGER terminal_allocation_retirement_proof BEFORE UPDATE ON terminal_allocations
WHEN NEW.state='retired' AND NOT EXISTS(
 SELECT 1 FROM evidence e JOIN object_records r ON r.object_hash=e.object_hash JOIN principals p ON p.principal_id=e.producer_id
 JOIN terminal_disposition_observations o ON o.evidence_id=e.evidence_id
 WHERE e.evidence_id=NEW.retirement_evidence_id AND o.terminal_id=OLD.terminal_id AND o.context_digest=OLD.context_digest
 AND o.dispatch_id IS OLD.dispatch_id AND e.object_hash=o.observation_object_hash AND e.kind='observation' AND e.schema_version=1 AND e.availability='available'
 AND r.state='live' AND p.state='active' AND p.principal_kind IN ('service','system')
 AND e.snapshot_ref='terminal-disposition:v1:'||OLD.context_digest
 AND (e.action_id=OLD.action_id OR (e.action_id IS NULL AND NOT EXISTS(SELECT 1 FROM actions WHERE action_id=OLD.action_id)))
 AND (o.dispatch_id IS NOT NULL OR NOT EXISTS(SELECT 1 FROM action_dispatches WHERE action_id=OLD.action_id))
 AND NOT EXISTS(SELECT 1 FROM actions a WHERE a.action_id=OLD.action_id AND (a.authority_generation IS NOT json_extract(OLD.context_json,'$.authority_generation') OR a.daemon_epoch IS NOT json_extract(OLD.context_json,'$.daemon_epoch') OR a.fence!=0 OR a.origin_kind!='control_plane' OR a.control_principal_id IS NOT OLD.principal_id))
 AND NOT EXISTS(SELECT 1 FROM terminal_sessions s WHERE s.terminal_id=OLD.terminal_id AND NOT EXISTS(
 SELECT 1 FROM processes q WHERE q.process_id=s.process_id AND q.process_id=OLD.process_id AND q.process_run_uuid=OLD.process_run_uuid
 AND q.action_id=OLD.action_id AND q.authority_generation=json_extract(OLD.context_json,'$.authority_generation') AND q.state='exited' AND s.state='exited' AND s.mission_id=OLD.mission_id
 AND s.purpose_key=OLD.purpose_key AND s.lease_id=OLD.lease_id AND s.owner_agent_id IS NULL))
 AND NOT EXISTS(SELECT 1 FROM processes q WHERE (q.process_id=OLD.process_id OR q.process_run_uuid=OLD.process_run_uuid)
 AND (q.process_id!=OLD.process_id OR q.process_run_uuid!=OLD.process_run_uuid OR q.action_id!=OLD.action_id OR q.authority_generation IS NOT json_extract(OLD.context_json,'$.authority_generation') OR q.state!='exited')))
BEGIN SELECT RAISE(ABORT,'terminal retirement requires exact causal internal observation'); END;
CREATE TRIGGER terminal_allocation_durable BEFORE DELETE ON terminal_allocations BEGIN SELECT RAISE(ABORT,'terminal preparation durable'); END;
";

const V009_SQL: &str = r"
PRAGMA defer_foreign_keys=ON;
CREATE TABLE provider_probe_contexts (
    probe_id TEXT PRIMARY KEY REFERENCES provider_probes(probe_id),
    receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
    actor_id TEXT NOT NULL,
    principal_id TEXT NOT NULL REFERENCES principals(principal_id),
    project_id TEXT REFERENCES projects(project_id),
    profile_version_ref TEXT NOT NULL REFERENCES provider_profiles(profile_version_ref),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK(daemon_epoch>=0),
    action_id TEXT NOT NULL UNIQUE,
    model_call_id TEXT NOT NULL UNIQUE,
    manifest_id TEXT NOT NULL UNIQUE,
    preparation_digest TEXT NOT NULL,
    context_digest TEXT NOT NULL UNIQUE,
    context_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE TRIGGER provider_probe_contexts_no_update BEFORE UPDATE ON provider_probe_contexts BEGIN SELECT RAISE(ABORT,'provider probe context immutable'); END;
CREATE TRIGGER provider_probe_contexts_no_delete BEFORE DELETE ON provider_probe_contexts BEGIN SELECT RAISE(ABORT,'provider probe context durable'); END;
CREATE TRIGGER admission_source_probe_context_insert AFTER INSERT ON provider_probe_contexts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_probe_context_update AFTER UPDATE ON provider_probe_contexts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_probe_context_delete AFTER DELETE ON provider_probe_contexts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;

CREATE TABLE model_call_manifests_v009 (
    manifest_id TEXT PRIMARY KEY,
    model_call_id TEXT NOT NULL REFERENCES model_calls(model_call_id),
    manifest_kind TEXT NOT NULL CHECK(manifest_kind IN ('request','result')),
    ordinal INTEGER NOT NULL CHECK(ordinal>=1),
    parent_manifest_id TEXT REFERENCES model_call_manifests_v009(manifest_id),
    context_manifest_id TEXT REFERENCES context_manifests(manifest_id),
    provider_id TEXT NOT NULL,
    resolved_model_id TEXT NOT NULL,
    capabilities_hash TEXT NOT NULL,
    allowed_model_policy_hash TEXT NOT NULL,
    request_payload_hash TEXT NOT NULL,
    usage_object_hash TEXT,
    result_object_hash TEXT,
    content_object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    created_at_ms INTEGER NOT NULL,
    context_kind TEXT NOT NULL DEFAULT 'invocation_context' CHECK(context_kind IN ('invocation_context','provider_probe_context')),
    provider_probe_context_id TEXT REFERENCES provider_probe_contexts(probe_id),
    UNIQUE(model_call_id,manifest_kind,ordinal),
    CHECK((context_kind='invocation_context' AND context_manifest_id IS NOT NULL AND provider_probe_context_id IS NULL)
       OR (context_kind='provider_probe_context' AND context_manifest_id IS NULL AND provider_probe_context_id IS NOT NULL))
) STRICT;
INSERT INTO model_call_manifests_v009
 (manifest_id,model_call_id,manifest_kind,ordinal,parent_manifest_id,context_manifest_id,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,usage_object_hash,result_object_hash,content_object_hash,created_at_ms,context_kind,provider_probe_context_id)
 SELECT manifest_id,model_call_id,manifest_kind,ordinal,parent_manifest_id,context_manifest_id,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,usage_object_hash,result_object_hash,content_object_hash,created_at_ms,'invocation_context',NULL FROM model_call_manifests;
DROP TABLE model_call_manifests;
ALTER TABLE model_call_manifests_v009 RENAME TO model_call_manifests;
CREATE TRIGGER model_call_context_owner BEFORE INSERT ON model_call_manifests
WHEN NOT EXISTS (
 SELECT 1 FROM model_calls c
 LEFT JOIN invocations i ON i.invocation_id=c.invocation_id
 LEFT JOIN context_manifests x ON x.manifest_id=NEW.context_manifest_id
 LEFT JOIN provider_probe_contexts p ON p.probe_id=NEW.provider_probe_context_id
 WHERE c.model_call_id=NEW.model_call_id AND
 ((NEW.context_kind='invocation_context' AND c.owner_kind='invocation' AND x.session_id=i.session_id)
 OR (NEW.context_kind='provider_probe_context' AND c.owner_kind='provider_probe' AND p.probe_id=c.provider_probe_id)))
BEGIN SELECT RAISE(ABORT,'model context does not belong to model origin'); END;
CREATE TRIGGER model_call_manifests_no_update BEFORE UPDATE ON model_call_manifests BEGIN SELECT RAISE(ABORT,'model call manifest immutable'); END;
CREATE TRIGGER model_call_manifests_no_delete BEFORE DELETE ON model_call_manifests BEGIN SELECT RAISE(ABORT,'model call manifest durable'); END;
CREATE TRIGGER admission_source_model_call_manifests_insert AFTER INSERT ON model_call_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_model_call_manifests_update AFTER UPDATE ON model_call_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_model_call_manifests_delete AFTER DELETE ON model_call_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
";

// REQ-AGENT-003/005: conservative CAS over every durable admission input.
// Proof rows are checked separately in the writer and never become a queue.
const V008_SQL: &str = r"
CREATE TABLE admission_source_clock(singleton INTEGER PRIMARY KEY CHECK(singleton=1),revision INTEGER NOT NULL CHECK(revision>=0)) STRICT;
INSERT INTO admission_source_clock VALUES(1,0);
CREATE TABLE admission_observation_clock(singleton INTEGER PRIMARY KEY CHECK(singleton=1),revision INTEGER NOT NULL CHECK(revision>=0)) STRICT;
INSERT INTO admission_observation_clock VALUES(1,0);
CREATE TRIGGER admission_evidence_insert AFTER INSERT ON evidence BEGIN UPDATE admission_observation_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_evidence_update AFTER UPDATE ON evidence BEGIN UPDATE admission_observation_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_evidence_delete AFTER DELETE ON evidence BEGIN UPDATE admission_observation_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TABLE execution_scheduler_state(pool_id TEXT PRIMARY KEY REFERENCES capacity_pools(pool_id),consecutive_controls INTEGER NOT NULL DEFAULT 0 CHECK(consecutive_controls>=0)) STRICT;
CREATE TABLE execution_fairness(pool_id TEXT NOT NULL REFERENCES capacity_pools(pool_id),project_key TEXT NOT NULL,deficit INTEGER NOT NULL,PRIMARY KEY(pool_id,project_key)) STRICT;
ALTER TABLE replay_workers ADD COLUMN queued_at_ms INTEGER;
ALTER TABLE provider_probes ADD COLUMN queued_at_ms INTEGER;
ALTER TABLE invocations ADD COLUMN queued_at_ms INTEGER;
CREATE TABLE admission_eligibility_observations(observation_seq INTEGER PRIMARY KEY AUTOINCREMENT,evidence_id TEXT NOT NULL UNIQUE REFERENCES evidence(evidence_id),valid_until_ms INTEGER NOT NULL,urgency TEXT NOT NULL CHECK(urgency IN ('ordinary','urgent_control'))) STRICT;
CREATE TRIGGER admission_eligibility_insert AFTER INSERT ON admission_eligibility_observations BEGIN UPDATE admission_observation_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_eligibility_immutable BEFORE UPDATE ON admission_eligibility_observations BEGIN SELECT RAISE(ABORT,'eligibility observation immutable'); END;
CREATE TRIGGER admission_eligibility_durable BEFORE DELETE ON admission_eligibility_observations BEGIN SELECT RAISE(ABORT,'eligibility observation durable'); END;
CREATE TABLE execution_checkpoint_requests(reservation_id TEXT PRIMARY KEY REFERENCES capacity_reservations(reservation_id),waiting_control TEXT NOT NULL REFERENCES invocations(invocation_id),requested_at_ms INTEGER NOT NULL,authority_generation TEXT NOT NULL,daemon_epoch INTEGER NOT NULL) STRICT;
CREATE TABLE execution_checkpoint_observations(reservation_id TEXT PRIMARY KEY REFERENCES capacity_reservations(reservation_id),observation_json TEXT NOT NULL,recorded_at_ms INTEGER NOT NULL) STRICT;
CREATE TRIGGER execution_checkpoint_observation_immutable BEFORE UPDATE ON execution_checkpoint_observations BEGIN SELECT RAISE(ABORT,'checkpoint observation immutable'); END;
CREATE TRIGGER execution_checkpoint_observation_durable BEFORE DELETE ON execution_checkpoint_observations BEGIN SELECT RAISE(ABORT,'checkpoint observation durable'); END;
CREATE TRIGGER admission_source_actions_insert AFTER INSERT ON actions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_actions_update AFTER UPDATE ON actions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_actions_delete AFTER DELETE ON actions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_action_dispatches_insert AFTER INSERT ON action_dispatches BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_action_dispatches_update AFTER UPDATE ON action_dispatches BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_action_dispatches_delete AFTER DELETE ON action_dispatches BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_model_calls_insert AFTER INSERT ON model_calls BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_model_calls_update AFTER UPDATE ON model_calls BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_model_calls_delete AFTER DELETE ON model_calls BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_model_call_manifests_insert AFTER INSERT ON model_call_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_model_call_manifests_update AFTER UPDATE ON model_call_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_model_call_manifests_delete AFTER DELETE ON model_call_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_processes_insert AFTER INSERT ON processes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_processes_update AFTER UPDATE ON processes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_processes_delete AFTER DELETE ON processes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_versions_insert AFTER INSERT ON workflow_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_versions_update AFTER UPDATE ON workflow_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_versions_delete AFTER DELETE ON workflow_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_timers_insert AFTER INSERT ON workflow_timers BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_timers_update AFTER UPDATE ON workflow_timers BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_timers_delete AFTER DELETE ON workflow_timers BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_entries_insert AFTER INSERT ON registry_entries BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_entries_update AFTER UPDATE ON registry_entries BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_entries_delete AFTER DELETE ON registry_entries BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_versions_insert AFTER INSERT ON registry_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_versions_update AFTER UPDATE ON registry_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_versions_delete AFTER DELETE ON registry_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_bindings_insert AFTER INSERT ON registry_bindings BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_bindings_update AFTER UPDATE ON registry_bindings BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_registry_bindings_delete AFTER DELETE ON registry_bindings BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_automation_versions_insert AFTER INSERT ON automation_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_automation_versions_update AFTER UPDATE ON automation_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_automation_versions_delete AFTER DELETE ON automation_versions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_automation_runs_insert AFTER INSERT ON automation_runs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_automation_runs_update AFTER UPDATE ON automation_runs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_automation_runs_delete AFTER DELETE ON automation_runs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_events_insert AFTER INSERT ON events BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_events_update AFTER UPDATE ON events BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_events_delete AFTER DELETE ON events BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_checkpoint_requests_insert AFTER INSERT ON execution_checkpoint_requests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_checkpoint_requests_update AFTER UPDATE ON execution_checkpoint_requests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_checkpoint_requests_delete AFTER DELETE ON execution_checkpoint_requests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_daemon_state_insert AFTER INSERT ON daemon_state BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_daemon_state_update AFTER UPDATE ON daemon_state BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_daemon_state_delete AFTER DELETE ON daemon_state BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_capacity_pools_insert AFTER INSERT ON capacity_pools BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_capacity_pools_update AFTER UPDATE ON capacity_pools BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_capacity_pools_delete AFTER DELETE ON capacity_pools BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_capacity_reservations_insert AFTER INSERT ON capacity_reservations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_capacity_reservations_update AFTER UPDATE ON capacity_reservations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_capacity_reservations_delete AFTER DELETE ON capacity_reservations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_reservation_owners_insert AFTER INSERT ON execution_reservation_owners BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_reservation_owners_update AFTER UPDATE ON execution_reservation_owners BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_reservation_owners_delete AFTER DELETE ON execution_reservation_owners BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_projects_insert AFTER INSERT ON projects BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_projects_update AFTER UPDATE ON projects BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_projects_delete AFTER DELETE ON projects BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_principals_insert AFTER INSERT ON principals BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_principals_update AFTER UPDATE ON principals BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_principals_delete AFTER DELETE ON principals BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_agents_insert AFTER INSERT ON agents BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_agents_update AFTER UPDATE ON agents BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_agents_delete AFTER DELETE ON agents BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_sessions_insert AFTER INSERT ON sessions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_sessions_update AFTER UPDATE ON sessions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_sessions_delete AFTER DELETE ON sessions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_context_manifests_insert AFTER INSERT ON context_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_context_manifests_update AFTER UPDATE ON context_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_context_manifests_delete AFTER DELETE ON context_manifests BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_effective_bundles_insert AFTER INSERT ON effective_bundles BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_effective_bundles_update AFTER UPDATE ON effective_bundles BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_effective_bundles_delete AFTER DELETE ON effective_bundles BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_invocations_insert AFTER INSERT ON invocations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_invocations_update AFTER UPDATE ON invocations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_invocations_delete AFTER DELETE ON invocations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_invocation_attempts_insert AFTER INSERT ON invocation_attempts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_invocation_attempts_update AFTER UPDATE ON invocation_attempts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_invocation_attempts_delete AFTER DELETE ON invocation_attempts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_replay_workers_insert AFTER INSERT ON replay_workers BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_replay_workers_update AFTER UPDATE ON replay_workers BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_replay_workers_delete AFTER DELETE ON replay_workers BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_provider_probes_insert AFTER INSERT ON provider_probes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_provider_probes_update AFTER UPDATE ON provider_probes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_provider_probes_delete AFTER DELETE ON provider_probes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_provider_profiles_insert AFTER INSERT ON provider_profiles BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_provider_profiles_update AFTER UPDATE ON provider_profiles BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_provider_profiles_delete AFTER DELETE ON provider_profiles BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_runs_insert AFTER INSERT ON workflow_runs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_runs_update AFTER UPDATE ON workflow_runs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_workflow_runs_delete AFTER DELETE ON workflow_runs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_step_instances_insert AFTER INSERT ON step_instances BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_step_instances_update AFTER UPDATE ON step_instances BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_step_instances_delete AFTER DELETE ON step_instances BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_tasks_insert AFTER INSERT ON tasks BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_tasks_update AFTER UPDATE ON tasks BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_tasks_delete AFTER DELETE ON tasks BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_missions_insert AFTER INSERT ON missions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_missions_update AFTER UPDATE ON missions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_missions_delete AFTER DELETE ON missions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_security_policies_insert AFTER INSERT ON security_policies BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_security_policies_update AFTER UPDATE ON security_policies BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_security_policies_delete AFTER DELETE ON security_policies BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_subject_suspensions_insert AFTER INSERT ON subject_suspensions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_subject_suspensions_update AFTER UPDATE ON subject_suspensions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_subject_suspensions_delete AFTER DELETE ON subject_suspensions BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_settings_insert AFTER INSERT ON settings BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_settings_update AFTER UPDATE ON settings BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_settings_delete AFTER DELETE ON settings BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_guard_configs_insert AFTER INSERT ON guard_configs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_guard_configs_update AFTER UPDATE ON guard_configs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_guard_configs_delete AFTER DELETE ON guard_configs BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_secret_references_insert AFTER INSERT ON secret_references BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_secret_references_update AFTER UPDATE ON secret_references BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_secret_references_delete AFTER DELETE ON secret_references BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_recovery_scope_freezes_insert AFTER INSERT ON recovery_scope_freezes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_recovery_scope_freezes_update AFTER UPDATE ON recovery_scope_freezes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_recovery_scope_freezes_delete AFTER DELETE ON recovery_scope_freezes BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_resource_leases_insert AFTER INSERT ON resource_leases BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_resource_leases_update AFTER UPDATE ON resource_leases BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_resource_leases_delete AFTER DELETE ON resource_leases BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_resource_fences_insert AFTER INSERT ON resource_fences BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_resource_fences_update AFTER UPDATE ON resource_fences BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_resource_fences_delete AFTER DELETE ON resource_fences BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_budget_accounts_insert AFTER INSERT ON budget_accounts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_budget_accounts_update AFTER UPDATE ON budget_accounts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_budget_accounts_delete AFTER DELETE ON budget_accounts BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_budget_reservations_insert AFTER INSERT ON budget_reservations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_budget_reservations_update AFTER UPDATE ON budget_reservations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_budget_reservations_delete AFTER DELETE ON budget_reservations BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_object_records_insert AFTER INSERT ON object_records BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_object_records_update AFTER UPDATE ON object_records BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_object_records_delete AFTER DELETE ON object_records BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_fairness_insert AFTER INSERT ON execution_fairness BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_fairness_update AFTER UPDATE ON execution_fairness BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_fairness_delete AFTER DELETE ON execution_fairness BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_scheduler_state_insert AFTER INSERT ON execution_scheduler_state BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_scheduler_state_update AFTER UPDATE ON execution_scheduler_state BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_execution_scheduler_state_delete AFTER DELETE ON execution_scheduler_state BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
";

const V007_SQL:&str="
ALTER TABLE backup_requests ADD COLUMN block_reason TEXT CHECK (block_reason IS NULL OR block_reason IN ('guard_profile_unavailable','isolation_profile_unavailable','authorization_denied'));
CREATE TRIGGER backup_block_reason_immutable BEFORE UPDATE ON backup_requests
WHEN OLD.block_reason IS NOT NULL AND OLD.block_reason IS NOT NEW.block_reason
BEGIN SELECT RAISE(ABORT,'backup block reason immutable'); END;
";

const V006_SQL: &str = "
CREATE TABLE backup_requests (
    backup_id TEXT PRIMARY KEY,
    receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
    action_id TEXT NOT NULL UNIQUE,
    actor_id TEXT NOT NULL,
    principal_id TEXT NOT NULL REFERENCES principals(principal_id),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    scope_revision INTEGER NOT NULL CHECK (scope_revision >= 0),
    policy_revision INTEGER NOT NULL CHECK (policy_revision >= 0),
    destination_path TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('reserved','claimed','executing','completed','blocked')),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    dispatch_id TEXT UNIQUE REFERENCES action_dispatches(dispatch_id),
    barrier_pin_id TEXT UNIQUE REFERENCES object_pins(pin_id),
    manifest_digest TEXT,
    manifest_json TEXT,
    created_at_ms INTEGER NOT NULL,
    CHECK ((state IN ('executing','completed') AND dispatch_id IS NOT NULL AND barrier_pin_id IS NOT NULL) OR state NOT IN ('executing','completed')),
    CHECK (state != 'completed' OR (manifest_digest IS NOT NULL AND manifest_json IS NOT NULL))
) STRICT;
CREATE TRIGGER backup_requests_immutable BEFORE UPDATE ON backup_requests
WHEN OLD.backup_id IS NOT NEW.backup_id OR OLD.receipt_id IS NOT NEW.receipt_id
 OR OLD.action_id IS NOT NEW.action_id OR OLD.actor_id IS NOT NEW.actor_id
 OR OLD.principal_id IS NOT NEW.principal_id OR OLD.authority_generation IS NOT NEW.authority_generation
 OR OLD.destination_path IS NOT NEW.destination_path OR OLD.payload_digest IS NOT NEW.payload_digest
 OR OLD.created_at_ms IS NOT NEW.created_at_ms
 OR (OLD.state != 'reserved' AND (OLD.daemon_epoch IS NOT NEW.daemon_epoch OR OLD.scope_revision IS NOT NEW.scope_revision OR OLD.policy_revision IS NOT NEW.policy_revision))
 OR (OLD.dispatch_id IS NOT NULL AND OLD.dispatch_id IS NOT NEW.dispatch_id)
 OR (OLD.barrier_pin_id IS NOT NULL AND OLD.barrier_pin_id IS NOT NEW.barrier_pin_id)
 OR (OLD.manifest_digest IS NOT NULL AND OLD.manifest_digest IS NOT NEW.manifest_digest)
 OR (OLD.manifest_json IS NOT NULL AND OLD.manifest_json IS NOT NEW.manifest_json)
 OR (OLD.state != 'reserved' AND NEW.state = 'reserved')
BEGIN SELECT RAISE(ABORT,'backup request immutable identity/provenance'); END;
CREATE TRIGGER backup_requests_no_delete BEFORE DELETE ON backup_requests BEGIN SELECT RAISE(ABORT,'backup request durable identity'); END;
";

const V005_SQL: &str = "
CREATE TABLE broker_intents (
    action_id TEXT PRIMARY KEY REFERENCES actions(action_id),
    intent_digest TEXT NOT NULL,
    intent_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE TABLE broker_evaluations (
    evaluation_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL REFERENCES broker_intents(action_id),
    previous_evaluation_id TEXT UNIQUE REFERENCES broker_evaluations(evaluation_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    policy_snapshot_json TEXT NOT NULL,
    policy_snapshot_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('authorizing','authorized','approval_required','denied','retryable')),
    retry_not_before_ms INTEGER,
    UNIQUE (action_id, ordinal)
) STRICT;
CREATE TABLE broker_phase_records (
    action_id TEXT NOT NULL REFERENCES broker_intents(action_id),
    evaluation_id TEXT NOT NULL REFERENCES broker_evaluations(evaluation_id),
    phase_key TEXT NOT NULL,
    phase_json TEXT NOT NULL,
    phase_digest TEXT NOT NULL,
    action_revision INTEGER NOT NULL CHECK (action_revision >= 1),
    event_id TEXT NOT NULL REFERENCES events(event_id),
    event_sequence INTEGER NOT NULL REFERENCES events(sequence),
    PRIMARY KEY (action_id,evaluation_id,phase_key)
) STRICT;
CREATE TABLE broker_dispatches (
    dispatch_id TEXT PRIMARY KEY REFERENCES action_dispatches(dispatch_id),
    action_id TEXT NOT NULL UNIQUE REFERENCES broker_intents(action_id),
    evaluation_id TEXT NOT NULL REFERENCES broker_evaluations(evaluation_id),
    decision_json TEXT NOT NULL,
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL,
    fence INTEGER NOT NULL
) STRICT;
CREATE TRIGGER broker_intents_no_update BEFORE UPDATE ON broker_intents BEGIN SELECT RAISE(ABORT,'broker intent immutable'); END;
CREATE TRIGGER broker_intents_no_delete BEFORE DELETE ON broker_intents BEGIN SELECT RAISE(ABORT,'broker intent immutable'); END;
CREATE TRIGGER broker_phase_records_no_update BEFORE UPDATE ON broker_phase_records BEGIN SELECT RAISE(ABORT,'broker phase immutable'); END;
CREATE TRIGGER broker_phase_records_no_delete BEFORE DELETE ON broker_phase_records BEGIN SELECT RAISE(ABORT,'broker phase immutable'); END;
CREATE TRIGGER broker_dispatches_no_update BEFORE UPDATE ON broker_dispatches BEGIN SELECT RAISE(ABORT,'broker dispatch immutable'); END;
CREATE TRIGGER broker_dispatches_no_delete BEFORE DELETE ON broker_dispatches BEGIN SELECT RAISE(ABORT,'broker dispatch immutable'); END;
";

fn migration_checksum(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sql.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    format!("sha256:{}", hex::encode(digest))
}

/// Contrôle en lecture seule la compatibilité du schéma avant toute écriture (`REQ-PERSIST-005`).
///
/// # Errors
/// Retourne [`StoreError::UnsupportedSchemaVersion`], [`StoreError::MigrationInterrupted`],
/// [`StoreError::MigrationChecksumMismatch`], [`StoreError::MigrationBackupRequired`] ou [`StoreError::StorageCorrupt`].
pub(crate) fn preflight_check_existing_db(
    ro_conn: &Connection,
    max_supported_version: i64,
    pre_migration_backup_id: Option<BackupId>,
) -> Result<i64, StoreError> {
    preflight_check_inner(
        ro_conn,
        max_supported_version,
        pre_migration_backup_id,
        false,
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "Validation séquentielle stricte quick_check, user_version, états, checksums et pré-sauvegarde"
)]
fn preflight_check_inner(
    ro_conn: &Connection,
    max_supported_version: i64,
    pre_migration_backup_id: Option<BackupId>,
    backup_verified_externally: bool,
) -> Result<i64, StoreError> {
    let quick: String = ro_conn
        .pragma_query_value(None, "quick_check", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    if !quick.eq_ignore_ascii_case("ok") {
        return Err(StoreError::StorageCorrupt(format!(
            "PRAGMA quick_check a retourné : {quick}"
        )));
    }

    let user_version: i64 = ro_conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;

    if user_version > max_supported_version {
        return Err(StoreError::UnsupportedSchemaVersion {
            found: user_version,
            supported_max: max_supported_version,
        });
    }

    let has_table: i64 = ro_conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;

    if has_table == 0 {
        if user_version > 0 {
            return Err(StoreError::MigrationInterrupted {
                version: user_version,
                detail: "table schema_migrations absente pour user_version > 0".to_owned(),
            });
        }
        return Ok(0);
    }

    let mut stmt = ro_conn
        .prepare(
            "SELECT version, checksum_sha256, state FROM schema_migrations ORDER BY version ASC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(StoreError::from_rusqlite)?;

    let mut collected = Vec::new();
    for row_res in rows {
        collected.push(row_res.map_err(StoreError::from_rusqlite)?);
    }

    for (ver, _, _) in &collected {
        if *ver > max_supported_version {
            return Err(StoreError::UnsupportedSchemaVersion {
                found: *ver,
                supported_max: max_supported_version,
            });
        }
    }

    for (ver, _, state_str) in &collected {
        let state = MigrationState::from_sql(state_str).ok_or_else(|| {
            StoreError::MigrationInterrupted {
                version: *ver,
                detail: format!("état de migration inconnu '{state_str}'"),
            }
        })?;
        if state != MigrationState::Applied {
            return Err(StoreError::MigrationInterrupted {
                version: *ver,
                detail: format!("migration laissée dans l'état '{state_str}'"),
            });
        }
    }

    let mut max_applied = 0_i64;
    for (ver, checksum, _) in collected {
        let def = MIGRATIONS.iter().find(|m| m.version == ver).ok_or(
            StoreError::UnsupportedSchemaVersion {
                found: ver,
                supported_max: max_supported_version,
            },
        )?;
        let expected = migration_checksum(def.sql);
        if checksum != expected {
            return Err(StoreError::MigrationChecksumMismatch {
                version: ver,
                expected,
                actual: checksum,
            });
        }
        if ver > max_applied {
            max_applied = ver;
        }
    }

    if max_applied != user_version {
        tracing::warn!(
            user_version,
            max_applied,
            "migration interrompue : discordance user_version vs schema_migrations"
        );
        return Err(StoreError::MigrationInterrupted {
            version: max_applied.max(user_version),
            detail: format!(
                "discordance PRAGMA user_version ({user_version}) vs schema_migrations ({max_applied})"
            ),
        });
    }

    let target_version = max_supported_version.min(CURRENT_SCHEMA_VERSION);
    if max_applied >= 1 && target_version > max_applied && !backup_verified_externally {
        let Some(bkp_id) = pre_migration_backup_id else {
            tracing::warn!(
                max_applied,
                target_version,
                "montée de version refusée sans sauvegarde pré-migration"
            );
            return Err(StoreError::MigrationBackupRequired);
        };
        let valid_backup: i64 = ro_conn
            .query_row(
                "SELECT COUNT(*) FROM backup_jobs WHERE backup_id = ?1 AND state = 'complete'",
                [bkp_id.to_string()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if valid_backup == 0 {
            tracing::warn!(backup_id = %bkp_id, "sauvegarde pré-migration absente ou incomplète");
            return Err(StoreError::MigrationBackupRequired);
        }
    }

    Ok(max_applied)
}

/// Applique les migrations manquantes jusqu'à `max_supported_version.min(CURRENT_SCHEMA_VERSION)`.
///
/// # Errors
/// Retourne [`StoreError`] en cas d'incohérence, absence de sauvegarde pré-migration ou panne `C11`.
pub(crate) fn apply_pending_migrations(
    conn: &mut Connection,
    max_supported_version: i64,
    pre_migration_backup_id: Option<BackupId>,
    now_ms: TimestampMs,
    cutpoints: &dyn CutpointObserver,
) -> Result<MigrationReport, StoreError> {
    apply_pending_migrations_inner(
        conn,
        max_supported_version,
        pre_migration_backup_id,
        now_ms,
        cutpoints,
        false,
    )
}

/// Met à niveau un snapshot déjà validé par `validate_backup_dir` lors d'un `RestoreBackup`.
///
/// # Errors
/// Retourne [`StoreError`] en cas d'échec de migration.
pub(crate) fn apply_pending_migrations_for_restore(
    conn: &mut Connection,
    max_supported_version: i64,
    verified_backup_id: BackupId,
    now_ms: TimestampMs,
    cutpoints: &dyn CutpointObserver,
) -> Result<MigrationReport, StoreError> {
    apply_pending_migrations_inner(
        conn,
        max_supported_version,
        Some(verified_backup_id),
        now_ms,
        cutpoints,
        true,
    )
}

fn apply_pending_migrations_inner(
    conn: &mut Connection,
    max_supported_version: i64,
    pre_migration_backup_id: Option<BackupId>,
    now_ms: TimestampMs,
    cutpoints: &dyn CutpointObserver,
    backup_verified_externally: bool,
) -> Result<MigrationReport, StoreError> {
    if max_supported_version < 1 {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("max_supported_schema_version invalide : {max_supported_version}"),
        });
    }

    let current = preflight_check_inner(
        conn,
        max_supported_version,
        pre_migration_backup_id,
        backup_verified_externally,
    )?;
    let target = max_supported_version.min(CURRENT_SCHEMA_VERSION);
    let mut newly_applied = Vec::new();

    for def in MIGRATIONS {
        if def.version <= current || def.version > target {
            continue;
        }
        let checksum = migration_checksum(def.sql);
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::from_rusqlite)?;

        if def.version == 9 {
            // Every legacy row will become invocation_context. Check the same
            // ownership relation as model_call_context_owner before any copy;
            // NOT EXISTS also rejects missing parents instead of ignoring NULL.
            let invalid_owner: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM model_call_manifests m WHERE NOT EXISTS(SELECT 1 FROM model_calls c JOIN invocations i ON i.invocation_id=c.invocation_id JOIN context_manifests x ON x.manifest_id=m.context_manifest_id WHERE c.model_call_id=m.model_call_id AND c.owner_kind='invocation' AND x.session_id=i.session_id))", [], |row| row.get(0)).map_err(StoreError::from_rusqlite)?;
            if invalid_owner {
                return Err(StoreError::InvariantViolation("V008 model context does not belong to its invocation origin; explicit reconciliation required before V009".to_owned()));
            }
        }

        tx.execute_batch(def.sql)
            .map_err(StoreError::from_rusqlite)?;

        if def.version == 9 {
            let violations: i64 = tx
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })
                .map_err(StoreError::from_rusqlite)?;
            if violations != 0 {
                return Err(StoreError::InvariantViolation(
                    "V009 context migration foreign key violation".to_owned(),
                ));
            }
        }

        let bkp_str = pre_migration_backup_id.map(|id| id.to_string());
        tx.execute(
            "INSERT INTO schema_migrations (version, name, checksum_sha256, state, pre_migration_backup_id, started_at_ms, completed_at_ms)
             VALUES (?1, ?2, ?3, 'applying', ?4, ?5, NULL)",
            rusqlite::params![def.version, def.name, checksum, bkp_str, now_ms.as_i64()],
        )
        .map_err(StoreError::from_rusqlite)?;

        cutpoints.hit(StoreCutpoint::C11MigrationInProgress)?;

        tx.execute(
            "UPDATE schema_migrations SET state = 'applied', completed_at_ms = ?1 WHERE version = ?2",
            rusqlite::params![now_ms.as_i64(), def.version],
        )
        .map_err(StoreError::from_rusqlite)?;

        tx.pragma_update(None, "user_version", def.version)
            .map_err(StoreError::from_rusqlite)?;

        let daemon_exists: i64 = tx
            .query_row("SELECT COUNT(*) FROM daemon_state", [], |r| r.get(0))
            .map_err(StoreError::from_rusqlite)?;
        if daemon_exists > 0 {
            tx.execute(
                "UPDATE daemon_state SET schema_version = ?1 WHERE singleton = 1",
                [def.version],
            )
            .map_err(StoreError::from_rusqlite)?;
        }

        tx.commit().map_err(StoreError::from_rusqlite)?;
        tracing::info!(
            migration_version = def.version,
            migration_name = def.name,
            "migration SQLite appliquée"
        );
        newly_applied.push(def.version);
    }

    Ok(MigrationReport {
        current_version: current.max(target),
        newly_applied,
    })
}

const V001_SQL: &str = r"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version >= 1),
    name TEXT NOT NULL,
    checksum_sha256 TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('applying', 'applied', 'failed')),
    pre_migration_backup_id TEXT,
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER
) STRICT;

CREATE TABLE IF NOT EXISTS store_runtime_manifests (
    manifest_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    sqlite_version TEXT NOT NULL,
    sqlite_source_id TEXT NOT NULL,
    expected_source_id TEXT NOT NULL,
    compile_options_json TEXT NOT NULL,
    effective_pragmas_json TEXT NOT NULL,
    authority_generation TEXT,
    daemon_epoch INTEGER,
    qualified_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS daemon_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    owner_uid INTEGER NOT NULL CHECK (owner_uid >= 0),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    daemon_instance_id TEXT NOT NULL,
    boot_identity TEXT NOT NULL,
    stream_id TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    mode TEXT NOT NULL CHECK (mode IN ('starting', 'recovering', 'ready', 'degraded', 'read_only_recovery', 'draining', 'stopped')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    started_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS command_receipts (
    actor_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    authority_generation TEXT NOT NULL,
    receipt_id TEXT NOT NULL UNIQUE,
    request_id TEXT NOT NULL,
    command_type TEXT NOT NULL,
    command_digest TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('accepted', 'completed', 'blocked', 'rejected')),
    response_state TEXT NOT NULL CHECK (response_state IN ('full', 'tombstone')),
    response_json TEXT,
    response_retained_until_ms INTEGER NOT NULL,
    first_event_seq INTEGER,
    last_event_seq INTEGER,
    committed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (actor_id, command_id),
    CHECK ((response_state = 'full' AND response_json IS NOT NULL) OR (response_state = 'tombstone' AND response_json IS NULL))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_command_receipts_lineage ON command_receipts(authority_generation, committed_at_ms);
CREATE INDEX IF NOT EXISTS idx_command_receipts_retention ON command_receipts(response_state, response_retained_until_ms);

CREATE TRIGGER IF NOT EXISTS trg_command_receipts_immutable_fields
BEFORE UPDATE ON command_receipts
FOR EACH ROW
WHEN NEW.actor_id != OLD.actor_id
  OR NEW.command_id != OLD.command_id
  OR NEW.authority_generation != OLD.authority_generation
  OR NEW.receipt_id != OLD.receipt_id
  OR NEW.command_digest != OLD.command_digest
  OR NEW.state != OLD.state
  OR NEW.response_retained_until_ms < OLD.response_retained_until_ms
  OR (OLD.response_state = 'tombstone' AND NEW.response_state = 'full')
BEGIN
    SELECT RAISE(ABORT, 'command_receipts immutable key/digest/lineage/retention violation');
END;

CREATE TABLE IF NOT EXISTS events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    stream_id TEXT NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    aggregate_revision INTEGER NOT NULL CHECK (aggregate_revision >= 1),
    event_ordinal INTEGER NOT NULL CHECK (event_ordinal >= 0),
    event_type TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    project_id TEXT,
    actor_ref TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    occurred_at_ms INTEGER NOT NULL,
    recorded_at_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    payload_state TEXT NOT NULL CHECK (payload_state IN ('full', 'tombstone')),
    payload_hash TEXT NOT NULL,
    sensitivity TEXT NOT NULL CHECK (sensitivity IN ('public', 'project', 'confidential', 'restricted')),
    command_actor_id TEXT,
    command_id TEXT,
    causation_kind TEXT NOT NULL CHECK (causation_kind IN ('command', 'event', 'action', 'timer', 'external')),
    causation_id TEXT NOT NULL,
    causation_receipt_id TEXT,
    correlation_id TEXT NOT NULL,
    UNIQUE (aggregate_type, aggregate_id, aggregate_revision, event_ordinal),
    CHECK ((command_actor_id IS NULL AND command_id IS NULL) OR (command_actor_id IS NOT NULL AND command_id IS NOT NULL)),
    CHECK ((causation_kind = 'command' AND causation_receipt_id IS NOT NULL AND causation_id = causation_receipt_id) OR (causation_kind != 'command' AND causation_receipt_id IS NULL)),
    FOREIGN KEY (command_actor_id, command_id) REFERENCES command_receipts(actor_id, command_id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (causation_receipt_id) REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE INDEX IF NOT EXISTS idx_events_stream_seq ON events(stream_id, sequence);
CREATE INDEX IF NOT EXISTS idx_events_aggregate ON events(aggregate_type, aggregate_id, aggregate_revision);
CREATE INDEX IF NOT EXISTS idx_events_recorded ON events(recorded_at_ms);

CREATE TRIGGER IF NOT EXISTS trg_events_no_physical_delete
BEFORE DELETE ON events
BEGIN
    SELECT RAISE(ABORT, 'events rows must never be physically deleted');
END;

CREATE TABLE IF NOT EXISTS outbox (
    event_id TEXT NOT NULL REFERENCES events(event_id),
    destination TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'leased', 'acked', 'dead_letter')),
    delivery_attempt INTEGER NOT NULL DEFAULT 0 CHECK (delivery_attempt >= 0),
    next_attempt_at_ms INTEGER NOT NULL,
    authority_generation TEXT,
    lease_epoch INTEGER,
    consumer_version TEXT,
    delivery_key TEXT NOT NULL UNIQUE,
    acked_at_ms INTEGER,
    last_error TEXT,
    rearm_count INTEGER NOT NULL DEFAULT 0 CHECK (rearm_count IN (0, 1)),
    PRIMARY KEY (event_id, destination)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_outbox_state_next ON outbox(state, next_attempt_at_ms);

CREATE TABLE IF NOT EXISTS inbox (
    consumer_id TEXT NOT NULL,
    event_id TEXT NOT NULL REFERENCES events(event_id),
    processed_at_ms INTEGER NOT NULL,
    result_hash TEXT NOT NULL,
    PRIMARY KEY (consumer_id, event_id)
) STRICT;

CREATE TABLE IF NOT EXISTS projects (
    project_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    weight INTEGER NOT NULL DEFAULT 1 CHECK (weight BETWEEN 1 AND 10),
    state TEXT NOT NULL DEFAULT 'active',
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS principals (
    principal_id TEXT PRIMARY KEY,
    principal_kind TEXT NOT NULL CHECK (principal_kind IN ('user', 'service', 'agent', 'system')),
    actor_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL DEFAULT 'active',
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS agents (
    agent_id TEXT PRIMARY KEY,
    level TEXT NOT NULL CHECK (level IN ('N', 'N+1', 'N+2')),
    organization_parent_id TEXT REFERENCES agents(agent_id),
    created_by_agent_id TEXT REFERENCES agents(agent_id),
    project_id TEXT REFERENCES projects(project_id),
    state TEXT NOT NULL CHECK (state IN ('dormant', 'eligible', 'active', 'blocked', 'paused', 'retired')),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE TRIGGER IF NOT EXISTS trg_agents_immutable_creator
BEFORE UPDATE ON agents
FOR EACH ROW
WHEN NEW.created_by_agent_id IS NOT OLD.created_by_agent_id
BEGIN
    SELECT RAISE(ABORT, 'agents.created_by_agent_id is immutable');
END;

CREATE TABLE IF NOT EXISTS worktrees (
    worktree_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    repo_id TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS goals (
    goal_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('user', 'agent')),
    owner_user_id TEXT REFERENCES principals(principal_id),
    owner_agent_id TEXT REFERENCES agents(agent_id),
    owner_epoch INTEGER NOT NULL DEFAULT 0 CHECK (owner_epoch >= 0),
    completion_policy TEXT NOT NULL CHECK (completion_policy IN ('mechanical', 'human', 'owner_agent')),
    state TEXT NOT NULL CHECK (state IN ('draft', 'active', 'blocked', 'achieved', 'cancelled')),
    definition_ref TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    CHECK ((owner_kind = 'user' AND owner_user_id IS NOT NULL AND owner_agent_id IS NULL)
        OR (owner_kind = 'agent' AND owner_agent_id IS NOT NULL AND owner_user_id IS NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS missions (
    mission_id TEXT PRIMARY KEY,
    goal_id TEXT REFERENCES goals(goal_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('user', 'agent')),
    owner_user_id TEXT REFERENCES principals(principal_id),
    owner_agent_id TEXT REFERENCES agents(agent_id),
    completion_policy TEXT NOT NULL CHECK (completion_policy IN ('mechanical', 'human', 'owner_agent')),
    leader_agent_id TEXT REFERENCES agents(agent_id),
    worktree_id TEXT REFERENCES worktrees(worktree_id),
    state TEXT NOT NULL CHECK (state IN ('proposed', 'ready', 'active', 'paused', 'blocked', 'succeeded', 'failed', 'cancelled')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK ((owner_kind = 'user' AND owner_user_id IS NOT NULL AND owner_agent_id IS NULL)
        OR (owner_kind = 'agent' AND owner_agent_id IS NOT NULL AND owner_user_id IS NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS object_records (
    object_hash TEXT PRIMARY KEY,
    object_generation INTEGER NOT NULL CHECK (object_generation >= 1),
    relative_path TEXT NOT NULL UNIQUE,
    byte_length INTEGER NOT NULL CHECK (byte_length >= 0),
    media_type TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('prepared', 'live', 'deleting', 'tombstone', 'quarantined')),
    delete_operation_id TEXT,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS object_references (
    reference_id TEXT PRIMARY KEY,
    object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    source_field TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('live', 'tombstone')),
    tombstone_reason TEXT,
    UNIQUE (owner_kind, owner_id, source_field, object_hash)
) STRICT;

CREATE TABLE IF NOT EXISTS object_pins (
    pin_id TEXT PRIMARY KEY,
    pin_kind TEXT NOT NULL CHECK (pin_kind IN ('preparation', 'read', 'backup', 'backup_barrier')),
    object_hash TEXT REFERENCES object_records(object_hash),
    object_generation INTEGER CHECK (object_generation IS NULL OR object_generation >= 1),
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    authority_generation TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('held', 'reconciling', 'released')),
    created_at_ms INTEGER NOT NULL,
    released_at_ms INTEGER,
    CHECK ((pin_kind = 'backup_barrier' AND object_hash IS NULL AND object_generation IS NULL)
        OR (pin_kind != 'backup_barrier' AND object_hash IS NOT NULL AND object_generation IS NOT NULL))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_object_pins_active ON object_pins(state, pin_kind, object_hash);

CREATE TABLE IF NOT EXISTS object_deletions (
    delete_operation_id TEXT PRIMARY KEY,
    object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    object_generation INTEGER NOT NULL CHECK (object_generation >= 1),
    relative_path TEXT NOT NULL,
    reason TEXT NOT NULL,
    decision_receipt_id TEXT REFERENCES command_receipts(receipt_id),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'unlinked', 'completed')),
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE (object_hash, object_generation)
) STRICT;

CREATE TABLE IF NOT EXISTS workflow_versions (
    workflow_version_id TEXT PRIMARY KEY,
    registry_version_id TEXT,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    visibility TEXT NOT NULL DEFAULT 'published' CHECK (visibility IN ('published', 'qualification_only', 'deprecated', 'revoked')),
    content_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS workflow_runs (
    run_id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions(mission_id),
    workflow_version_id TEXT NOT NULL REFERENCES workflow_versions(workflow_version_id),
    purpose TEXT NOT NULL DEFAULT 'operational' CHECK (purpose IN ('operational', 'evaluation')),
    candidate_ref TEXT,
    predecessor_run_id TEXT REFERENCES workflow_runs(run_id),
    selection_reason TEXT NOT NULL,
    selected_via_alias TEXT,
    effective_bundle_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('prepared', 'running', 'paused', 'blocked', 'cancelling', 'succeeded', 'failed', 'cancelled')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    CHECK (ended_at_ms IS NULL OR ended_at_ms >= started_at_ms)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_runs_one_successor
ON workflow_runs(predecessor_run_id) WHERE predecessor_run_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS workflow_iterations (
    iteration_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    loop_node_id TEXT NOT NULL,
    iteration_vector TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (run_id, loop_node_id, iteration_vector)
) STRICT;

CREATE TABLE IF NOT EXISTS step_instances (
    step_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    iteration_id TEXT REFERENCES workflow_iterations(iteration_id),
    definition_node_id TEXT NOT NULL,
    iteration_vector TEXT NOT NULL,
    step_attempt INTEGER NOT NULL CHECK (step_attempt >= 1),
    state TEXT NOT NULL CHECK (state IN ('ready', 'queued', 'running', 'waiting_evidence', 'awaiting_decision', 'blocked', 'succeeded', 'failed', 'cancelled')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (run_id, definition_node_id, iteration_vector, step_attempt)
) STRICT;

CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions(mission_id),
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    step_id TEXT REFERENCES step_instances(step_id),
    contract_version_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    scope_revision INTEGER NOT NULL CHECK (scope_revision >= 0),
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(agent_id),
    head_manifest_id TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    state TEXT NOT NULL CHECK (state IN ('open', 'closed', 'retired')),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_one_open_per_agent
ON sessions(agent_id) WHERE state = 'open';

CREATE TABLE IF NOT EXISTS session_entries (
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    entry_seq INTEGER NOT NULL CHECK (entry_seq >= 1),
    entry_kind TEXT NOT NULL,
    object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    causation_event_id TEXT REFERENCES events(event_id),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, entry_seq)
) STRICT;

CREATE TABLE IF NOT EXISTS context_manifests (
    manifest_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    parent_manifest_id TEXT REFERENCES context_manifests(manifest_id),
    base_session_revision INTEGER NOT NULL CHECK (base_session_revision >= 0),
    content_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS effective_bundles (
    bundle_hash TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    role_version_id TEXT NOT NULL,
    workflow_version_id TEXT REFERENCES workflow_versions(workflow_version_id),
    requested_model_profile_version_id TEXT NOT NULL,
    allowed_model_policy_hash TEXT NOT NULL,
    initial_context_manifest_id TEXT NOT NULL REFERENCES context_manifests(manifest_id),
    scope_id TEXT NOT NULL,
    scope_revision INTEGER NOT NULL CHECK (scope_revision >= 0),
    content_object_hash TEXT NOT NULL REFERENCES object_records(object_hash)
) STRICT;

CREATE TABLE IF NOT EXISTS invocations (
    invocation_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(agent_id),
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    parent_invocation_id TEXT REFERENCES invocations(invocation_id),
    trigger_event_id TEXT NOT NULL REFERENCES events(event_id),
    task_id TEXT REFERENCES tasks(task_id),
    run_id TEXT REFERENCES workflow_runs(run_id),
    step_id TEXT REFERENCES step_instances(step_id),
    effective_bundle_hash TEXT NOT NULL REFERENCES effective_bundles(bundle_hash),
    context_manifest_id TEXT NOT NULL REFERENCES context_manifests(manifest_id),
    state TEXT NOT NULL CHECK (state IN ('queued', 'admitted', 'running', 'checkpointing', 'waiting_durable', 'reconciling', 'completed', 'failed', 'cancelled')),
    wait_reason TEXT CHECK (wait_reason IS NULL OR wait_reason IN ('waiting_owner', 'waiting_children', 'authentication', 'document_event', 'budget', 'timer')),
    admission_class TEXT CHECK (admission_class IS NULL OR admission_class IN ('control', 'work')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    terminal_at_ms INTEGER,
    CHECK (wait_reason IS NULL OR state IN ('queued', 'waiting_durable')),
    CHECK (state NOT IN ('queued', 'admitted', 'running', 'checkpointing', 'reconciling') OR admission_class IS NOT NULL)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_invocations_one_active_per_agent
ON invocations(agent_id) WHERE state NOT IN ('completed', 'failed', 'cancelled');

CREATE TABLE IF NOT EXISTS resource_fences (
    authority_generation TEXT NOT NULL,
    resource_kind TEXT NOT NULL,
    resource_key TEXT NOT NULL,
    last_fence INTEGER NOT NULL CHECK (last_fence >= 0),
    PRIMARY KEY (authority_generation, resource_kind, resource_key)
) STRICT;

CREATE TABLE IF NOT EXISTS resource_leases (
    lease_id TEXT PRIMARY KEY,
    authority_generation TEXT NOT NULL,
    resource_kind TEXT NOT NULL,
    resource_key TEXT NOT NULL,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    boot_identity TEXT NOT NULL,
    ttl_ms INTEGER NOT NULL CHECK (ttl_ms > 0),
    last_heartbeat_seq INTEGER NOT NULL CHECK (last_heartbeat_seq >= 0),
    expires_at_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('held', 'suspect', 'reconciling', 'released', 'revoked')),
    FOREIGN KEY (authority_generation, resource_kind, resource_key)
        REFERENCES resource_fences(authority_generation, resource_kind, resource_key)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_resource_leases_one_active
ON resource_leases(authority_generation, resource_kind, resource_key)
WHERE state IN ('held', 'suspect', 'reconciling');

CREATE TABLE IF NOT EXISTS capacity_pools (
    pool_id TEXT PRIMARY KEY,
    pool_kind TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    unit TEXT NOT NULL,
    requested_limit INTEGER NOT NULL CHECK (requested_limit >= 0),
    effective_limit INTEGER NOT NULL CHECK (effective_limit >= 0),
    admissions_frozen INTEGER NOT NULL CHECK (admissions_frozen IN (0, 1)),
    hard_limit INTEGER NOT NULL CHECK (hard_limit >= 1),
    control_reserve_mode TEXT CHECK (control_reserve_mode IS NULL OR control_reserve_mode IN ('soft', 'hard')),
    control_reserved_units INTEGER,
    control_burst_limit INTEGER,
    checkpoint_escalation_s INTEGER,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (pool_kind, scope_key, unit),
    CHECK (requested_limit <= hard_limit AND effective_limit <= hard_limit),
    CHECK ((pool_kind = 'execution' AND hard_limit = 50 AND requested_limit BETWEEN 1 AND 50 AND effective_limit BETWEEN 1 AND 50 AND control_reserve_mode IS NOT NULL AND control_reserved_units IS NOT NULL AND control_burst_limit >= 1 AND checkpoint_escalation_s BETWEEN 10 AND 600)
        OR (pool_kind != 'execution' AND control_reserve_mode IS NULL AND control_reserved_units IS NULL AND control_burst_limit IS NULL AND checkpoint_escalation_s IS NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS capacity_reservations (
    reservation_id TEXT PRIMARY KEY,
    pool_id TEXT NOT NULL REFERENCES capacity_pools(pool_id),
    owner_type TEXT NOT NULL CHECK (owner_type IN ('agent_invocation', 'replay_worker', 'provider_probe', 'run', 'process', 'terminal', 'browser_context')),
    owner_id TEXT NOT NULL,
    admission_class TEXT CHECK (admission_class IS NULL OR admission_class IN ('control', 'work')),
    amount INTEGER NOT NULL CHECK (amount > 0),
    state TEXT NOT NULL CHECK (state IN ('held', 'reconciling', 'released')),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    created_at_ms INTEGER NOT NULL,
    released_at_ms INTEGER
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_capacity_reservations_active_owner
ON capacity_reservations(pool_id, owner_type, owner_id)
WHERE state IN ('held', 'reconciling');

CREATE TABLE IF NOT EXISTS budget_accounts (
    account_id TEXT PRIMARY KEY,
    scope_type TEXT NOT NULL CHECK (scope_type IN ('global', 'project', 'provider')),
    scope_id TEXT NOT NULL,
    currency TEXT NOT NULL,
    limit_micros INTEGER NOT NULL CHECK (limit_micros >= 0),
    spent_micros INTEGER NOT NULL CHECK (spent_micros >= 0),
    reserved_micros INTEGER NOT NULL CHECK (reserved_micros >= 0),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (scope_type, scope_id, currency)
) STRICT;

CREATE TABLE IF NOT EXISTS invocation_attempts (
    attempt_id TEXT PRIMARY KEY,
    invocation_id TEXT NOT NULL REFERENCES invocations(invocation_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    process_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('admitted', 'running', 'checkpointing', 'reconciling', 'ended', 'failed', 'fenced')),
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    error_code TEXT,
    UNIQUE (invocation_id, ordinal)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_invocation_attempts_one_active
ON invocation_attempts(invocation_id)
WHERE state IN ('admitted', 'running', 'checkpointing', 'reconciling');

CREATE TABLE IF NOT EXISTS actions (
    action_id TEXT PRIMARY KEY,
    origin_kind TEXT NOT NULL CHECK (origin_kind IN ('invocation', 'replay', 'control_plane')),
    control_principal_id TEXT REFERENCES principals(principal_id),
    run_id TEXT REFERENCES workflow_runs(run_id),
    step_id TEXT REFERENCES step_instances(step_id),
    invocation_id TEXT REFERENCES invocations(invocation_id),
    attempt_id TEXT REFERENCES invocation_attempts(attempt_id),
    replay_worker_id TEXT,
    action_type TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    authorization_ref TEXT,
    policy_revision INTEGER NOT NULL CHECK (policy_revision >= 0),
    resource_lease_id TEXT REFERENCES resource_leases(lease_id),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'authorizing', 'approval_required', 'authorized', 'denied', 'dispatching', 'in_flight', 'observed_success', 'observed_failure', 'outcome_unknown', 'cancel_requested', 'cancelled')),
    reconcile_strategy TEXT NOT NULL CHECK (reconcile_strategy IN ('read_only_retryable', 'local_compare_and_set', 'provider_idempotency', 'observe_then_decide', 'manual_reconciliation')),
    provider_idempotency_key TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    CHECK ((origin_kind = 'invocation' AND invocation_id IS NOT NULL AND attempt_id IS NOT NULL AND replay_worker_id IS NULL AND control_principal_id IS NULL)
        OR (origin_kind = 'replay' AND replay_worker_id IS NOT NULL AND run_id IS NOT NULL AND step_id IS NOT NULL AND invocation_id IS NULL AND attempt_id IS NULL AND control_principal_id IS NULL)
        OR (origin_kind = 'control_plane' AND control_principal_id IS NOT NULL AND invocation_id IS NULL AND attempt_id IS NULL AND replay_worker_id IS NULL)),
    CHECK (state NOT IN ('prepared', 'authorizing', 'approval_required') OR authorization_ref IS NULL)
) STRICT;

CREATE TABLE IF NOT EXISTS action_dispatches (
    dispatch_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL REFERENCES actions(action_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    adapter_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    provider_request_id TEXT,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    transport_state TEXT NOT NULL,
    receipt_hash TEXT,
    UNIQUE (action_id, ordinal)
) STRICT;

CREATE TABLE IF NOT EXISTS budget_reservations (
    reservation_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES budget_accounts(account_id),
    action_id TEXT NOT NULL REFERENCES actions(action_id),
    reserved_micros INTEGER NOT NULL CHECK (reserved_micros >= 0),
    settled_micros INTEGER CHECK (settled_micros IS NULL OR settled_micros >= 0),
    state TEXT NOT NULL CHECK (state IN ('held', 'unknown', 'settled', 'released')),
    pricing_version TEXT NOT NULL,
    max_request_hash TEXT NOT NULL,
    UNIQUE (account_id, action_id)
) STRICT;

CREATE TABLE IF NOT EXISTS processes (
    process_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL REFERENCES actions(action_id),
    process_run_uuid TEXT NOT NULL UNIQUE,
    authority_generation TEXT NOT NULL,
    host_id TEXT,
    pid INTEGER CHECK (pid IS NULL OR pid > 0),
    pgid INTEGER CHECK (pgid IS NULL OR pgid > 0),
    boot_identity TEXT,
    start_identity TEXT,
    identity_backend TEXT NOT NULL,
    owner_uid INTEGER NOT NULL CHECK (owner_uid >= 0),
    cwd_ref TEXT NOT NULL,
    worktree_id TEXT REFERENCES worktrees(worktree_id),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'live', 'exited', 'lost', 'suspect', 'identity_mismatch', 'unknown')),
    last_observed_at_ms INTEGER NOT NULL,
    exit_code INTEGER,
    exit_signal INTEGER
) STRICT;

CREATE TABLE IF NOT EXISTS terminal_sessions (
    terminal_id TEXT PRIMARY KEY,
    process_id TEXT NOT NULL REFERENCES processes(process_id),
    owner_agent_id TEXT REFERENCES agents(agent_id),
    mission_id TEXT NOT NULL REFERENCES missions(mission_id),
    purpose_key TEXT NOT NULL,
    rows INTEGER NOT NULL CHECK (rows BETWEEN 1 AND 500),
    cols INTEGER NOT NULL CHECK (cols BETWEEN 1 AND 1000),
    lease_id TEXT NOT NULL REFERENCES resource_leases(lease_id),
    input_seq INTEGER NOT NULL CHECK (input_seq >= 0),
    durable_log_seq INTEGER NOT NULL CHECK (durable_log_seq >= 0),
    state TEXT NOT NULL CHECK (state IN ('preparing', 'ready', 'busy', 'detached', 'stopping', 'exited', 'suspect', 'unavailable'))
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_terminal_sessions_reusable
ON terminal_sessions(mission_id, purpose_key)
WHERE state IN ('preparing', 'ready', 'busy', 'detached');

CREATE TABLE IF NOT EXISTS backup_jobs (
    backup_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK (state IN ('preparing', 'copying', 'verifying', 'complete', 'aborted')),
    source_authority_generation TEXT NOT NULL,
    snapshot_stream_id TEXT,
    snapshot_sequence INTEGER,
    manifest_object_hash TEXT REFERENCES object_records(object_hash),
    coverage TEXT NOT NULL,
    destination_path TEXT,
    retention_tier TEXT NOT NULL DEFAULT 'manual' CHECK (retention_tier IN ('daily', 'weekly', 'manual')),
    created_at_ms INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE TABLE IF NOT EXISTS restore_requests (
    restore_id TEXT PRIMARY KEY,
    command_receipt_id TEXT NOT NULL REFERENCES command_receipts(receipt_id),
    backup_id TEXT NOT NULL,
    source_authority_generation TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('requested', 'neutralizing', 'capsule_ready', 'blocked', 'handed_over', 'aborted')),
    blocked_reason TEXT,
    capsule_object_hash TEXT REFERENCES object_records(object_hash),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK ((state = 'blocked' AND blocked_reason IS NOT NULL AND capsule_object_hash IS NULL) OR (state != 'blocked'))
) STRICT;

CREATE TABLE IF NOT EXISTS restore_imports (
    restore_id TEXT PRIMARY KEY,
    command_receipt_id TEXT NOT NULL,
    backup_id TEXT NOT NULL,
    source_authority_generation TEXT NOT NULL,
    target_authority_generation TEXT NOT NULL,
    capsule_object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    coverage TEXT NOT NULL CHECK (coverage IN ('complete', 'partial', 'unknown')),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'activated')),
    frozen_scopes_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK (target_authority_generation != source_authority_generation)
) STRICT;

CREATE TABLE IF NOT EXISTS recovery_scope_freezes (
    scope_id TEXT PRIMARY KEY,
    restore_id TEXT NOT NULL REFERENCES restore_imports(restore_id),
    reason TEXT NOT NULL,
    global_freeze INTEGER NOT NULL CHECK (global_freeze IN (0, 1)),
    state TEXT NOT NULL CHECK (state IN ('frozen', 'disposed')),
    created_at_ms INTEGER NOT NULL,
    disposed_at_ms INTEGER,
    disposition_receipt_id TEXT REFERENCES command_receipts(receipt_id)
) STRICT;

CREATE TABLE IF NOT EXISTS evidence (
    evidence_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('observation', 'claim', 'judgment')),
    action_id TEXT REFERENCES actions(action_id),
    run_id TEXT REFERENCES workflow_runs(run_id),
    snapshot_ref TEXT,
    object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    producer_id TEXT NOT NULL,
    observed_at_ms INTEGER NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    supersedes_id TEXT REFERENCES evidence(evidence_id),
    availability TEXT NOT NULL CHECK (availability IN ('available', 'purged', 'unavailable'))
) STRICT;

CREATE TRIGGER IF NOT EXISTS trg_evidence_no_claim_to_observation
BEFORE UPDATE ON evidence
FOR EACH ROW
WHEN OLD.kind = 'claim' AND NEW.kind = 'observation'
BEGIN
    SELECT RAISE(ABORT, 'cannot convert claim to observation by update');
END;
";

const V002_SQL: &str = r"
CREATE TABLE IF NOT EXISTS replay_workers (
    replay_worker_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    step_id TEXT NOT NULL REFERENCES step_instances(step_id),
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal >= 1),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    process_id TEXT REFERENCES processes(process_id),
    checkpoint_hash TEXT,
    state TEXT NOT NULL CHECK (state IN ('queued', 'admitted', 'running', 'checkpointing', 'waiting_durable', 'reconciling', 'ended', 'failed', 'fenced')),
    wait_reason TEXT,
    admission_class TEXT CHECK (admission_class IS NULL OR admission_class = 'work'),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK (state NOT IN ('queued', 'admitted', 'running', 'checkpointing', 'reconciling') OR admission_class = 'work')
) STRICT;

CREATE TABLE IF NOT EXISTS provider_profiles (
    profile_version_ref TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    locality TEXT NOT NULL CHECK (locality IN ('local_only', 'approved_remote')),
    pricing_version TEXT NOT NULL,
    price_valid_until_ms INTEGER,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('ovpt_release', 'user_import', 'user_declared')),
    pricing_stale_policy TEXT NOT NULL DEFAULT 'block' CHECK (pricing_stale_policy IN ('block', 'warn')),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS provider_probes (
    probe_id TEXT PRIMARY KEY,
    command_receipt_id TEXT NOT NULL REFERENCES command_receipts(receipt_id),
    principal_id TEXT NOT NULL REFERENCES principals(principal_id),
    source_run_id TEXT REFERENCES workflow_runs(run_id),
    profile_version_ref TEXT NOT NULL REFERENCES provider_profiles(profile_version_ref),
    context_hash TEXT NOT NULL,
    phase TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'admitted', 'running', 'reconciling', 'ended', 'failed', 'cancelled')),
    wait_reason TEXT,
    admission_class TEXT CHECK (admission_class IS NULL OR admission_class = 'work'),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK (state NOT IN ('queued', 'admitted', 'running', 'reconciling') OR admission_class = 'work')
) STRICT;

CREATE TABLE IF NOT EXISTS execution_reservation_owners (
    reservation_id TEXT PRIMARY KEY REFERENCES capacity_reservations(reservation_id),
    invocation_id TEXT REFERENCES invocations(invocation_id),
    attempt_id TEXT REFERENCES invocation_attempts(attempt_id),
    replay_worker_id TEXT REFERENCES replay_workers(replay_worker_id),
    provider_probe_id TEXT REFERENCES provider_probes(probe_id),
    CHECK (((invocation_id IS NOT NULL) + (replay_worker_id IS NOT NULL) + (provider_probe_id IS NOT NULL)) = 1),
    CHECK (attempt_id IS NULL OR invocation_id IS NOT NULL)
) STRICT;

CREATE TABLE IF NOT EXISTS model_calls (
    model_call_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL UNIQUE REFERENCES actions(action_id),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('invocation', 'provider_probe')),
    invocation_id TEXT REFERENCES invocations(invocation_id),
    provider_probe_id TEXT REFERENCES provider_probes(probe_id),
    parent_model_call_id TEXT REFERENCES model_calls(model_call_id),
    trigger_event_id TEXT NOT NULL REFERENCES events(event_id),
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK ((owner_kind = 'invocation' AND invocation_id IS NOT NULL AND provider_probe_id IS NULL)
        OR (owner_kind = 'provider_probe' AND provider_probe_id IS NOT NULL AND invocation_id IS NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS model_call_manifests (
    manifest_id TEXT PRIMARY KEY,
    model_call_id TEXT NOT NULL REFERENCES model_calls(model_call_id),
    manifest_kind TEXT NOT NULL CHECK (manifest_kind IN ('request', 'result')),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    parent_manifest_id TEXT REFERENCES model_call_manifests(manifest_id),
    context_manifest_id TEXT NOT NULL REFERENCES context_manifests(manifest_id),
    provider_id TEXT NOT NULL,
    resolved_model_id TEXT NOT NULL,
    capabilities_hash TEXT NOT NULL,
    allowed_model_policy_hash TEXT NOT NULL,
    request_payload_hash TEXT NOT NULL,
    usage_object_hash TEXT,
    result_object_hash TEXT,
    content_object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (model_call_id, manifest_kind, ordinal)
) STRICT;

CREATE TABLE IF NOT EXISTS workflow_timers (
    timer_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    step_id TEXT REFERENCES step_instances(step_id),
    timer_kind TEXT NOT NULL,
    due_at_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('armed', 'fired', 'cancelled')),
    fired_event_id TEXT REFERENCES events(event_id),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE TABLE IF NOT EXISTS mission_assignments (
    assignment_id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions(mission_id),
    agent_id TEXT NOT NULL REFERENCES agents(agent_id),
    role_version_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    scope_revision INTEGER NOT NULL CHECK (scope_revision >= 0),
    assignment_epoch INTEGER NOT NULL CHECK (assignment_epoch >= 0),
    state TEXT NOT NULL CHECK (state IN ('active', 'superseded', 'released', 'revoked')),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_mission_assignments_one_active
ON mission_assignments(mission_id, agent_id) WHERE state = 'active';

CREATE TABLE IF NOT EXISTS mailbox_items (
    mailbox_item_id TEXT PRIMARY KEY,
    recipient_agent_id TEXT NOT NULL REFERENCES agents(agent_id),
    event_id TEXT NOT NULL REFERENCES events(event_id),
    visibility TEXT NOT NULL,
    wake_rule TEXT NOT NULL,
    coalescing_key TEXT,
    state TEXT NOT NULL CHECK (state IN ('pending', 'delivered', 'processed')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (recipient_agent_id, event_id)
) STRICT;

CREATE TABLE IF NOT EXISTS for_each_cursors (
    step_id TEXT PRIMARY KEY REFERENCES step_instances(step_id),
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    data_digest TEXT NOT NULL,
    item_count INTEGER NOT NULL CHECK (item_count >= 0),
    next_index INTEGER NOT NULL CHECK (next_index >= 0),
    batch_start INTEGER NOT NULL CHECK (batch_start >= 0),
    phase TEXT NOT NULL CHECK (phase IN ('body', 'after_batch')),
    item_results_ref TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK (0 <= batch_start AND batch_start <= next_index AND next_index <= item_count)
) STRICT;

CREATE TABLE IF NOT EXISTS decision_chains (
    decision_chain_id TEXT PRIMARY KEY,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    category TEXT NOT NULL,
    parent_chain_id TEXT REFERENCES decision_chains(decision_chain_id),
    policy_version TEXT NOT NULL,
    consecutive_turns INTEGER NOT NULL CHECK (consecutive_turns >= 0),
    cumulative_turns INTEGER NOT NULL CHECK (cumulative_turns >= 0),
    reserved_turns INTEGER NOT NULL CHECK (reserved_turns >= 0),
    last_progress_event_id TEXT REFERENCES events(event_id),
    state TEXT NOT NULL CHECK (state IN ('open', 'stalled', 'closed')),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_decision_chains_one_open
ON decision_chains(subject_kind, subject_id, category, coalesce(parent_chain_id, ''))
WHERE state IN ('open', 'stalled');

CREATE TABLE IF NOT EXISTS evaluation_reports (
    evaluation_report_id TEXT PRIMARY KEY,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    required_checks TEXT NOT NULL,
    judge_policy TEXT NOT NULL,
    result TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS discovery_receipts (
    discovery_receipt_id TEXT PRIMARY KEY,
    authority_generation TEXT NOT NULL,
    decision TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS registry_entries (
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('skill', 'role', 'contract', 'automation', 'technical_resource')),
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('global', 'project', 'repository')),
    scope_owner_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    PRIMARY KEY (entry_kind, scope_kind, scope_owner_key, entry_id),
    CHECK ((scope_kind = 'global' AND scope_owner_key = 'global') OR (scope_kind != 'global' AND length(scope_owner_key) > 0))
) STRICT;

CREATE TABLE IF NOT EXISTS registry_versions (
    registry_version_id TEXT PRIMARY KEY,
    entry_kind TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_owner_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 1),
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    content_hash TEXT NOT NULL,
    publication_state TEXT NOT NULL CHECK (publication_state IN ('published', 'deprecated', 'revoked')),
    author_principal_id TEXT NOT NULL REFERENCES principals(principal_id),
    discovery_receipt_id TEXT NOT NULL REFERENCES discovery_receipts(discovery_receipt_id),
    evaluation_report_id TEXT NOT NULL REFERENCES evaluation_reports(evaluation_report_id),
    published_at_ms INTEGER NOT NULL,
    UNIQUE (entry_kind, scope_kind, scope_owner_key, entry_id, version),
    FOREIGN KEY (entry_kind, scope_kind, scope_owner_key, entry_id)
        REFERENCES registry_entries(entry_kind, scope_kind, scope_owner_key, entry_id)
) STRICT;

CREATE TABLE IF NOT EXISTS registry_drafts (
    draft_id TEXT PRIMARY KEY,
    entry_kind TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_owner_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    validation_report_id TEXT REFERENCES evaluation_reports(evaluation_report_id),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    FOREIGN KEY (entry_kind, scope_kind, scope_owner_key, entry_id)
        REFERENCES registry_entries(entry_kind, scope_kind, scope_owner_key, entry_id)
) STRICT;

CREATE TABLE IF NOT EXISTS registry_aliases (
    alias_id TEXT PRIMARY KEY,
    entry_kind TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_owner_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('version', 'draft')),
    target_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    FOREIGN KEY (entry_kind, scope_kind, scope_owner_key, entry_id)
        REFERENCES registry_entries(entry_kind, scope_kind, scope_owner_key, entry_id)
) STRICT;

CREATE TABLE IF NOT EXISTS registry_bindings (
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    slot_id TEXT NOT NULL,
    bound_versions_hash TEXT NOT NULL,
    bound_event_id TEXT NOT NULL REFERENCES events(event_id),
    state TEXT NOT NULL,
    PRIMARY KEY (run_id, slot_id)
) STRICT;

CREATE TABLE IF NOT EXISTS source_bindings (
    source_binding_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    kind TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS automation_versions (
    automation_version_id TEXT PRIMARY KEY,
    registry_version_id TEXT NOT NULL REFERENCES registry_versions(registry_version_id),
    parameter_schema_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL REFERENCES object_records(object_hash)
) STRICT;

CREATE TABLE IF NOT EXISTS automation_version_modes (
    automation_version_id TEXT NOT NULL REFERENCES automation_versions(automation_version_id),
    automation_mode TEXT NOT NULL CHECK (automation_mode IN ('replay', 'supervised')),
    workflow_version_id TEXT NOT NULL REFERENCES workflow_versions(workflow_version_id),
    model_policy TEXT NOT NULL CHECK (model_policy IN ('forbidden', 'allowed_bounded')),
    driver_manifest_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    contract_manifest_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    PRIMARY KEY (automation_version_id, automation_mode),
    CHECK ((automation_mode = 'replay' AND model_policy = 'forbidden')
        OR (automation_mode = 'supervised' AND model_policy = 'allowed_bounded'))
) STRICT;

CREATE TABLE IF NOT EXISTS automation_data_revisions (
    data_revision_id TEXT PRIMARY KEY,
    source_binding_id TEXT NOT NULL REFERENCES source_bindings(source_binding_id),
    schema_hash TEXT NOT NULL,
    parser_profile_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    row_count INTEGER NOT NULL CHECK (row_count >= 0),
    validation_receipt_id TEXT NOT NULL REFERENCES command_receipts(receipt_id)
) STRICT;

CREATE TABLE IF NOT EXISTS automation_runs (
    run_id TEXT PRIMARY KEY REFERENCES workflow_runs(run_id),
    automation_version_id TEXT NOT NULL,
    data_revision_id TEXT NOT NULL REFERENCES automation_data_revisions(data_revision_id),
    automation_mode TEXT NOT NULL CHECK (automation_mode IN ('replay', 'supervised')),
    model_policy TEXT NOT NULL CHECK (model_policy IN ('forbidden', 'allowed_bounded')),
    driver_manifest_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    FOREIGN KEY (automation_version_id, automation_mode)
        REFERENCES automation_version_modes(automation_version_id, automation_mode),
    CHECK ((automation_mode = 'replay' AND model_policy = 'forbidden')
        OR (automation_mode = 'supervised' AND model_policy = 'allowed_bounded'))
) STRICT;

CREATE TABLE IF NOT EXISTS guided_setup_drafts (
    draft_id TEXT PRIMARY KEY,
    template_version_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    validation_receipt_id TEXT REFERENCES command_receipts(receipt_id),
    preview_hash TEXT,
    applied_receipt_id TEXT REFERENCES command_receipts(receipt_id)
) STRICT;

CREATE TABLE IF NOT EXISTS model_catalog_snapshots (
    catalog_id TEXT NOT NULL,
    catalog_version TEXT NOT NULL,
    digest TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('ovpt_release', 'user_import', 'user_declared')),
    published_at_ms INTEGER NOT NULL,
    valid_until_ms INTEGER NOT NULL,
    imported_at_ms INTEGER NOT NULL,
    content_ref TEXT NOT NULL,
    PRIMARY KEY (catalog_id, catalog_version)
) STRICT;
";

const V003_SQL: &str = "
ALTER TABLE projects ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE projects ADD COLUMN primary_root_path TEXT;
ALTER TABLE projects ADD COLUMN sources_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE projects ADD COLUMN repositories_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE projects ADD COLUMN settings_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE projects ADD COLUMN updated_at_ms INTEGER NOT NULL DEFAULT 0;

ALTER TABLE actions ADD COLUMN project_id TEXT;
ALTER TABLE actions ADD COLUMN scope_key TEXT;
ALTER TABLE actions ADD COLUMN approval_scope TEXT;
ALTER TABLE actions ADD COLUMN subject_kind TEXT;
ALTER TABLE actions ADD COLUMN subject_id TEXT;
ALTER TABLE actions ADD COLUMN reconciliation_status TEXT;
ALTER TABLE actions ADD COLUMN disposition TEXT;
ALTER TABLE actions ADD COLUMN disposition_reason TEXT;
ALTER TABLE actions ADD COLUMN disposition_evidence_ref TEXT;
ALTER TABLE actions ADD COLUMN disposition_receipt_id TEXT;
ALTER TABLE actions ADD COLUMN disposition_decided_by TEXT;
ALTER TABLE actions ADD COLUMN disposition_decided_at_ms INTEGER;
ALTER TABLE actions ADD COLUMN updated_at_ms INTEGER NOT NULL DEFAULT 0;

ALTER TABLE object_records ADD COLUMN retention_class TEXT NOT NULL DEFAULT 'general';
ALTER TABLE object_records ADD COLUMN keep_by_policy INTEGER NOT NULL DEFAULT 0 CHECK (keep_by_policy IN (0, 1));
ALTER TABLE object_records ADD COLUMN project_id TEXT;

ALTER TABLE restore_imports ADD COLUMN imported_manifest_json TEXT;

CREATE TABLE IF NOT EXISTS settings (
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('user', 'project', 'repository', 'workspace')),
    scope_id TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    execution_pool_json TEXT NOT NULL,
    retention_policy_version TEXT NOT NULL DEFAULT 'unleash.retention.default.v1',
    retention_policy_json TEXT NOT NULL,
    pending_retention_policy_json TEXT,
    pending_effective_at_ms INTEGER,
    analyzer_paths_json TEXT NOT NULL DEFAULT '[]',
    model_selection_bindings_json TEXT NOT NULL DEFAULT '[]',
    extra_typed_json TEXT NOT NULL DEFAULT '{}',
    updated_at_ms INTEGER NOT NULL,
    updated_by_actor TEXT NOT NULL,
    PRIMARY KEY (scope_kind, scope_id)
) STRICT;

CREATE TABLE IF NOT EXISTS security_policies (
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('global', 'project')),
    scope_id TEXT NOT NULL,
    isolation_mode TEXT NOT NULL DEFAULT 'standard' CHECK (isolation_mode IN ('standard', 'strict')),
    policy_version TEXT NOT NULL,
    policy_digest TEXT NOT NULL CHECK (length(policy_digest) >= 16),
    reason TEXT NOT NULL,
    updated_by_actor TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (scope_kind, scope_id)
) STRICT;

CREATE TABLE IF NOT EXISTS secret_references (
    secret_ref_id TEXT PRIMARY KEY,
    opaque_secret_ref TEXT NOT NULL UNIQUE,
    namespace TEXT NOT NULL,
    purpose TEXT NOT NULL,
    secret_kind TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL DEFAULT 'user',
    scope_id TEXT NOT NULL DEFAULT 'default',
    backend_kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending_import', 'active', 'locked', 'revoked_local', 'revoked_remote')),
    local_revoked_at_ms INTEGER,
    remote_revocation_state TEXT NOT NULL DEFAULT 'not_applicable' CHECK (remote_revocation_state IN ('not_applicable', 'unsupported', 'pending', 'confirmed', 'failed')),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS guard_configs (
    config_id TEXT PRIMARY KEY,
    guard_mode TEXT NOT NULL CHECK (guard_mode IN ('broker_only', 'guard_optional', 'guard_required')),
    endpoint TEXT,
    transport TEXT NOT NULL DEFAULT 'none' CHECK (transport IN ('none', 'unix_socket', 'https_tls', 'https_mtls')),
    secret_ref_id TEXT REFERENCES secret_references(secret_ref_id) ON DELETE SET NULL,
    evaluate_timeout_ms INTEGER NOT NULL DEFAULT 2000 CHECK (evaluate_timeout_ms BETWEEN 100 AND 60000),
    health_check_interval_s INTEGER NOT NULL DEFAULT 30 CHECK (health_check_interval_s BETWEEN 5 AND 3600),
    decision_cache_max_age_s INTEGER NOT NULL DEFAULT 300 CHECK (decision_cache_max_age_s BETWEEN 0 AND 3600),
    fallback_policy_ref TEXT,
    guard_mapping_profile TEXT NOT NULL DEFAULT 'unverified',
    health_status TEXT NOT NULL DEFAULT 'unknown' CHECK (health_status IN ('unknown', 'healthy', 'degraded', 'unavailable')),
    negotiated_version TEXT,
    capabilities_json TEXT NOT NULL DEFAULT '[]',
    last_health_check_ms INTEGER,
    reason TEXT NOT NULL DEFAULT '',
    updated_by_actor TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    updated_at_ms INTEGER NOT NULL,
    CHECK (guard_mode != 'guard_optional' OR (fallback_policy_ref IS NOT NULL AND length(trim(fallback_policy_ref)) > 0))
) STRICT;

CREATE TABLE IF NOT EXISTS subject_suspensions (
    suspension_id TEXT PRIMARY KEY,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'suspended', 'revoked')),
    reason TEXT NOT NULL,
    guard_propagation_state TEXT NOT NULL DEFAULT 'local_only' CHECK (guard_propagation_state IN ('local_only', 'propagated', 'unnegotiated_warning')),
    acted_by_actor TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (subject_kind, subject_id, scope_kind, scope_id)
) STRICT;

CREATE TABLE IF NOT EXISTS alerts (
    alert_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'error', 'blocking')),
    state TEXT NOT NULL CHECK (state IN ('active', 'acknowledged', 'resolved', 'superseded')),
    blocking INTEGER NOT NULL DEFAULT 0 CHECK (blocking IN (0, 1)),
    project_id TEXT REFERENCES projects(project_id) ON DELETE CASCADE,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    reason_code TEXT,
    evidence_ref TEXT,
    allowed_operations_json TEXT NOT NULL DEFAULT '[]',
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    resolved_at_ms INTEGER
) STRICT;

CREATE INDEX IF NOT EXISTS idx_alerts_state_blocking
    ON alerts (state, blocking, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_alerts_project
    ON alerts (project_id, state, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS approval_resolutions (
    resolution_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL REFERENCES actions(action_id) ON DELETE CASCADE,
    approval_scope TEXT NOT NULL,
    payload_digest TEXT NOT NULL CHECK (length(payload_digest) = 71 AND substr(payload_digest, 1, 7) = 'sha256:'),
    decision TEXT NOT NULL CHECK (decision IN ('approve', 'deny')),
    reason TEXT NOT NULL,
    decided_by_actor TEXT NOT NULL,
    decider_kind TEXT NOT NULL CHECK (decider_kind IN ('human_operator', 'policy_admin')),
    action_revision_before INTEGER NOT NULL CHECK (action_revision_before >= 0),
    action_revision_after INTEGER NOT NULL CHECK (action_revision_after > action_revision_before),
    command_receipt_id TEXT NOT NULL REFERENCES command_receipts(receipt_id) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS retention_purges (
    purge_id TEXT PRIMARY KEY,
    policy_version TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    requested_by_actor TEXT NOT NULL,
    command_receipt_id TEXT REFERENCES command_receipts(receipt_id) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    requested_older_than_ms INTEGER,
    effective_cutoffs_json TEXT NOT NULL,
    events_tombstoned_count INTEGER NOT NULL CHECK (events_tombstoned_count >= 0),
    receipts_tombstoned_count INTEGER NOT NULL CHECK (receipts_tombstoned_count >= 0),
    objects_purged_count INTEGER NOT NULL CHECK (objects_purged_count >= 0),
    protected_skipped_count INTEGER NOT NULL CHECK (protected_skipped_count >= 0),
    affected_backups_json TEXT NOT NULL DEFAULT '[]',
    limitations_json TEXT NOT NULL DEFAULT '[]',
    wal_checkpoint_truncated INTEGER NOT NULL DEFAULT 0 CHECK (wal_checkpoint_truncated IN (0, 1)),
    executed_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS restore_manifest_records (
    entry_id TEXT PRIMARY KEY,
    imported_by_restore_id TEXT NOT NULL REFERENCES restore_imports(restore_id) ON DELETE CASCADE,
    source_restore_id TEXT NOT NULL,
    command_receipt_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    command_digest TEXT NOT NULL,
    backup_id TEXT NOT NULL,
    source_authority_generation TEXT NOT NULL,
    state TEXT NOT NULL,
    blocked_reason TEXT,
    updated_at_ms INTEGER NOT NULL,
    imported_at_ms INTEGER NOT NULL
) STRICT;
";

const V004_SQL: &str = "
CREATE TABLE recovery_scope_dispositions (
    scope_id TEXT NOT NULL REFERENCES recovery_scope_freezes(scope_id),
    restore_id TEXT NOT NULL REFERENCES restore_imports(restore_id),
    decision_receipt_id TEXT NOT NULL REFERENCES command_receipts(receipt_id),
    decided_by_actor TEXT NOT NULL,
    decision_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (scope_id, restore_id)
) STRICT;
";
