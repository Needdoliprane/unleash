//! Qualification du build natif `SQLite` et profils PRAGMA vérifiés (ADR-01, REQ-PERSIST-002).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use unleash_domain::TimestampMs;

use crate::error::StoreError;

/// Version numérique minimale qualifiée incluant le correctif WAL-reset (`3.51.3`).
pub const MIN_QUALIFIED_SQLITE_VERSION: (u32, u32, u32) = (3, 51, 3);

/// Versions backportées approuvées contenant le correctif WAL-reset (SYS-13).
pub const APPROVED_BACKPORT_VERSIONS: &[&str] = &["3.44.6", "3.50.7"];

/// Versions officiellement retirées par `SQLite` et interdites (SYS-13).
pub const WITHDRAWN_SQLITE_VERSIONS: &[&str] = &["3.52.0"];

/// Politique de validation du build `SQLite` au démarrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteBuildPolicy {
    /// `sqlite_source_id()` attendu issu de la compilation embarquée (ou surchargé en test).
    pub expected_source_id: String,
    /// Exiger l'égalité exacte entre `sqlite_source_id()` runtime et `expected_source_id`.
    pub enforce_source_id_match: bool,
    /// Backports supplémentaires explicitement approuvés (`(version, source_id)`).
    pub extra_approved_backports: Vec<(String, String)>,
}

impl Default for SqliteBuildPolicy {
    fn default() -> Self {
        Self {
            expected_source_id: compiled_sqlite_source_id().to_owned(),
            enforce_source_id_match: true,
            extra_approved_backports: Vec::new(),
        }
    }
}

/// Renvoie `SQLITE_VERSION` issu des en-têtes embarqués compilés.
#[must_use]
pub fn compiled_sqlite_version() -> &'static str {
    rusqlite::version()
}

/// Renvoie `SQLITE_SOURCE_ID` issu des en-têtes embarqués compilés (`libsqlite3-sys`), sans `unsafe`.
#[must_use]
pub fn compiled_sqlite_source_id() -> &'static str {
    rusqlite::ffi::SQLITE_SOURCE_ID.to_str().unwrap_or("")
}

/// Profil effectif des PRAGMAs observé sur la connexion writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectivePragmaProfile {
    /// Mode de journalisation (`wal`).
    pub journal_mode: String,
    /// Niveau `synchronous` (`2` = `FULL`).
    pub synchronous: i64,
    /// Activation des clés étrangères (`1`).
    pub foreign_keys: i64,
    /// Désactivation du schéma de confiance (`0`).
    pub trusted_schema: i64,
    /// Effacement sécurisé (`2` = `FAST`).
    pub secure_delete: i64,
    /// Délai de contention `busy_timeout` en ms (`250`).
    pub busy_timeout_ms: i64,
    /// Autocheckpoint désactivé (`0`).
    pub wal_autocheckpoint: i64,
    /// `fullfsync` actif sur macOS (`1` sur macOS).
    pub fullfsync: i64,
    /// `checkpoint_fullfsync` actif sur macOS (`1` sur macOS).
    pub checkpoint_fullfsync: i64,
    /// Taille de page `SQLite` observée en octets.
    pub page_size: i64,
}

/// Manifeste de qualification runtime enregistré au démarrage de `unleash-store`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqliteRuntimeManifest {
    /// Version `SQLite` observée par `SELECT sqlite_version()`.
    pub sqlite_version: String,
    /// Source ID `SQLite` observé par `SELECT sqlite_source_id()`.
    pub sqlite_source_id: String,
    /// Source ID compilé attendu.
    pub expected_source_id: String,
    /// Options de compilation observées via `PRAGMA compile_options`.
    pub compile_options: Vec<String>,
    /// Profil effectif des PRAGMAs sur la connexion writer.
    pub effective_pragmas: EffectivePragmaProfile,
    /// Horodatage UTC d'enregistrement du manifeste.
    pub qualified_at_ms: TimestampMs,
}

