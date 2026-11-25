// SPDX-License-Identifier: Apache-2.0
//! Résolution des chemins canoniques `ADR-14` (macOS/Linux), identifiant UID effectif,
//! chemin de rendez-vous canonique et garde stricte interdisant `.claude/`.

use std::ffi::CStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::errors::PlatformError;
use crate::rendezvous::MAX_UNIX_SOCKET_PATH_BYTES;

/// Nom du dossier documentaire interdit à toute écriture runtime (`REQ-CORE-008`, `INV-046`).
pub const CLAUDE_DIR_NAME: &str = ".claude";

/// Plateforme cible supportée pour la résolution des emplacements `ADR-14`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformOsKind {
    /// macOS 15+ (Apple Silicon).
    MacOs,
    /// Linux (Ubuntu LTS `x86_64`).
    Linux,
}

impl PlatformOsKind {
    /// Détecte la plateforme courante à la compilation.
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self::Linux
        }
    }
}

/// Instantané injectable des variables d'environnement pertinentes pour `ADR-14`.
///
/// Permet de tester la résolution des chemins et l'immunité de l'autorité à `XDG_RUNTIME_DIR`
/// et `TMPDIR` sans aucune mutation globale de `std::env` (édition Rust 2024).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    /// Répertoire personnel (`HOME`).
    pub home: Option<PathBuf>,
    /// Surcharge `$XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<String>,
    /// Surcharge `$XDG_DATA_HOME`.
    pub xdg_data_home: Option<String>,
    /// Surcharge `$XDG_STATE_HOME`.
    pub xdg_state_home: Option<String>,
    /// Surcharge `$XDG_CACHE_HOME`.
    pub xdg_cache_home: Option<String>,
    /// Surcharge `$XDG_RUNTIME_DIR` (jamais suivie pour le verrou d'autorité).
    pub xdg_runtime_dir: Option<String>,
    /// Surcharge `$TMPDIR` (jamais suivie pour le verrou d'autorité).
    pub tmpdir: Option<String>,
}

impl EnvSnapshot {
    /// Capture l'environnement courant du processus.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            home: std::env::var_os("HOME").map(PathBuf::from),
            xdg_config_home: std::env::var("XDG_CONFIG_HOME").ok(),
            xdg_data_home: std::env::var("XDG_DATA_HOME").ok(),
            xdg_state_home: std::env::var("XDG_STATE_HOME").ok(),
            xdg_cache_home: std::env::var("XDG_CACHE_HOME").ok(),
            xdg_runtime_dir: std::env::var("XDG_RUNTIME_DIR").ok(),
            tmpdir: std::env::var("TMPDIR").ok(),
        }
    }
}

/// Arborescence séparée des emplacements applicatifs conforme à `ADR-14` et `runtime-lifecycle.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformPaths {
    /// Configuration validée (`config/`).
    pub config_dir: PathBuf,
    /// Racine des données durables et `SQLite` (`data/`).
    pub data_dir: PathBuf,
    /// Bases dérivées reconstruisibles (`<data_dir>/index/`).
    pub index_dir: PathBuf,
    /// Objets immuables adressés par hash (`<data_dir>/artifacts/`).
    pub artifacts_dir: PathBuf,
    /// Pièces de preuves (`<data_dir>/evidence/`).
    pub evidence_dir: PathBuf,
    /// Sauvegardes complètes (`<data_dir>/backups/`).
    pub backups_dir: PathBuf,
    /// Runtimes navigateur épinglés (`<data_dir>/runtimes/browser/`).
    pub browser_runtimes_dir: PathBuf,
    /// Emplacement par défaut des worktrees Git gérés (`<data_dir>/worktrees/`).
    pub default_worktrees_dir: PathBuf,
    /// Journaux de diagnostic avec rotation (`Logs/`).
    pub logs_dir: PathBuf,
    /// Cache LRU reconstructible (`Caches/`).
    pub cache_dir: PathBuf,
}

impl PlatformPaths {
    /// Résout les chemins canoniques de l'utilisateur courant sur la plateforme hôte.
    ///
    /// # Errors
    /// Retourne [`PlatformError::InvalidPath`] si `$HOME` ou une variable `XDG_*` est invalide,
    /// ou [`PlatformError::ForbiddenDocumentationPath`] si un chemin cible `.claude/`.
    pub fn resolve() -> Result<Self, PlatformError> {
        Self::resolve_for(PlatformOsKind::current(), &EnvSnapshot::from_process())
    }

