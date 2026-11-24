// SPDX-License-Identifier: Apache-2.0
//! Manifeste privé durable du `process-host` et gestion protégée du secret de contrôle.
//!
//! Invariants (`process-supervision.md`, `REQ-TERM-002`, `REQ-TERM-003`) :
//! - Le secret de contrôle vit dans un fichier privé `0600` séparé (`control.secret`), encapsulé en
//!   mémoire via [`secrecy::SecretString`] / [`zeroize::ZeroizeOnDrop`].
//! - Il n'est JAMAIS transmis en argument CLI (`argv`), écrit dans le manifeste public/privé JSON,
//!   ni affiché dans les logs ou `Debug`/`Display` (`[REDACTED]`).
//! - Toutes les écritures passent par `write_private_file_atomic` (`O_NOFOLLOW`, `0600`, `sync_all`,
//!   garde `.claude/`).

use core::fmt;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unleash_domain::{
    ActionId, AgentId, AuthorityGeneration, DaemonEpoch, Fence, HostId, MissionId, ProcessId,
    ProcessRunUuid, Sha256Digest, TerminalId, WorktreeId,
};
use unleash_platform::{
    BootIdentity, IdentityBackend, StartIdentity, assert_not_under_claude,
    ensure_private_rendezvous_dir, read_private_file_to_string, validate_rendezvous_dir,
    write_private_file_atomic,
};
use zeroize::Zeroize;

use crate::error::ProcessHostError;
use crate::isolation::{IsolationMode, IsolationProfileKind};

/// Nom du fichier de manifeste durable du host (`0600`).
pub const HOST_MANIFEST_FILENAME: &str = "host-manifest.json";

/// Nom du fichier séparé contenant le secret de contrôle (`0600`).
pub const CONTROL_SECRET_FILENAME: &str = "control.secret";

/// Nom du fichier optionnel contenant les secrets canaris pour redaction des logs (`0600`).
pub const CANARY_SECRETS_FILENAME: &str = "canaries.secret";

/// Secret d'authentification partagé entre daemon et process-host pour la lignée/session courante.
///
/// N'expose jamais sa valeur dans `Debug` ou `Display` et zéroïse la mémoire à la destruction.
#[derive(Clone)]
pub struct HostControlSecret {
    inner: SecretString,
}

impl fmt::Debug for HostControlSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HostControlSecret([REDACTED])")
    }
}

