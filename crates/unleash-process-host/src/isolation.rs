// SPDX-License-Identifier: Apache-2.0
//! Isolation OS stricte (mode `strict` Q-05) et préparation d'environnement épuré (`security-boundaries.md`).
//!
//! - Sur macOS : génère un profil Seatbelt SBPL v1 appliqué via `/usr/bin/sandbox-exec`, refusant la
//!   lecture de `data_root`, du répertoire du socket, de `~/Library/Keychains` / `/Library/Keychains`
//!   et des chemins de credentials déclarés, refusant toute écriture hors des racines assignées, et
//!   limitant la sortie réseau au seul proxy d'egress (ou refus total sans proxy).
//! - Sur Linux : prépare l'exécution `bubblewrap` (`--unshare-all`, `--ro-bind`, masquage des racines
//!   interdites par `--tmpfs`, `--clearenv`) ; le filtre `seccomp` BPF et le montage du socket proxy
//!   d'egress dans l'espace de noms `bwrap` restent hors périmètre M0 et non qualifiés sur Linux.
//! - Fail-closed : si le backend requis est absent ou incapable d'imposer les obligations demandées,
//!   retourne explicitement [`ProcessHostError::IsolationBackendMissing`] ou
//!   [`ProcessHostError::IsolationObligationUnenforceable`] (`IsolationUnavailable`), sans jamais
//!   effectuer de repli silencieux vers `standard`.

use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unleash_domain::Sha256Digest;
use unleash_platform::{
    assert_not_under_claude, ensure_private_rendezvous_dir, write_private_file_atomic,
};

use crate::error::ProcessHostError;

/// Mode d'isolation du processus enfant (`Q-05` / `security-boundaries.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationMode {
    /// Mode par défaut : non confiné au niveau OS, environnement épuré sans secrets généraux.
    Standard,
    /// Mode strict : exige un confinement OS effectif et fail-closed (`IsolationUnavailable` sinon).
    Strict,
}

/// Profil d'isolation appliqué en mode strict (`security-boundaries.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationProfileKind {
    /// Commande, build, test ou terminal interactif borné au worktree/racine assignée.
    WorkspaceCommand,
    /// Profil dédié au moteur navigateur (`BrowserPort`), incluant profil privé et ports loopback de contrôle.
    BrowserEngine,
}

/// Spécification des frontières à imposer en mode `strict`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrictIsolationSpec {
    /// Profil demandé (`WorkspaceCommand` ou `BrowserEngine`).
    pub profile_kind: IsolationProfileKind,
    /// Racines autorisées en écriture (worktree/racine assignée, tmp dédié, cache outil géré).
    pub assigned_write_roots: Vec<PathBuf>,
    /// Racine des données Unleash (`data_root`), strictement interdite en lecture/écriture (hors worktree explicite).
    pub data_root: PathBuf,
    /// Répertoire privé des sockets/verrous d'Unleash, strictement interdit en lecture/écriture.
    pub socket_dir: PathBuf,
    /// Chemins de trousseaux OS (`~/Library/Keychains`, `/Library/Keychains`, etc.) interdits en lecture.
    pub keychain_paths: Vec<PathBuf>,
    /// Chemins de credentials déclarés (ex. `~/.ssh`, `~/.aws`) interdits en lecture.
    pub declared_credential_paths: Vec<PathBuf>,
    /// Adresse loopback du proxy d'egress local autorisé (`None` = toute sortie réseau refusée).
    pub egress_proxy_addr: Option<SocketAddr>,
    /// Ports loopback de contrôle supplémentaires autorisés uniquement pour `BrowserEngine`.
    pub extra_loopback_control_ports: Vec<u16>,
    /// Répertoire privé `0700` du host où déposer le profil généré (`0600`) hors des racines d'écriture enfant.
    pub profile_output_dir: PathBuf,
}

/// Preuve et artefacts préparés par le backend d'isolation pour le lancement du processus enfant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedLaunchCommand {
    /// Exécutable final à lancer (`program` en `standard`, `/usr/bin/sandbox-exec` ou `bwrap` en `strict`).
    pub executable: PathBuf,
    /// Arguments exacts sans interpolation shell.
    pub args: Vec<String>,
    /// Environnement complet épuré à appliquer après `env_clear()`.
    pub clean_env: BTreeMap<String, String>,
    /// Identifiant du backend d'isolation appliqué (`none_standard`, `macos-seatbelt`, `linux-bubblewrap`).
    pub backend_name: String,
    /// Empreinte SHA-256 du profil généré en mode `strict`.
    pub profile_digest: Option<Sha256Digest>,
    /// Chemin du fichier profil généré en mode `strict` (mode `0600`).
    pub profile_path: Option<PathBuf>,
}

/// Résultat d'une sonde d'exécution de capacité sur un binaire de sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxProbeOutcome {
    /// Le backend a validé et appliqué le profil test avec succès.
    Supported,
    /// Le binaire du backend est absent ou non exécutable.
    MissingBackend {
        /// Détail d'absence.
        detail: String,
    },
    /// Le backend existe mais ne peut pas imposer l'obligation requise dans cet environnement.
    Unenforceable {
        /// Cause technique précise.
        detail: String,
    },
}