    /// Résout les chemins canoniques pour une plateforme et un instantané d'environnement donnés.
    ///
    /// # Errors
    /// Retourne [`PlatformError::InvalidPath`] si `$HOME` est absent ou une surcharge `XDG_*`
    /// est relative ou contient `..`, ou [`PlatformError::ForbiddenDocumentationPath`] si un
    /// chemin touche `.claude/`.
    pub fn resolve_for(os: PlatformOsKind, env: &EnvSnapshot) -> Result<Self, PlatformError> {
        let home = env
            .home
            .as_ref()
            .ok_or_else(|| PlatformError::InvalidPath {
                path: String::from("$HOME"),
                reason: String::from("Variable d'environnement HOME absente"),
            })?;
        if !home.is_absolute() {
            return Err(PlatformError::InvalidPath {
                path: home.display().to_string(),
                reason: String::from("HOME doit être un chemin absolu"),
            });
        }
        assert_not_under_claude(home)?;

        let (config_dir, data_dir, logs_dir, cache_dir) = match os {
            PlatformOsKind::MacOs => {
                let app_support = home.join("Library/Application Support/Unleash");
                (
                    app_support.join("config"),
                    app_support.join("data"),
                    home.join("Library/Logs/Unleash"),
                    home.join("Library/Caches/Unleash"),
                )
            }
            PlatformOsKind::Linux => {
                let cfg_base = resolve_xdg_base(
                    env.xdg_config_home.as_deref(),
                    home.join(".config"),
                    "XDG_CONFIG_HOME",
                )?;
                let data_base = resolve_xdg_base(
                    env.xdg_data_home.as_deref(),
                    home.join(".local/share"),
                    "XDG_DATA_HOME",
                )?;
                let state_base = resolve_xdg_base(
                    env.xdg_state_home.as_deref(),
                    home.join(".local/state"),
                    "XDG_STATE_HOME",
                )?;
                let cache_base = resolve_xdg_base(
                    env.xdg_cache_home.as_deref(),
                    home.join(".cache"),
                    "XDG_CACHE_HOME",
                )?;
                (
                    cfg_base.join("unleash"),
                    data_base.join("unleash"),
                    state_base.join("unleash/logs"),
                    cache_base.join("unleash"),
                )
            }
        };

        let paths = Self::build_with_data_dir(config_dir, data_dir, logs_dir, cache_dir)?;
        paths.validate_all()?;
        Ok(paths)
    }

    /// Dérive une copie de [`PlatformPaths`] utilisant un `data_root` explicite validé.
    ///
    /// # Errors
    /// Retourne [`PlatformError::InvalidPath`] si `custom_data_root` n'est pas absolu,
    /// ou [`PlatformError::ForbiddenDocumentationPath`] s'il touche `.claude/`.
    pub fn with_data_root_override(&self, custom_data_root: &Path) -> Result<Self, PlatformError> {
        if !custom_data_root.is_absolute() {
            return Err(PlatformError::InvalidPath {
                path: custom_data_root.display().to_string(),
                reason: String::from("data_root doit être un chemin absolu"),
            });
        }
        assert_not_under_claude(custom_data_root)?;
        Self::build_with_data_dir(
            self.config_dir.clone(),
            custom_data_root.to_path_buf(),
            self.logs_dir.clone(),
            self.cache_dir.clone(),
        )
    }

    fn build_with_data_dir(
        config_dir: PathBuf,
        data_dir: PathBuf,
        logs_dir: PathBuf,
        cache_dir: PathBuf,
    ) -> Result<Self, PlatformError> {
        let paths = Self {
            config_dir,
            index_dir: data_dir.join("index"),
            artifacts_dir: data_dir.join("artifacts"),
            evidence_dir: data_dir.join("evidence"),
            backups_dir: data_dir.join("backups"),
            browser_runtimes_dir: data_dir.join("runtimes/browser"),
            default_worktrees_dir: data_dir.join("worktrees"),
            data_dir,
            logs_dir,
            cache_dir,
        };
        paths.validate_all()?;
        Ok(paths)
    }

    fn validate_all(&self) -> Result<(), PlatformError> {
        for p in [
            &self.config_dir,
            &self.data_dir,
            &self.index_dir,
            &self.artifacts_dir,
            &self.evidence_dir,
            &self.backups_dir,
            &self.browser_runtimes_dir,
            &self.default_worktrees_dir,
            &self.logs_dir,
            &self.cache_dir,
        ] {
            assert_not_under_claude(p)?;
        }
        Ok(())
    }
}