impl fmt::Display for HostControlSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl HostControlSecret {
    /// Construit un secret de contrôle validé (non vide, sans octet nul ni retour à la ligne).
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::InvalidArgument`] si le secret est vide ou mal formé.
    pub fn new(raw: impl Into<String>) -> Result<Self, ProcessHostError> {
        let mut s = raw.into();
        if s.trim().is_empty() || s.contains(['\0', '\n', '\r']) {
            s.zeroize();
            return Err(ProcessHostError::InvalidArgument {
                reason: String::from(
                    "le secret de contrôle ne peut pas être vide ni contenir de contrôle",
                ),
            });
        }
        let secret = SecretString::from(s);
        Ok(Self { inner: secret })
    }

    /// Génère un nouveau secret de contrôle aléatoire (256 bits encodés en hexadécimal).
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = random_32_bytes();
        let hex_val = hex::encode(bytes);
        bytes.zeroize();
        Self {
            inner: SecretString::from(hex_val),
        }
    }

    /// Expose le secret brut uniquement pour l'authentification sur canal privé ou persistance `0600`.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.inner.expose_secret()
    }

    /// Compare deux secrets en temps constant via comparaison des empreintes SHA-256.
    #[must_use]
    pub fn constant_time_eq(&self, candidate: &str) -> bool {
        let expected_hash: [u8; 32] = Sha256::digest(self.expose_secret().as_bytes()).into();
        let actual_hash: [u8; 32] = Sha256::digest(candidate.as_bytes()).into();
        let mut diff = 0_u8;
        for (a, b) in expected_hash.iter().zip(actual_hash.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

/// État durable de lancement et de cycle de vie du processus géré par le host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostLaunchState {
    /// Manifeste écrit avant le spawn effectif de l'enfant.
    Prepared,
    /// Enfant démarré et identité OS complète collectée.
    Running,
    /// Enfant terminé et récolté proprement avec statut observé.
    Exited {
        /// Code de sortie si sortie normale.
        exit_code: Option<i32>,
        /// Signal POSIX si terminé par signal.
        exit_signal: Option<i32>,
    },
    /// Coupure pendant le lancement ne permettant pas de confirmer le démarrage.
    LaunchOutcomeUnknown,
    /// Boot machine changé (`boot_identity` différente) : processus réputé disparu.
    Lost,
}

/// Type de ressource détenue par l'instance `unleash-process-host`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceKind {
    /// Terminal PTY interactif géré via `portable-pty`.
    Pty {
        /// Nombre de lignes initial.
        rows: u16,
        /// Nombre de colonnes initial.
        cols: u16,
    },
    /// Service ou commande non PTY avec tubes stdout/stderr dédiés.
    Service,
}

/// Manifeste privé durable écrit atomiquement en `0600` par `unleash-process-host`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostManifest {
    /// Version de schéma du manifeste (vaut `1`).
    pub schema_version: u32,
    /// Identifiant du host.
    pub host_id: HostId,
    /// Identifiant logique du processus.
    pub process_id: ProcessId,
    /// Identifiant de session terminal associée (si PTY).
    pub terminal_id: Option<TerminalId>,
    /// UUID aléatoire unique par lancement physique.
    pub process_run_uuid: ProcessRunUuid,
    /// Lignée d'autorité propriétaire.
    pub authority_generation: AuthorityGeneration,
    /// Plus haute époque daemon acceptée dans cette lignée.
    pub daemon_epoch: DaemonEpoch,
    /// Plus haut fence accepté dans cette époque/lignée.
    pub highest_fence: Fence,
    /// Action ayant autorisé le lancement.
    pub action_id: ActionId,
    /// Mission propriétaire éventuelle.
    pub mission_id: Option<MissionId>,
    /// Agent propriétaire éventuel.
    pub owner_agent_id: Option<AgentId>,
    /// Répertoire de travail canonique du processus enfant.
    pub cwd_canonical: PathBuf,
    /// Worktree éventuel rattaché.
    pub worktree_id: Option<WorktreeId>,
    /// Empreinte SHA-256 canonique (RFC 8785) du programme, des arguments et des clés d'environnement.
    pub command_digest: Sha256Digest,
    /// Environment actually prepared in explicit mode; absent for legacy hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_environment_digest: Option<Sha256Digest>,
    /// Allocation supplied by the authenticated launcher, separate from the applied environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocation_context_digest: Option<Sha256Digest>,
    /// Qualified terminal control state. Missing in legacy manifests; absence
    /// never proves support for retirement or a retired native capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_state: Option<crate::HostControlState>,
    /// UID effectif propriétaire.
    pub owner_uid: u32,
    /// PID observé du processus enfant direct.
    pub pid: Option<u32>,
    /// PGID observé du groupe leader (`setsid`).
    pub pgid: Option<u32>,
    /// Identité de boot OS au moment du lancement.
    pub boot_identity: Option<BootIdentity>,
    /// Identité de démarrage OS unique du processus enfant.
    pub start_identity: Option<StartIdentity>,
    /// Backend OS utilisé pour l'observation d'identité.
    pub identity_backend: IdentityBackend,
    /// Mode d'isolation appliqué (`standard` ou `strict`).
    pub isolation_mode: IsolationMode,
    /// Backend d'isolation effectif appliqué (ex. `macos-seatbelt`).
    pub isolation_backend: Option<String>,
    /// Empreinte SHA-256 du profil de confinement appliqué en mode `strict`.
    pub isolation_profile_digest: Option<Sha256Digest>,
    /// Chemin du socket Unix privé de contrôle du host.
    pub socket_path: PathBuf,
    /// Chemin du répertoire de spool de logs.
    pub spool_dir: PathBuf,
    /// Chemin du fichier `0600` séparé portant le secret de contrôle (jamais la valeur du secret).
    pub control_secret_path: PathBuf,
    /// État durable de lancement / sortie.
    pub launch_state: HostLaunchState,
    /// Dernière séquence d'entrée PTY confirmée par reçu durable.
    pub last_confirmed_input_seq: u64,
    /// Séquence d'entrée en vol entre écriture PTY et reçu durable (`OutcomeUnknown` à la reprise).
    pub in_flight_input_seq: Option<u64>,
    /// Plus haut offset/séquence durable committé dans le spool de logs.
    pub durable_log_seq: u64,
}

impl HostManifest {
    /// Écrit atomiquement ce manifeste en `0600` (sans lien symbolique, hors `.claude/`).
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si le répertoire parent n'est pas `0700` ou en cas d'I/O.
    pub fn save_atomic(&self, path: &Path) -> Result<(), ProcessHostError> {
        assert_not_under_claude(path)?;
        if let Some(parent) = path.parent() {
            ensure_private_rendezvous_dir(parent)?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        write_private_file_atomic(path, &json)?;
        Ok(())
    }

    /// Lit et valide le manifeste depuis `path` en vérifiant permissions `0600`, propriétaire et parent `0700`.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si le fichier est absent, malformé, symlink ou trop permissif.
    pub fn load_private(path: &Path) -> Result<Self, ProcessHostError> {
        assert_not_under_claude(path)?;
        if let Some(parent) = path.parent() {
            validate_rendezvous_dir(parent)?;
        }
        let meta = fs::symlink_metadata(path)?;
        if (meta.permissions().mode() & 0o7777) != 0o600 {
            return Err(ProcessHostError::Platform(
                unleash_platform::PlatformError::InsecurePermissions {
                    path: path.display().to_string(),
                    expected_mode: 0o600,
                    actual_mode: meta.permissions().mode() & 0o7777,
                },
            ));
        }
        let raw = read_private_file_to_string(path)?.ok_or_else(|| {
            ProcessHostError::TerminalUnavailable {
                reason: format!("manifeste host absent : '{}'", path.display()),
            }
        })?;
        let parsed: Self = serde_json::from_str(&raw)?;
        Ok(parsed)
    }
}

/// Écrit un secret de contrôle de façon atomique en `0600` dans un répertoire `0700`.
///
/// # Errors
/// Retourne [`ProcessHostError`] si les permissions ou écritures échouent.
pub fn store_control_secret(
    path: &Path,
    secret: &HostControlSecret,
) -> Result<(), ProcessHostError> {
    assert_not_under_claude(path)?;
    if let Some(parent) = path.parent() {
        ensure_private_rendezvous_dir(parent)?;
    }
    write_private_file_atomic(path, secret.expose_secret().as_bytes())?;
    Ok(())
}

/// Charge le secret de contrôle depuis `path` en vérifiant strictement les droits `0700`/`0600`.
///
/// # Errors
/// Retourne [`ProcessHostError`] si absent, symlink ou permissions non conformes.
pub fn load_control_secret(path: &Path) -> Result<HostControlSecret, ProcessHostError> {
    assert_not_under_claude(path)?;
    if let Some(parent) = path.parent() {
        validate_rendezvous_dir(parent)?;
    }
    let meta = fs::symlink_metadata(path)?;
    if (meta.permissions().mode() & 0o7777) != 0o600 {
        return Err(ProcessHostError::Platform(
            unleash_platform::PlatformError::InsecurePermissions {
                path: path.display().to_string(),
                expected_mode: 0o600,
                actual_mode: meta.permissions().mode() & 0o7777,
            },
        ));
    }
    let raw = read_private_file_to_string(path)?.ok_or_else(|| {
        ProcessHostError::AuthenticationFailed {
            reason: String::from("fichier secret de contrôle introuvable"),
        }
    })?;
    HostControlSecret::new(raw)
}

/// Qualified explicit environment format, distinct from the legacy inherited baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentMode {
    /// Only configured finite locale values and the private managed TMPDIR.
    ExplicitV1,
    /// Exact private environment for a strictly confined, egress-denied macOS service.
    StrictServiceV1,
}

/// Build the exact explicit environment without consulting the parent environment.
/// # Errors
/// Refuses unknown values or an invalid managed temporary directory.
pub fn explicit_environment_v1(
    explicit: &BTreeMap<String, String>,
    child_tmp: &Path,
) -> Result<BTreeMap<String, String>, ProcessHostError> {
    let bad = || ProcessHostError::InvalidArgument {
        reason: "invalid explicit environment".into(),
    };
    if !child_tmp.is_absolute() || explicit.is_empty() || explicit.len() > 4 {
        return Err(bad());
    }
    for (k, v) in explicit {
        let allowed = match k.as_str() {
            "LANG" | "LC_ALL" => {
                ["C", "C.UTF-8", "en_US.UTF-8", "fr_FR.UTF-8"].contains(&v.as_str())
            }
            "TERM" => ["xterm", "xterm-256color", "vt100", "dumb"].contains(&v.as_str()),
            "TZ" => v == "UTC",
            _ => false,
        };
        if !allowed {
            return Err(bad());
        }
    }
    let mut result = explicit.clone();
    result.insert(
        "TMPDIR".into(),
        child_tmp.to_str().ok_or_else(bad)?.to_owned(),
    );
    Ok(result)
}

/// Digest of the complete effective, non-secret environment under a versioned convention.
/// # Errors
/// Returns a canonical serialization error.
pub fn effective_environment_digest_v1(
    environment: &BTreeMap<String, String>,
) -> Result<Sha256Digest, ProcessHostError> {
    Ok(unleash_protocol::canonical_sha256(
        &serde_json::json!({"version":1,"environment":environment}),
    )?)
}

/// Spécification de démarrage lue depuis le fichier privé `--bootstrap` par le binaire `unleash-process-host`.
///
/// Aucun secret de contrôle ni canari ne transite par la ligne de commande (`argv`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostBootstrapSpec {
    /// Absent preserves legacy behavior; configured terminals never fall back to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_mode: Option<EnvironmentMode>,
    /// Expected digest checked before a configured child can be started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_effective_environment_digest: Option<Sha256Digest>,
    /// Immutable allocation supplied by the daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocation_context_digest: Option<Sha256Digest>,
    /// Répertoire privé `0700` du host (manifeste, socket, journal, spool).
    pub runtime_dir: PathBuf,
    /// Identifiant du host.
    pub host_id: HostId,
    /// Identifiant logique du processus.
    pub process_id: ProcessId,
    /// Identifiant du terminal (si PTY).
    pub terminal_id: Option<TerminalId>,
    /// UUID unique de cette exécution physique.
    pub process_run_uuid: ProcessRunUuid,
    /// Lignée d'autorité d'origine.
    pub authority_generation: AuthorityGeneration,
    /// Époque initiale du daemon.
    pub daemon_epoch: DaemonEpoch,
    /// Fence initial.
    pub resource_fence: Fence,
    /// Action d'autorisation.
    pub action_id: ActionId,
    /// Mission propriétaire éventuelle.
    pub mission_id: Option<MissionId>,
    /// Agent propriétaire éventuel.
    pub owner_agent_id: Option<AgentId>,
    /// Worktree éventuel.
    pub worktree_id: Option<WorktreeId>,
    /// Répertoire de travail du processus enfant.
    pub cwd: PathBuf,
    /// Programme à exécuter (sans interpolation shell).
    pub program: String,
    /// Arguments exacts.
    pub arguments: Vec<String>,
    /// Variables d'environnement explicites non secrètes.
    pub explicit_env: BTreeMap<String, String>,
    /// Nature de ressource (PTY ou service).
    pub resource_kind: ResourceKind,
    /// Mode d'isolation demandé (`standard` ou `strict`).
    pub isolation_mode: IsolationMode,
    /// Spécification stricte si `isolation_mode == Strict`.
    pub strict_spec: Option<crate::isolation::StrictIsolationSpec>,
    /// Politique de profil (informative si besoin).
    pub isolation_profile_kind: Option<IsolationProfileKind>,
    /// Chemin du fichier `0600` portant le secret de contrôle initial.
    pub control_secret_path: PathBuf,
    /// Chemin optionnel du fichier `0600` contenant la liste JSON des canaris à masquer dans les logs.
    pub canaries_secret_path: Option<PathBuf>,
    /// Taille maximale par segment de log dans le spool (en octets).
    pub max_segment_bytes: usize,
    /// Quota total de logs conservés sur disque (en octets).
    pub max_total_log_bytes: usize,
    /// Plafond du tampon mémoire volatil (en octets).
    pub max_volatile_bytes: usize,
}

impl HostBootstrapSpec {
    /// Calcule le digest RFC 8785 + SHA-256 de la commande et des clés d'environnement.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::Protocol`] si la canonisation JCS échoue.
    pub fn compute_command_digest(&self) -> Result<Sha256Digest, ProcessHostError> {
        #[derive(Serialize)]
        struct DigestView<'a> {
            arguments: &'a [String],
            cwd: String,
            env_keys: Vec<&'a str>,
            program: &'a str,
        }
        let view = DigestView {
            arguments: &self.arguments,
            cwd: self.cwd.display().to_string(),
            env_keys: self.explicit_env.keys().map(String::as_str).collect(),
            program: &self.program,
        };
        Ok(unleash_protocol::canonical_sha256(&view)?)
    }
}