/// Vérifie la qualification d'un couple `(sqlite_version, sqlite_source_id, compile_options)`
/// par rapport à `policy`.
///
/// # Errors
/// Retourne [`StoreError::BuildUnqualified`] (`STORAGE_BUILD_UNQUALIFIED`) si la version est
/// retirée (`3.52.0`), non corrigée pour le bug WAL-reset, sans option `THREADSAFE` / WAL,
/// ou si `sqlite_source_id()` diverge de la valeur compilée attendue.
pub fn validate_build_metadata(
    runtime_version: &str,
    runtime_source_id: &str,
    compile_options: &[String],
    policy: &SqliteBuildPolicy,
) -> Result<(), StoreError> {
    if policy.enforce_source_id_match && runtime_source_id != policy.expected_source_id {
        return Err(StoreError::BuildUnqualified {
            detail: format!(
                "sqlite_source_id() divergent : attendu '{}', observé '{}'",
                policy.expected_source_id, runtime_source_id
            ),
        });
    }

    if WITHDRAWN_SQLITE_VERSIONS.contains(&runtime_version) {
        return Err(StoreError::BuildUnqualified {
            detail: format!("version SQLite '{runtime_version}' officiellement retirée"),
        });
    }

    if compile_options.iter().any(|opt| opt == "OMIT_WAL") {
        return Err(StoreError::BuildUnqualified {
            detail: "SQLite compilée avec OMIT_WAL".to_owned(),
        });
    }

    if compile_options.iter().any(|opt| opt == "THREADSAFE=0") {
        return Err(StoreError::BuildUnqualified {
            detail: "SQLite compilée avec THREADSAFE=0".to_owned(),
        });
    }

    let is_extra_backport = policy
        .extra_approved_backports
        .iter()
        .any(|(ver, sid)| ver == runtime_version && sid == runtime_source_id);

    if is_extra_backport || APPROVED_BACKPORT_VERSIONS.contains(&runtime_version) {
        return Ok(());
    }

    let parsed =
        parse_semver_triple(runtime_version).ok_or_else(|| StoreError::BuildUnqualified {
            detail: format!("version SQLite non reconnue '{runtime_version}'"),
        })?;

    if parsed < MIN_QUALIFIED_SQLITE_VERSION {
        return Err(StoreError::BuildUnqualified {
            detail: format!(
                "version SQLite '{runtime_version}' antérieure au correctif WAL-reset 3.51.3 et absente des backports approuvés"
            ),
        });
    }

    Ok(())
}