/// Port abstrait permettant de sonder le backend d'isolation OS (injectable en tests négatifs).
pub trait IsolationBackendProbe: Send + Sync {
    /// Retourne le chemin configuré de `/usr/bin/sandbox-exec` sur macOS.
    fn macos_sandbox_exec_path(&self) -> &Path;

    /// Vérifie si `sandbox-exec` sait compiler et appliquer le profil généré (sans effet métier).
    fn probe_macos_seatbelt(&self, sandbox_exec: &Path, profile_path: &Path)
    -> SandboxProbeOutcome;

    /// Retourne le chemin configuré de `bwrap` sur Linux.
    fn linux_bwrap_path(&self) -> &Path;

    /// Vérifie si `bwrap` sait créer un espace de noms isolé complet (`--unshare-all`) sans seccomp en M0.
    fn probe_linux_bwrap(&self, bwrap_path: &Path) -> SandboxProbeOutcome;
}

/// Sonde OS réelle de production utilisant `/usr/bin/sandbox-exec` (macOS) ou `bwrap` (Linux).
#[derive(Debug, Clone)]
pub struct OsIsolationBackendProbe {
    sandbox_exec_path: PathBuf,
    bwrap_path: PathBuf,
}

impl Default for OsIsolationBackendProbe {
    fn default() -> Self {
        Self {
            sandbox_exec_path: PathBuf::from("/usr/bin/sandbox-exec"),
            bwrap_path: PathBuf::from("/usr/bin/bwrap"),
        }
    }
}

impl OsIsolationBackendProbe {
    /// Crée une sonde pointant vers les emplacements OS standards.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Crée une sonde avec chemins personnalisés (utile pour tester `ISOLATION_BACKEND_MISSING`).
    #[must_use]
    pub fn with_paths(sandbox_exec_path: PathBuf, bwrap_path: PathBuf) -> Self {
        Self {
            sandbox_exec_path,
            bwrap_path,
        }
    }
}

impl IsolationBackendProbe for OsIsolationBackendProbe {
    fn macos_sandbox_exec_path(&self) -> &Path {
        &self.sandbox_exec_path
    }

    fn probe_macos_seatbelt(
        &self,
        sandbox_exec: &Path,
        profile_path: &Path,
    ) -> SandboxProbeOutcome {
        if !is_executable_file(sandbox_exec) {
            return SandboxProbeOutcome::MissingBackend {
                detail: format!(
                    "exécutable '{}' introuvable ou non exécutable",
                    sandbox_exec.display()
                ),
            };
        }

        let probe_target = if Path::new("/usr/bin/true").exists() {
            "/usr/bin/true"
        } else {
            "/bin/echo"
        };

        let output = Command::new(sandbox_exec)
            .arg("-f")
            .arg(profile_path)
            .arg("--")
            .arg(probe_target)
            .env_clear()
            .output();

        match output {
            Ok(out) if out.status.success() => SandboxProbeOutcome::Supported,
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
                SandboxProbeOutcome::Unenforceable {
                    detail: if stderr.is_empty() {
                        format!("sandbox-exec probe a échoué avec statut {}", out.status)
                    } else {
                        format!("sandbox-exec probe refusé : {stderr}")
                    },
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                SandboxProbeOutcome::MissingBackend {
                    detail: format!(
                        "exécutable '{}' introuvable : {err}",
                        sandbox_exec.display()
                    ),
                }
            }
            Err(err) => SandboxProbeOutcome::Unenforceable {
                detail: format!("impossible d'exécuter '{}': {err}", sandbox_exec.display()),
            },
        }
    }

    fn linux_bwrap_path(&self) -> &Path {
        &self.bwrap_path
    }

    fn probe_linux_bwrap(&self, bwrap_path: &Path) -> SandboxProbeOutcome {
        if !is_executable_file(bwrap_path) {
            return SandboxProbeOutcome::MissingBackend {
                detail: format!(
                    "exécutable bubblewrap '{}' introuvable ou non exécutable",
                    bwrap_path.display()
                ),
            };
        }

        let output = Command::new(bwrap_path)
            .args([
                "--unshare-all",
                "--ro-bind",
                "/",
                "/",
                "--die-with-parent",
                "/bin/true",
            ])
            .env_clear()
            .output();

        match output {
            Ok(out) if out.status.success() => SandboxProbeOutcome::Supported,
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
                SandboxProbeOutcome::Unenforceable {
                    detail: format!("bwrap --unshare-all refusé ({}) : {stderr}", out.status),
                }
            }
            Err(err) => SandboxProbeOutcome::Unenforceable {
                detail: format!("impossible d'exécuter bwrap : {err}"),
            },
        }
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    meta.is_file() && (meta.permissions().mode() & 0o111) != 0
}