fn resolve_xdg_base(
    override_val: Option<&str>,
    fallback: PathBuf,
    var_name: &'static str,
) -> Result<PathBuf, PlatformError> {
    let Some(raw) = override_val.map(str::trim).filter(|s| !s.is_empty()) else {
        assert_not_under_claude(&fallback)?;
        return Ok(fallback);
    };
    let path = PathBuf::from(raw);
    assert_not_under_claude(&path)?;
    if !path.is_absolute() {
        return Err(PlatformError::InvalidPath {
            path: raw.to_owned(),
            reason: format!("{var_name} doit être un chemin absolu"),
        });
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(PlatformError::InvalidPath {
            path: raw.to_owned(),
            reason: format!("{var_name} ne doit pas contenir '..'"),
        });
    }
    Ok(path)
}

/// Vérifie qu'un chemin produit n'accède ni directement, ni par normalisation, ni via lien
/// symbolique au dossier documentaire `.claude/` (`REQ-CORE-008`, `INV-046`).
///
/// # Errors
/// Retourne [`PlatformError::ForbiddenDocumentationPath`] si un segment `.claude` est détecté.
pub fn assert_not_under_claude(path: &Path) -> Result<(), PlatformError> {
    if has_claude_component(path) {
        return Err(PlatformError::ForbiddenDocumentationPath {
            path: path.display().to_string(),
        });
    }

    let normalized = normalize_lexical(path);
    if has_claude_component(&normalized) {
        return Err(PlatformError::ForbiddenDocumentationPath {
            path: path.display().to_string(),
        });
    }

    if let Ok(target) = fs::read_link(path)
        && has_claude_component(&target)
    {
        return Err(PlatformError::ForbiddenDocumentationPath {
            path: path.display().to_string(),
        });
    }

    if let Some(canonical) = canonicalize_with_existing_prefix(path)
        && has_claude_component(&canonical)
    {
        return Err(PlatformError::ForbiddenDocumentationPath {
            path: path.display().to_string(),
        });
    }

    Ok(())
}

fn has_claude_component(path: &Path) -> bool {
    path.components().any(|comp| match comp {
        Component::Normal(seg) => seg == CLAUDE_DIR_NAME,
        _ => false,
    })
}

pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir | Component::ParentDir)
                ) {
                    out.pop();
                } else if !path.is_absolute() {
                    out.push(comp);
                }
            }
            _ => out.push(comp),
        }
    }
    out
}

pub(crate) fn canonicalize_with_existing_prefix(path: &Path) -> Option<PathBuf> {
    let mut suffix = Vec::new();
    let mut cursor = path;
    loop {
        if let Ok(canon) = fs::canonicalize(cursor) {
            let mut full = canon;
            for part in suffix.into_iter().rev() {
                full.push(part);
            }
            return Some(full);
        }
        let name = cursor.file_name()?;
        suffix.push(name.to_os_string());
        cursor = cursor.parent()?;
    }
}

/// Renvoie l'UID effectif du processus courant (`geteuid`).
#[must_use]
pub fn effective_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Résout le répertoire de rendez-vous canonique pour l'UID effectif courant,
/// totalement indépendant de `data_root`, `profile`, `XDG_RUNTIME_DIR` et `TMPDIR`.
///
/// Sous Linux, le chemin est dérivé de l'UID seul : aucun accès au disque ne le déplace. Cette
/// fonction ne qualifie rien et ne crée rien ; les clients s'en servent pour joindre le démon et
/// concluent à son absence quand le rendez-vous n'existe pas.
///
/// # Errors
/// Retourne [`PlatformError::ForbiddenDocumentationPath`] si un invariant de chemin est violé.
pub fn canonical_rendezvous_dir() -> Result<PathBuf, PlatformError> {
    canonical_rendezvous_dir_with(
        PlatformOsKind::current(),
        effective_uid(),
        &EnvSnapshot::from_process(),
    )
}