/// Charge la liste des canaris secrets à masquer depuis un fichier privé `0600` (sans log).
///
/// # Errors
/// Retourne [`ProcessHostError`] en cas de permissions invalides ou JSON corrompu.
pub fn load_canary_secrets(path: &Path) -> Result<Vec<SecretString>, ProcessHostError> {
    assert_not_under_claude(path)?;
    if let Some(parent) = path.parent() {
        validate_rendezvous_dir(parent)?;
    }
    let meta = fs::symlink_metadata(path)?;
    if (meta.permissions().mode() & 0o7777) != 0o600 {
        return Err(ProcessHostError::Platform(
            unleash_platform::PlatformError::InsecurePermissions {
                path: path.display().to_string(),
                expected_mode: 0o600,
                actual_mode: meta.permissions().mode() & 0o7777,
            },
        ));
    }
    let Some(mut raw) = read_private_file_to_string(path)? else {
        return Ok(Vec::new());
    };
    let parsed: Vec<String> = serde_json::from_str(&raw)?;
    raw.zeroize();
    Ok(parsed
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(SecretString::from)
        .collect())
}

/// Écrit les canaris secrets dans un fichier `0600` séparé pour consommation par le host.
///
/// # Errors
/// Retourne [`ProcessHostError`] si le chemin ou les permissions échouent.
pub fn store_canary_secrets(path: &Path, canaries: &[&str]) -> Result<(), ProcessHostError> {
    assert_not_under_claude(path)?;
    if let Some(parent) = path.parent() {
        ensure_private_rendezvous_dir(parent)?;
    }
    let mut json = serde_json::to_vec(canaries)?;
    let res = write_private_file_atomic(path, &json);
    json.zeroize();
    res?;
    Ok(())
}