/// Reconstruit un environnement propre et explicite sans hériter des secrets généraux du parent.
#[must_use]
pub fn build_clean_child_env(
    explicit_env: &BTreeMap<String, String>,
    assigned_tmp_dir: Option<&Path>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    // Conserver uniquement des variables système minimales non secrètes si non surchargées.
    for key in ["PATH", "LANG", "LC_ALL", "TERM"] {
        if let Some(val) = std::env::var_os(key).and_then(|v| v.into_string().ok()) {
            env.insert(key.to_owned(), val);
        }
    }
    env.entry("PATH".to_owned())
        .or_insert_with(|| String::from("/usr/bin:/bin:/usr/sbin:/sbin"));
    env.entry("TERM".to_owned())
        .or_insert_with(|| String::from("xterm-256color"));

    if let Some(tmp) = assigned_tmp_dir {
        env.insert(String::from("TMPDIR"), tmp.display().to_string());
    }

    for (k, v) in explicit_env {
        // Ne jamais injecter de variable de contrôle interne Unleash ni de nom vide/nul.
        if !k.is_empty()
            && !k.contains('\0')
            && !v.contains('\0')
            && !k.starts_with("UNLEASH_HOST_")
        {
            env.insert(k.clone(), v.clone());
        }
    }

    env
}

/// Prépare la commande finale de lancement en appliquant strictement `isolation_mode`.
///
/// En mode `Strict`, tout défaut de backend ou d'obligation échoue en `IsolationUnavailable`
/// sans jamais effectuer de repli vers `Standard`.
///
/// # Errors
/// - [`ProcessHostError::IsolationBackendMissing`] si le backend requis est absent.
/// - [`ProcessHostError::IsolationObligationUnenforceable`] si une règle ne peut être imposée.
/// - [`ProcessHostError::InvalidArgument`] si le programme est vide.
pub fn prepare_launch_command(
    program: &str,
    args: &[String],
    explicit_env: &BTreeMap<String, String>,
    isolation_mode: IsolationMode,
    strict_spec: Option<&StrictIsolationSpec>,
    probe: &dyn IsolationBackendProbe,
) -> Result<PreparedLaunchCommand, ProcessHostError> {
    if program.trim().is_empty() || program.contains('\0') {
        return Err(ProcessHostError::InvalidArgument {
            reason: String::from("programme enfant vide ou contenant un octet nul"),
        });
    }

    let first_write_root =
        strict_spec.and_then(|s| s.assigned_write_roots.first().map(PathBuf::as_path));
    let clean_env = build_clean_child_env(explicit_env, first_write_root);

    match isolation_mode {
        IsolationMode::Standard => Ok(PreparedLaunchCommand {
            executable: PathBuf::from(program),
            args: args.to_vec(),
            clean_env,
            backend_name: String::from("none_standard"),
            profile_digest: None,
            profile_path: None,
        }),
        IsolationMode::Strict => {
            let spec = strict_spec.ok_or_else(|| ProcessHostError::IsolationObligationUnenforceable {
                backend: String::from(current_strict_backend_name()),
                reason: String::from("spécification d'isolation stricte absente"),
                remediation: String::from(
                    "Fournir une StrictIsolationSpec complète ou repasser explicitement le projet en standard",
                ),
            })?;
            prepare_strict_launch_command(program, args, clean_env, spec, probe)
        }
    }
}

const fn current_strict_backend_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos-seatbelt"
    }
    #[cfg(target_os = "linux")]
    {
        "linux-bubblewrap"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "unsupported-os"
    }
}

pub(crate) fn prepare_strict_launch_command(
    program: &str,
    args: &[String],
    clean_env: BTreeMap<String, String>,
    spec: &StrictIsolationSpec,
    probe: &dyn IsolationBackendProbe,
) -> Result<PreparedLaunchCommand, ProcessHostError> {
    #[cfg(target_os = "macos")]
    {
        prepare_macos_seatbelt_launch(program, args, clean_env, spec, probe)
    }
    #[cfg(target_os = "linux")]
    {
        prepare_linux_bwrap_launch(program, args, clean_env, spec, probe)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (program, args, clean_env, spec, probe);
        Err(ProcessHostError::IsolationBackendMissing {
            backend: String::from("unsupported-os"),
            remediation: String::from(
                "Mode strict non supporté sur cet OS ; repasser explicitement en standard",
            ),
        })
    }
}

pub(crate) const MACOS_SEATBELT_PROFILE_FILENAME: &str = "seatbelt-strict.sb";