/// Inspecte la connexion `SQLite`, valide son build et renvoie version, `source_id` et options.
///
/// # Errors
/// Retourne [`StoreError::BuildUnqualified`] en cas de non-conformité ou [`StoreError`] sur erreur `SQLite`.
pub fn inspect_and_verify_build(
    conn: &Connection,
    policy: &SqliteBuildPolicy,
) -> Result<(String, String, Vec<String>), StoreError> {
    let (version, source_id): (String, String) = conn
        .query_row("SELECT sqlite_version(), sqlite_source_id()", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(StoreError::from_rusqlite)?;

    let mut stmt = conn
        .prepare("PRAGMA compile_options")
        .map_err(StoreError::from_rusqlite)?;
    let opts_iter = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(StoreError::from_rusqlite)?;
    let mut compile_options = Vec::new();
    for item in opts_iter {
        compile_options.push(item.map_err(StoreError::from_rusqlite)?);
    }

    validate_build_metadata(&version, &source_id, &compile_options, policy)?;
    Ok((version, source_id, compile_options))
}

/// Applique le profil PRAGMA complet de la connexion writer unique et vérifie les valeurs effectives.
///
/// # Errors
/// Retourne [`StoreError`] si un PRAGMA échoue ou n'atteint pas la valeur attendue.
pub fn apply_and_verify_writer_pragmas(
    conn: &Connection,
    busy_timeout_ms: u32,
) -> Result<EffectivePragmaProfile, StoreError> {
    conn.pragma_update(None, "busy_timeout", i64::from(busy_timeout_ms))
        .map_err(StoreError::from_rusqlite)?;

    let journal_mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
        .map_err(StoreError::from_rusqlite)?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::InvariantViolation(format!(
            "PRAGMA journal_mode=WAL refusé (observé '{journal_mode}')"
        )));
    }

    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(StoreError::from_rusqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::from_rusqlite)?;
    conn.pragma_update(None, "trusted_schema", "OFF")
        .map_err(StoreError::from_rusqlite)?;
    conn.pragma_update(None, "secure_delete", "FAST")
        .map_err(StoreError::from_rusqlite)?;
    conn.pragma_update(None, "wal_autocheckpoint", 0_i64)
        .map_err(StoreError::from_rusqlite)?;

    #[cfg(target_os = "macos")]
    {
        conn.pragma_update(None, "fullfsync", "ON")
            .map_err(StoreError::from_rusqlite)?;
        conn.pragma_update(None, "checkpoint_fullfsync", "ON")
            .map_err(StoreError::from_rusqlite)?;
    }

    let synchronous: i64 = conn
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let foreign_keys: i64 = conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let trusted_schema: i64 = conn
        .pragma_query_value(None, "trusted_schema", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let secure_delete: i64 = conn
        .pragma_query_value(None, "secure_delete", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let effective_busy_timeout: i64 = conn
        .pragma_query_value(None, "busy_timeout", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let wal_autocheckpoint: i64 = conn
        .pragma_query_value(None, "wal_autocheckpoint", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let fullfsync: i64 = conn
        .pragma_query_value(None, "fullfsync", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let checkpoint_fullfsync: i64 = conn
        .pragma_query_value(None, "checkpoint_fullfsync", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let page_size: i64 = conn
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;

    let expected_busy = i64::from(busy_timeout_ms);
    if synchronous != 2
        || foreign_keys != 1
        || trusted_schema != 0
        || secure_delete != 2
        || effective_busy_timeout != expected_busy
        || wal_autocheckpoint != 0
    {
        return Err(StoreError::InvariantViolation(format!(
            "PRAGMA writer non conforme : synchronous={synchronous}, foreign_keys={foreign_keys}, trusted_schema={trusted_schema}, secure_delete={secure_delete}, busy_timeout={effective_busy_timeout} (attendu {expected_busy}), wal_autocheckpoint={wal_autocheckpoint}"
        )));
    }
    #[cfg(target_os = "macos")]
    if fullfsync != 1 || checkpoint_fullfsync != 1 {
        return Err(StoreError::InvariantViolation(format!(
            "PRAGMA macOS fullfsync requis : fullfsync={fullfsync}, checkpoint_fullfsync={checkpoint_fullfsync}"
        )));
    }

    Ok(EffectivePragmaProfile {
        journal_mode: journal_mode.to_lowercase(),
        synchronous,
        foreign_keys,
        trusted_schema,
        secure_delete,
        busy_timeout_ms: effective_busy_timeout,
        wal_autocheckpoint,
        fullfsync,
        checkpoint_fullfsync,
        page_size,
    })
}

/// Applique le profil PRAGMA d'une connexion lectrice (`query_only=ON`).
///
/// # Errors
/// Retourne [`StoreError`] si la configuration en lecture seule échoue.
pub fn apply_and_verify_reader_pragmas(
    conn: &Connection,
    busy_timeout_ms: u32,
) -> Result<(), StoreError> {
    conn.pragma_update(None, "busy_timeout", i64::from(busy_timeout_ms))
        .map_err(StoreError::from_rusqlite)?;
    conn.pragma_update(None, "query_only", "ON")
        .map_err(StoreError::from_rusqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::from_rusqlite)?;
    conn.pragma_update(None, "trusted_schema", "OFF")
        .map_err(StoreError::from_rusqlite)?;

    let query_only: i64 = conn
        .pragma_query_value(None, "query_only", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let foreign_keys: i64 = conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let trusted_schema: i64 = conn
        .pragma_query_value(None, "trusted_schema", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let effective_busy: i64 = conn
        .pragma_query_value(None, "busy_timeout", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let expected_busy = i64::from(busy_timeout_ms);
    if query_only != 1
        || foreign_keys != 1
        || trusted_schema != 0
        || effective_busy != expected_busy
    {
        return Err(StoreError::InvariantViolation(format!(
            "PRAGMA lecteur non conforme : query_only={query_only}, foreign_keys={foreign_keys}, trusted_schema={trusted_schema}, busy_timeout={effective_busy} (attendu {expected_busy})"
        )));
    }
    Ok(())
}

fn parse_semver_triple(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch_raw = parts.next()?;
    let patch_digits: String = patch_raw.chars().take_while(char::is_ascii_digit).collect();
    let patch = patch_digits.parse::<u32>().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_writer_and_reader_pragmas_verify_all_checked_fields() {
        let dir = unleash_testkit::local_test_dir("qual-pragma").unwrap();
        let db = dir.path().join("pragma.sqlite");
        let conn = Connection::open(&db).unwrap();
        let profile = apply_and_verify_writer_pragmas(&conn, 250).unwrap();
        assert_eq!(profile.secure_delete, 2);
        assert_eq!(profile.busy_timeout_ms, 250);
        assert_eq!(profile.synchronous, 2);
        assert_eq!(profile.foreign_keys, 1);
        assert_eq!(profile.trusted_schema, 0);
        assert_eq!(profile.wal_autocheckpoint, 0);

        let ro = Connection::open(&db).unwrap();
        apply_and_verify_reader_pragmas(&ro, 250).unwrap();
    }
}