/// Génère 32 octets aléatoires depuis `/dev/urandom` (avec repli horloge/compteur/SHA-256).
pub(crate) fn random_32_bytes() -> [u8; 32] {
    use std::io::Read;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(1);

    let mut buf = [0_u8; 32];
    if let Ok(mut f) = fs::File::open("/dev/urandom")
        && f.read_exact(&mut buf).is_ok()
    {
        return buf;
    }

    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(COUNTER.fetch_add(1, Ordering::SeqCst).to_le_bytes());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    hasher.update(nanos.to_le_bytes());
    hasher.finalize().into()
}

/// Génère un UUID v4 à partir d'entropie OS (`/dev/urandom`).
pub(crate) fn new_random_uuid() -> uuid::Uuid {
    let r = random_32_bytes();
    let mut b = [0_u8; 16];
    b.copy_from_slice(&r[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unleash_testkit::local_test_dir;

    #[test]
    fn unavailable_random_source_is_an_error_never_a_fallback() {
        // Une source d'aléa indisponible ne produit aucun octet : un hachage de pid, de compteur
        // et d'horloge serait un secret de contrôle calculable.
        assert!(matches!(
            random_32_bytes_with(|_| Err(ProcessHostError::RandomSourceUnavailable)),
            Err(ProcessHostError::RandomSourceUnavailable)
        ));
        assert!(random_32_bytes().is_ok_and(|bytes| bytes.len() == 32));
    }

    #[test]
    fn control_secret_redacted_in_debug_and_display_and_ct_eq() {
        let secret = HostControlSecret::new("super-secret-control-token-42").unwrap();
        let dbg_str = format!("{secret:?}");
        let disp_str = format!("{secret}");
        assert!(!dbg_str.contains("super-secret"));
        assert!(!disp_str.contains("super-secret"));
        assert!(dbg_str.contains("[REDACTED]"));
        assert!(secret.constant_time_eq("super-secret-control-token-42"));
        assert!(!secret.constant_time_eq("super-secret-control-token-43"));
        assert!(!secret.constant_time_eq("short"));
    }

    #[test]
    fn manifest_and_secret_are_stored_0600_and_never_contain_raw_secret() {
        let dir = local_test_dir("ph-man").unwrap();
        let rdv = dir.path().join("host-1");
        ensure_private_rendezvous_dir(&rdv).unwrap();
        let secret_path = rdv.join(CONTROL_SECRET_FILENAME);
        let manifest_path = rdv.join(HOST_MANIFEST_FILENAME);

        let secret = HostControlSecret::new("canary-ctrl-secret-xyz").unwrap();
        store_control_secret(&secret_path, &secret).unwrap();

        let manifest = HostManifest {
            effective_environment_digest: None,
            allocation_context_digest: None,
            control_state: None,
            schema_version: 1,
            host_id: HostId::from_u128(1),
            process_id: ProcessId::from_u128(2),
            terminal_id: Some(TerminalId::from_u128(3)),
            process_run_uuid: ProcessRunUuid::from_u128(4),
            authority_generation: AuthorityGeneration::from_u128(5),
            daemon_epoch: DaemonEpoch::new(1),
            highest_fence: Fence::new(1),
            action_id: ActionId::from_u128(6),
            mission_id: Some(MissionId::from_u128(7)),
            owner_agent_id: None,
            cwd_canonical: dir.path().to_path_buf(),
            worktree_id: None,
            command_digest: Sha256Digest::from_bytes(b"cmd"),
            owner_uid: unleash_platform::effective_uid(),
            pid: Some(12345),
            pgid: Some(12345),
            boot_identity: Some(BootIdentity::new("boot-1").unwrap()),
            start_identity: Some(StartIdentity::new("start-1").unwrap()),
            identity_backend: IdentityBackend::MacosProcPidinfo,
            isolation_mode: IsolationMode::Standard,
            isolation_backend: None,
            isolation_profile_digest: None,
            socket_path: rdv.join("host.sock"),
            spool_dir: rdv.join("spool"),
            control_secret_path: secret_path.clone(),
            launch_state: HostLaunchState::Running,
            last_confirmed_input_seq: 0,
            in_flight_input_seq: None,
            durable_log_seq: 0,
        };
        manifest.save_atomic(&manifest_path).unwrap();

        let raw_manifest = fs::read_to_string(&manifest_path).unwrap();
        assert!(!raw_manifest.contains("canary-ctrl-secret-xyz"));
        let loaded = HostManifest::load_private(&manifest_path).unwrap();
        assert_eq!(loaded, manifest);
        let loaded_secret = load_control_secret(&secret_path).unwrap();
        assert!(loaded_secret.constant_time_eq("canary-ctrl-secret-xyz"));
    }
}