#[cfg(any(target_os = "macos", test))]
fn prepare_macos_seatbelt_launch(
    program: &str,
    args: &[String],
    clean_env: BTreeMap<String, String>,
    spec: &StrictIsolationSpec,
    probe: &dyn IsolationBackendProbe,
) -> Result<PreparedLaunchCommand, ProcessHostError> {
    let sandbox_exec = probe.macos_sandbox_exec_path();
    let profile_text = render_macos_seatbelt_profile(spec)?;

    assert_not_under_claude(&spec.profile_output_dir)?;
    ensure_private_rendezvous_dir(&spec.profile_output_dir)?;

    let profile_path = spec
        .profile_output_dir
        .join(MACOS_SEATBELT_PROFILE_FILENAME);
    write_private_file_atomic(&profile_path, profile_text.as_bytes())?;

    let mut hasher = Sha256::new();
    hasher.update(profile_text.as_bytes());
    let digest = Sha256Digest::from_raw_bytes(hasher.finalize().into());

    match probe.probe_macos_seatbelt(sandbox_exec, &profile_path) {
        SandboxProbeOutcome::Supported => {}
        SandboxProbeOutcome::MissingBackend { detail } => {
            return Err(ProcessHostError::IsolationBackendMissing {
                backend: String::from("macos-seatbelt"),
                remediation: format!(
                    "{detail} — installer/restaurer /usr/bin/sandbox-exec ou repasser explicitement le projet en mode standard"
                ),
            });
        }
        SandboxProbeOutcome::Unenforceable { detail } => {
            return Err(ProcessHostError::IsolationObligationUnenforceable {
                backend: String::from("macos-seatbelt"),
                reason: detail,
                remediation: String::from(
                    "Corriger les contraintes du profil ou repasser explicitement le projet en mode standard",
                ),
            });
        }
    }

    let mut final_args = Vec::with_capacity(args.len().saturating_add(4));
    final_args.push(String::from("-f"));
    final_args.push(profile_path.display().to_string());
    final_args.push(String::from("--"));
    final_args.push(program.to_owned());
    final_args.extend(args.iter().cloned());

    Ok(PreparedLaunchCommand {
        executable: sandbox_exec.to_path_buf(),
        args: final_args,
        clean_env,
        backend_name: String::from("macos-seatbelt"),
        profile_digest: Some(digest),
        profile_path: Some(profile_path),
    })
}

/// Génère un profil Seatbelt SBPL v1 déterministe pour macOS (`Q-05`).
///
/// # Errors
/// Retourne [`ProcessHostError::IsolationObligationUnenforceable`] si les racines assignées sont
/// vides, contiennent des caractères non représentables de façon sûre en SBPL, ou chevauchent un
/// répertoire protégé (`data_root`, `socket_dir`, trousseau ou credential).
pub fn render_macos_seatbelt_profile(
    spec: &StrictIsolationSpec,
) -> Result<String, ProcessHostError> {
    const BACKEND: &str = "macos-seatbelt";
    if spec.profile_kind == IsolationProfileKind::WorkspaceCommand
        && !spec.extra_loopback_control_ports.is_empty()
    {
        return Err(ProcessHostError::IsolationObligationUnenforceable {
            backend: BACKEND.to_owned(),
            reason: String::from("extra_loopback_control_ports réservé au profil browser_engine"),
            remediation: String::from("Retirer les ports loopback de contrôle du profil commande"),
        });
    }

    let validated = resolve_and_validate_isolation_paths(spec, BACKEND)?;

    if let Some(addr) = spec.egress_proxy_addr
        && !addr.ip().is_loopback()
    {
        return Err(ProcessHostError::IsolationObligationUnenforceable {
            backend: BACKEND.to_owned(),
            reason: format!("le proxy d'egress doit être loopback, reçu {}", addr.ip()),
            remediation: String::from("Configurer le proxy d'egress local sur 127.0.0.1/::1"),
        });
    }

    let mut out = String::new();
    out.push_str("(version 1)\n");
    out.push_str("(allow default)\n");

    if !validated.deny_reads.is_empty() {
        append_sbpl_subpath_block(
            &mut out,
            "(deny file-read*\n",
            &validated.deny_reads,
            BACKEND,
        )?;
    }

    // Seul un worktree strictement inclus sous data_root (ex. <data_root>/worktrees/...) peut recevoir
    // une réautorisation de lecture ciblée après le refus de data_root ; aucun recouvrement sous
    // socket_dir, keychain_paths ou declared_credential_paths n'est admis.
    let nested_read_allows: Vec<PathBuf> = validated
        .write_roots
        .iter()
        .filter(|wr| {
            validated
                .data_root_variants
                .iter()
                .any(|dr| wr.starts_with(dr) && *wr != dr)
        })
        .cloned()
        .collect();
    if !nested_read_allows.is_empty() {
        append_sbpl_subpath_block(
            &mut out,
            "(allow file-read*\n",
            &nested_read_allows,
            BACKEND,
        )?;
    }

    out.push_str("(deny file-write* (subpath \"/\"))\n");
    out.push_str("(allow file-write*\n");
    for path in &validated.write_roots {
        let escaped = escape_sbpl_path(path, BACKEND)?;
        out.push_str("  (subpath \"");
        out.push_str(&escaped);
        out.push_str("\")\n");
    }
    out.push_str("  (literal \"/dev/null\")\n");
    out.push_str("  (literal \"/dev/zero\")\n");
    out.push_str("  (literal \"/dev/tty\")\n");
    out.push_str("  (literal \"/dev/dtracehelper\")\n");
    out.push_str("  (regex #\"^/dev/ttys[0-9]+$\")\n");
    out.push_str(")\n");

    append_sbpl_network_rules(&mut out, spec);
    Ok(out)
}

fn append_sbpl_subpath_block(
    out: &mut String,
    header: &str,
    paths: &[PathBuf],
    backend: &'static str,
) -> Result<(), ProcessHostError> {
    out.push_str(header);
    for path in paths {
        let escaped = escape_sbpl_path(path, backend)?;
        out.push_str("  (subpath \"");
        out.push_str(&escaped);
        out.push_str("\")\n");
    }
    out.push_str(")\n");
    Ok(())
}