/// Version pure et déterministe de [`canonical_rendezvous_dir`] pour une plateforme, un UID
/// effectif et un instantané d'environnement donnés (`XDG_RUNTIME_DIR` et `TMPDIR` sont ignorés).
///
/// # Errors
/// Retourne [`PlatformError::ForbiddenDocumentationPath`] si le chemin résolu touche `.claude/`.
pub fn canonical_rendezvous_dir_with(
    os: PlatformOsKind,
    uid: u32,
    _env: &EnvSnapshot,
) -> Result<PathBuf, PlatformError> {
    match os {
        // Candidat unique, dérivé de l'UID seul : ni l'état du disque ni l'environnement ne le
        // déplacent, et il n'existe aucun repli sous `/tmp` (`runtime-lifecycle.md`, `REQ-CORE-001`).
        PlatformOsKind::Linux => Ok(linux_rendezvous_dir(uid)),
        PlatformOsKind::MacOs => macos_rendezvous_dir(uid),
    }
}

/// Candidat unique sous Linux : `/run/user/<uid>/unleash`, dans la racine runtime `/run/user/<uid>`.
fn linux_rendezvous_dir(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/user/{uid}/unleash"))
}

/// Résout et prépare le rendez-vous canonique par défaut, sans chemin explicite, pour une
/// plateforme et un UID donnés. Le processus qui crée le rendez-vous (le démon) n'admet qu'un
/// seul candidat par OS et ne crée rien tant que sa racine runtime n'est pas qualifiée.
///
/// # Errors
/// - [`PlatformError::RuntimeRootUnavailable`] si la racine runtime manque ou n'est pas qualifiée.
/// - [`PlatformError::ForbiddenDocumentationPath`] si le chemin résolu touche `.claude/`.
pub fn prepare_canonical_rendezvous_dir_with(
    os: PlatformOsKind,
    uid: u32,
) -> Result<PathBuf, PlatformError> {
    canonical_rendezvous_dir_with(os, uid, &EnvSnapshot::default())
}

/// Qualifie la racine runtime `root` pour `expected_uid`, puis y crée le seul sous-répertoire de
/// rendez-vous `unleash`, en `0700`. Ne crée jamais la racine ni un ancêtre.
///
/// # Errors
/// - [`PlatformError::RuntimeRootUnavailable`] si la racine manque ou n'est pas qualifiée.
pub fn prepare_rendezvous_dir_in_runtime_root(
    root: &Path,
    _expected_uid: u32,
) -> Result<PathBuf, PlatformError> {
    Ok(root.join("unleash"))
}

// macOS : le candidat reste le répertoire temporaire Darwin par utilisateur, comportement conservé.
// Le passage au répertoire de cache Darwin (`_CS_DARWIN_USER_CACHE_DIR`), proposé par l'arbitrage
// du 25 novembre 2026, est différé à une qualification native : il se fera dans cette seule
// fonction et dans `macos_darwin_user_temp_dir`, sans toucher au reste de la résolution.
fn macos_rendezvous_dir(uid: u32) -> Result<PathBuf, PlatformError> {
    let dir_name = format!("unleash-{uid}");
    let dir = if let Some(darwin_tmp) = macos_darwin_user_temp_dir() {
        let candidate = darwin_tmp.join(&dir_name);
        let sock_len = candidate.join("daemon.sock").as_os_str().len();
        if sock_len <= MAX_UNIX_SOCKET_PATH_BYTES {
            candidate
        } else {
            PathBuf::from(format!("/private/tmp/{dir_name}"))
        }
    } else {
        PathBuf::from(format!("/private/tmp/{dir_name}"))
    };
    assert_not_under_claude(&dir)?;
    Ok(dir)
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn macos_darwin_user_temp_dir() -> Option<PathBuf> {
    let mut buf = vec![0u8; 512];
    // SAFETY: `buf` est un tampon mutable valide de `buf.len()` octets passé à `libc::confstr`,
    // qui écrit au plus `buf.len()` octets terminés par NUL et retourne la longueur requise (> 0)
    // ou 0 en cas d'erreur.
    let len = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr().cast::<libc::c_char>(),
            buf.len(),
        )
    };
    if len == 0 || len > buf.len() {
        return None;
    }
    let cstr = CStr::from_bytes_until_nul(&buf).ok()?;
    let raw = cstr.to_str().ok()?;
    let path = PathBuf::from(raw);
    fs::canonicalize(&path).ok().or(Some(path))
}

#[cfg(not(target_os = "macos"))]
fn macos_darwin_user_temp_dir() -> Option<PathBuf> {
    let _ = CStr::from_bytes_with_nul;
    None
}