fn append_sbpl_network_rules(out: &mut String, spec: &StrictIsolationSpec) {
    use std::fmt::Write as _;

    out.push_str("(deny network-outbound)\n");
    if let Some(proxy) = spec.egress_proxy_addr {
        let _ = writeln!(
            out,
            "(allow network-outbound (remote ip \"localhost:{}\"))",
            proxy.port()
        );
    }
    for port in &spec.extra_loopback_control_ports {
        let _ = writeln!(
            out,
            "(allow network-outbound (remote ip \"localhost:{port}\"))"
        );
    }
}

/// Plan de commande Linux `bubblewrap` (`--unshare-all`, `--clearenv`, `--ro-bind`, `--tmpfs`),
/// validé en construction de plan en M0 (sans filtre `seccomp` ni montage de socket proxy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxBwrapPlan {
    /// Arguments passés à `bwrap`.
    pub bwrap_args: Vec<String>,
    /// Empreinte SHA-256 du plan généré.
    pub profile_digest: Sha256Digest,
}

/// Construit le plan d'isolation Linux `bubblewrap` (`--unshare-all`, `--ro-bind`, masquage des chemins
/// protégés par `--tmpfs`, `--clearenv`) pour `Q-05` ; refuse fail-closed tout proxy loopback requis
/// tant que le montage de socket proxy dans l'espace de noms `bwrap` et le filtre `seccomp` ne sont
/// pas implémentés.
///
/// # Errors
/// Retourne [`ProcessHostError::IsolationObligationUnenforceable`] si les contraintes sont invalides
/// ou exigent un proxy réseau / ports loopback non pris en charge par ce plan M0.
pub fn build_linux_bwrap_plan(
    program: &str,
    args: &[String],
    clean_env: &BTreeMap<String, String>,
    spec: &StrictIsolationSpec,
) -> Result<LinuxBwrapPlan, ProcessHostError> {
    const BACKEND: &str = "linux-bubblewrap";
    if spec.egress_proxy_addr.is_some() || !spec.extra_loopback_control_ports.is_empty() {
        return Err(ProcessHostError::IsolationObligationUnenforceable {
            backend: BACKEND.to_owned(),
            reason: String::from(
                "montage du socket proxy d'egress dans l'espace de noms bwrap (--unshare-all) non implémenté en M0",
            ),
            remediation: String::from(
                "Désactiver le proxy d'egress ou exécuter sans montage réseau Linux en M0",
            ),
        });
    }
    let validated = resolve_and_validate_isolation_paths(spec, BACKEND)?;

    let mut bwrap_args = vec![
        String::from("--unshare-all"),
        String::from("--die-with-parent"),
        String::from("--ro-bind"),
        String::from("/"),
        String::from("/"),
        String::from("--dev"),
        String::from("/dev"),
        String::from("--proc"),
        String::from("/proc"),
        String::from("--clearenv"),
    ];

    for (k, v) in clean_env {
        bwrap_args.push(String::from("--setenv"));
        bwrap_args.push(k.clone());
        bwrap_args.push(v.clone());
    }

    // Masquer les chemins protégés existants sous tmpfs inaccessible avant de monter les worktrees.
    for denied in &validated.deny_reads {
        if denied.exists() {
            bwrap_args.push(String::from("--tmpfs"));
            bwrap_args.push(denied.display().to_string());
        }
    }

    for wr in &validated.write_roots {
        let s = wr.display().to_string();
        bwrap_args.push(String::from("--bind"));
        bwrap_args.push(s.clone());
        bwrap_args.push(s);
    }

    bwrap_args.push(String::from("--"));
    bwrap_args.push(program.to_owned());
    bwrap_args.extend(args.iter().cloned());

    let mut hasher = Sha256::new();
    for arg in &bwrap_args {
        hasher.update(arg.as_bytes());
        hasher.update([0]);
    }
    let profile_digest = Sha256Digest::from_raw_bytes(hasher.finalize().into());

    Ok(LinuxBwrapPlan {
        bwrap_args,
        profile_digest,
    })
}

#[cfg(any(target_os = "linux", test))]
fn prepare_linux_bwrap_launch(
    program: &str,
    args: &[String],
    clean_env: BTreeMap<String, String>,
    spec: &StrictIsolationSpec,
    probe: &dyn IsolationBackendProbe,
) -> Result<PreparedLaunchCommand, ProcessHostError> {
    let bwrap_path = probe.linux_bwrap_path();
    let plan = build_linux_bwrap_plan(program, args, &clean_env, spec)?;

    match probe.probe_linux_bwrap(bwrap_path) {
        SandboxProbeOutcome::Supported => {}
        SandboxProbeOutcome::MissingBackend { detail } => {
            return Err(ProcessHostError::IsolationBackendMissing {
                backend: String::from("linux-bubblewrap"),
                remediation: format!(
                    "{detail} — installer bubblewrap (bwrap) ou repasser explicitement le projet en mode standard"
                ),
            });
        }
        SandboxProbeOutcome::Unenforceable { detail } => {
            return Err(ProcessHostError::IsolationObligationUnenforceable {
                backend: String::from("linux-bubblewrap"),
                reason: detail,
                remediation: String::from(
                    "Activer user namespaces ou repasser explicitement le projet en mode standard",
                ),
            });
        }
    }

    Ok(PreparedLaunchCommand {
        executable: bwrap_path.to_path_buf(),
        args: plan.bwrap_args,
        clean_env,
        backend_name: String::from("linux-bubblewrap"),
        profile_digest: Some(plan.profile_digest),
        profile_path: None,
    })
}

struct ValidatedIsolationPaths {
    write_roots: Vec<PathBuf>,
    data_root_variants: Vec<PathBuf>,
    deny_reads: Vec<PathBuf>,
}

fn resolve_and_validate_isolation_paths(
    spec: &StrictIsolationSpec,
    backend: &'static str,
) -> Result<ValidatedIsolationPaths, ProcessHostError> {
    if spec.assigned_write_roots.is_empty() {
        return Err(ProcessHostError::IsolationObligationUnenforceable {
            backend: backend.to_owned(),
            reason: String::from("aucune racine d'écriture assignée"),
            remediation: String::from(
                "Assigner au moins un worktree ou répertoire temporaire dédié",
            ),
        });
    }

    let write_roots = resolve_strict_path_variants(&spec.assigned_write_roots, backend)?;
    let data_root_variants =
        resolve_strict_path_variants(std::slice::from_ref(&spec.data_root), backend)?;
    let mut non_overlappable = Vec::new();
    non_overlappable.extend(resolve_strict_path_variants(
        std::slice::from_ref(&spec.socket_dir),
        backend,
    )?);
    non_overlappable.extend(resolve_strict_path_variants(&spec.keychain_paths, backend)?);
    non_overlappable.extend(resolve_strict_path_variants(
        &spec.declared_credential_paths,
        backend,
    )?);

    for wr in &write_roots {
        for data_path in &data_root_variants {
            if data_path.starts_with(wr) {
                return Err(ProcessHostError::IsolationObligationUnenforceable {
                    backend: backend.to_owned(),
                    reason: format!(
                        "racine d'écriture '{}' englobe ou égale data_root '{}'",
                        wr.display(),
                        data_path.display()
                    ),
                    remediation: String::from("Séparer la racine d'écriture de data_root"),
                });
            }
        }
        for forbidden in &non_overlappable {
            if forbidden.starts_with(wr) || wr.starts_with(forbidden) {
                return Err(ProcessHostError::IsolationObligationUnenforceable {
                    backend: backend.to_owned(),
                    reason: format!(
                        "racine d'écriture '{}' chevauche le répertoire protégé '{}'",
                        wr.display(),
                        forbidden.display()
                    ),
                    remediation: String::from(
                        "Séparer strictement les racines d'écriture des répertoires de sockets, trousseaux et credentials",
                    ),
                });
            }
        }
    }

    let mut deny_reads = data_root_variants.clone();
    deny_reads.extend(non_overlappable);
    Ok(ValidatedIsolationPaths {
        write_roots,
        data_root_variants,
        deny_reads,
    })
}

fn resolve_strict_path_variants(
    paths: &[PathBuf],
    backend: &'static str,
) -> Result<Vec<PathBuf>, ProcessHostError> {
    let mut out = Vec::new();
    for raw in paths {
        if !raw.is_absolute() {
            return Err(ProcessHostError::IsolationObligationUnenforceable {
                backend: backend.to_owned(),
                reason: format!("chemin strict non absolu : '{}'", raw.display()),
                remediation: String::from("Fournir exclusivement des chemins absolus"),
            });
        }
        let lexical = normalize_lexical(raw);
        let canonical = canonicalize_with_existing_prefix(&lexical);
        push_unique_path(&mut out, lexical);
        if let Some(c) = canonical {
            push_unique_path(&mut out, c);
        }
    }
    Ok(out)
}

fn push_unique_path(list: &mut Vec<PathBuf>, p: PathBuf) {
    if !list.contains(&p) {
        list.push(p);
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
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
                }
            }
            _ => out.push(comp),
        }
    }
    out
}

fn canonicalize_with_existing_prefix(path: &Path) -> Option<PathBuf> {
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

fn escape_sbpl_path(path: &Path, backend: &'static str) -> Result<String, ProcessHostError> {
    let s = path
        .to_str()
        .ok_or_else(|| ProcessHostError::IsolationObligationUnenforceable {
            backend: backend.to_owned(),
            reason: String::from("chemin non UTF-8 invalide pour profil Seatbelt"),
            remediation: String::from("Utiliser des chemins UTF-8 valides"),
        })?;
    if s.contains(['\0', '\n', '\r']) {
        return Err(ProcessHostError::IsolationObligationUnenforceable {
            backend: backend.to_owned(),
            reason: format!("caractère de contrôle interdit dans le chemin '{s}'"),
            remediation: String::from("Nettoyer les caractères de contrôle des chemins"),
        });
    }
    Ok(s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use unleash_testkit::local_test_dir;

    struct FakeProbe {
        outcome: SandboxProbeOutcome,
    }

    impl IsolationBackendProbe for FakeProbe {
        fn macos_sandbox_exec_path(&self) -> &Path {
            Path::new("/nonexistent/sandbox-exec")
        }

        fn probe_macos_seatbelt(
            &self,
            _sandbox_exec: &Path,
            _profile_path: &Path,
        ) -> SandboxProbeOutcome {
            self.outcome.clone()
        }

        fn linux_bwrap_path(&self) -> &Path {
            Path::new("/nonexistent/bwrap")
        }

        fn probe_linux_bwrap(&self, _bwrap_path: &Path) -> SandboxProbeOutcome {
            self.outcome.clone()
        }
    }

    #[test]
    fn strict_refuses_when_backend_missing_never_falls_back_to_standard() {
        let dir = local_test_dir("iso-miss").unwrap();
        let work = dir.path().join("work");
        let data = dir.path().join("data");
        let sock = dir.path().join("sock");
        let prof = dir.path().join("prof");
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&sock).unwrap();

        let spec = StrictIsolationSpec {
            profile_kind: IsolationProfileKind::WorkspaceCommand,
            assigned_write_roots: vec![work],
            data_root: data,
            socket_dir: sock,
            keychain_paths: vec![],
            declared_credential_paths: vec![],
            egress_proxy_addr: None,
            extra_loopback_control_ports: vec![],
            profile_output_dir: prof,
        };

        let probe = FakeProbe {
            outcome: SandboxProbeOutcome::MissingBackend {
                detail: "binary absent".into(),
            },
        };
        let err = prepare_launch_command(
            "/bin/echo",
            &["hi".into()],
            &BTreeMap::new(),
            IsolationMode::Strict,
            Some(&spec),
            &probe,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ProcessHostError::IsolationBackendMissing { .. }
        ));
        assert_eq!(
            err.to_ipc_error(None).reason_code,
            Some(unleash_protocol::ReasonCode::IsolationBackendMissing)
        );
    }

    #[test]
    fn strict_rejects_overlapping_write_root_over_data_root() {
        let dir = local_test_dir("iso-ovl").unwrap();
        let data = dir.path().join("data");
        let sock = dir.path().join("sock");
        let prof = dir.path().join("prof");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&sock).unwrap();

        let spec = StrictIsolationSpec {
            profile_kind: IsolationProfileKind::WorkspaceCommand,
            assigned_write_roots: vec![dir.path().to_path_buf()],
            data_root: data,
            socket_dir: sock,
            keychain_paths: vec![],
            declared_credential_paths: vec![],
            egress_proxy_addr: None,
            extra_loopback_control_ports: vec![],
            profile_output_dir: prof,
        };
        let err = render_macos_seatbelt_profile(&spec).unwrap_err();
        assert!(matches!(
            err,
            ProcessHostError::IsolationObligationUnenforceable { .. }
        ));
    }

    #[test]
    fn linux_bwrap_plan_includes_unshare_net_clearenv_and_binds() {
        let dir = local_test_dir("iso-bwrap").unwrap();
        let work = dir.path().join("work");
        let data = dir.path().join("data");
        let sock = dir.path().join("sock");
        let prof = dir.path().join("prof");
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&sock).unwrap();

        let spec = StrictIsolationSpec {
            profile_kind: IsolationProfileKind::WorkspaceCommand,
            assigned_write_roots: vec![work],
            data_root: data,
            socket_dir: sock,
            keychain_paths: vec![],
            declared_credential_paths: vec![],
            egress_proxy_addr: None,
            extra_loopback_control_ports: vec![],
            profile_output_dir: prof,
        };
        let plan = build_linux_bwrap_plan("/bin/echo", &["hello".into()], &BTreeMap::new(), &spec)
            .unwrap();
        assert!(plan.bwrap_args.contains(&"--unshare-all".to_owned()));
        assert!(plan.bwrap_args.contains(&"--clearenv".to_owned()));
        assert!(plan.bwrap_args.contains(&"--ro-bind".to_owned()));
    }

    #[test]
    fn strict_rejects_write_root_nested_inside_socket_keychain_or_credentials() {
        let dir = local_test_dir("iso-nest-forb").unwrap();
        let data = dir.path().join("data");
        let sock = dir.path().join("sock");
        let keychain = dir.path().join("keychain");
        let cred = dir.path().join("creds");
        let prof = dir.path().join("prof");
        let under_sock = sock.join("child");
        let under_kc = keychain.join("sub");
        let under_cred = cred.join("sub");
        for p in [&data, &under_sock, &under_kc, &under_cred] {
            fs::create_dir_all(p).unwrap();
        }

        for forbidden_wr in [under_sock, under_kc, under_cred] {
            let spec = StrictIsolationSpec {
                profile_kind: IsolationProfileKind::WorkspaceCommand,
                assigned_write_roots: vec![forbidden_wr],
                data_root: data.clone(),
                socket_dir: sock.clone(),
                keychain_paths: vec![keychain.clone()],
                declared_credential_paths: vec![cred.clone()],
                egress_proxy_addr: None,
                extra_loopback_control_ports: vec![],
                profile_output_dir: prof.clone(),
            };
            assert!(matches!(
                render_macos_seatbelt_profile(&spec),
                Err(ProcessHostError::IsolationObligationUnenforceable { .. })
            ));
            assert!(matches!(
                build_linux_bwrap_plan("/bin/echo", &[], &BTreeMap::new(), &spec),
                Err(ProcessHostError::IsolationObligationUnenforceable { .. })
            ));
        }
    }

    #[test]
    fn strict_allows_nested_worktree_under_data_root_only() {
        let dir = local_test_dir("iso-wt-data").unwrap();
        let data = dir.path().join("data");
        let wt = data.join("worktrees").join("wt-1");
        let sock = dir.path().join("sock");
        let cred = dir.path().join("creds");
        let prof = dir.path().join("prof");
        for p in [&wt, &sock, &cred] {
            fs::create_dir_all(p).unwrap();
        }

        let spec = StrictIsolationSpec {
            profile_kind: IsolationProfileKind::WorkspaceCommand,
            assigned_write_roots: vec![wt.clone()],
            data_root: data.clone(),
            socket_dir: sock.clone(),
            keychain_paths: vec![],
            declared_credential_paths: vec![cred.clone()],
            egress_proxy_addr: None,
            extra_loopback_control_ports: vec![],
            profile_output_dir: prof,
        };
        let profile = render_macos_seatbelt_profile(&spec).unwrap();
        let wt_canon = fs::canonicalize(&wt).unwrap();
        let data_canon = fs::canonicalize(&data).unwrap();
        let expected_allow = format!("(allow file-read*\n  (subpath \"{}\")\n", wt.display());
        let expected_allow_canon = format!(
            "(allow file-read*\n  (subpath \"{}\")\n",
            wt_canon.display()
        );
        assert!(
            profile.contains(&expected_allow) || profile.contains(&expected_allow_canon),
            "profile={profile}"
        );
        assert!(profile.contains(&data_canon.display().to_string()));
        assert!(profile.contains(&sock.display().to_string()));
        assert!(profile.contains(&cred.display().to_string()));
    }

    #[test]
    fn prepare_platform_strict_launchers_validate_positive_missing_and_unenforceable() {
        let dir = local_test_dir("iso-prep").unwrap();
        let work = dir.path().join("work");
        let data = dir.path().join("data");
        let sock = dir.path().join("sock");
        let prof = dir.path().join("prof");
        for p in [&work, &data, &sock] {
            fs::create_dir_all(p).unwrap();
        }

        let spec = StrictIsolationSpec {
            profile_kind: IsolationProfileKind::WorkspaceCommand,
            assigned_write_roots: vec![work],
            data_root: data,
            socket_dir: sock,
            keychain_paths: vec![],
            declared_credential_paths: vec![],
            egress_proxy_addr: None,
            extra_loopback_control_ports: vec![],
            profile_output_dir: prof,
        };

        let ok_probe = FakeProbe {
            outcome: SandboxProbeOutcome::Supported,
        };
        let linux_ok = prepare_linux_bwrap_launch(
            "/bin/echo",
            &["hi".into()],
            BTreeMap::new(),
            &spec,
            &ok_probe,
        )
        .unwrap();
        assert_eq!(linux_ok.backend_name, "linux-bubblewrap");
        let mut with_proxy = spec.clone();
        with_proxy.egress_proxy_addr = Some("127.0.0.1:18080".parse().unwrap());
        assert!(matches!(
            build_linux_bwrap_plan("/bin/echo", &[], &BTreeMap::new(), &with_proxy),
            Err(ProcessHostError::IsolationObligationUnenforceable { .. })
        ));
        let mac_ok = prepare_macos_seatbelt_launch(
            "/bin/echo",
            &["hi".into()],
            BTreeMap::new(),
            &spec,
            &ok_probe,
        )
        .unwrap();
        assert_eq!(mac_ok.backend_name, "macos-seatbelt");

        let miss_probe = FakeProbe {
            outcome: SandboxProbeOutcome::MissingBackend {
                detail: "absent".into(),
            },
        };
        assert!(matches!(
            prepare_linux_bwrap_launch("/bin/echo", &[], BTreeMap::new(), &spec, &miss_probe),
            Err(ProcessHostError::IsolationBackendMissing { .. })
        ));
        assert!(matches!(
            prepare_macos_seatbelt_launch("/bin/echo", &[], BTreeMap::new(), &spec, &miss_probe),
            Err(ProcessHostError::IsolationBackendMissing { .. })
        ));

        let unenf_probe = FakeProbe {
            outcome: SandboxProbeOutcome::Unenforceable {
                detail: "refus".into(),
            },
        };
        assert!(matches!(
            prepare_linux_bwrap_launch("/bin/echo", &[], BTreeMap::new(), &spec, &unenf_probe),
            Err(ProcessHostError::IsolationObligationUnenforceable { .. })
        ));
        assert!(matches!(
            prepare_macos_seatbelt_launch("/bin/echo", &[], BTreeMap::new(), &spec, &unenf_probe),
            Err(ProcessHostError::IsolationObligationUnenforceable { .. })
        ));
    }
}
