// SPDX-License-Identifier: Apache-2.0
//! Inspecteur Git en lecture seule : profil épinglé, sous-processus bornés et collecte de
//! l'inventaire des worktrees d'un dépôt (`REQ-GIT-001`, `AC-F010-001`).
//!
//! Aucune commande lancée ici ne modifie le dépôt : ni index, ni refs, ni configuration, ni
//! fichiers. Le profil ignore les configurations système et globale, désactive les écritures
//! facultatives (`GIT_OPTIONAL_LOCKS=0`) et les replacement refs (`--no-replace-objects`), refuse
//! tout transport et rend inertes les filtres configurés dans le dépôt : aucun hook, filtre,
//! helper ni fsmonitor n'est exécuté.
//!
//! L'état `dirty` s'appuie sur les métadonnées `stat` des fichiers suivis, sans rafraîchir
//! l'index. Git ne relit un contenu que pour une entrée d'index encore récente, au sens de Git ;
//! comme les filtres sont inertes, un fichier géré par un filtre peut alors être annoncé `dirty`.
//! Rien n'est jamais annoncé `clean` sans observation : ce qui n'a pas pu être établi reste
//! `unknown`, et les sous-modules ne sont pas inspectés.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, ChildStdout, Command};
use tokio::time::Instant;
use unleash_domain::Sha256Digest;
use unleash_protocol::{
    GitObjectFormat, GitObjectId, GitOperation, InventoryCoverage, InventoryCoverageStatus,
    WorktreeDirty, WorktreeItem, WorktreeOwnership, canonical_sha256, is_worktree_branch_ref,
};

/// Programme unique lancé ; il est cherché dans le `PATH` transmis au sous-processus.
const GIT_PROGRAM: &str = "git";
/// Version minimale qualifiée (ADR-09).
const MIN_GIT_VERSION: (u16, u16) = (2, 39);
/// Identifiant du profil de lecture : il entre dans `git_profile_digest`.
const READ_PROFILE_ID: &str = "unleash.git.read.v1";

/// Délai bornant toute la collecte ; il répond avant l'échéance usuelle d'un client.
const INVENTORY_TIMEOUT: Duration = Duration::from_secs(25);
/// Délai laissé à un processus pour se terminer après une lecture interrompue.
const CHILD_GRACE: Duration = Duration::from_secs(5);

const READ_CHUNK: usize = 16 * 1024;
const MAX_LISTING_BYTES: usize = 1024 * 1024;
const MAX_STDERR_BYTES: usize = 16 * 1024;
/// Plus grand flux d'index parcouru à la recherche de sous-modules.
const MAX_STREAMED_BYTES: u64 = 256 * 1024 * 1024;
/// Plus long chemin attendu dans une sortie Git.
const MAX_PATH_BYTES: usize = 64 * 1024;
const MAX_WORKTREES: usize = 256;

const DIFF_INDEX: [&str; 7] = [
    "diff-index",
    "--quiet",
    "--ignore-submodules=all",
    "--no-ext-diff",
    "--no-textconv",
    "HEAD",
    "--",
];
const DIFF_FILES: [&str; 5] = [
    "diff-files",
    "--quiet",
    "--ignore-submodules=all",
    "--no-ext-diff",
    "--",
];
const STAGED_ENTRIES: [&str; 3] = ["ls-files", "--stage", "-z"];
const UNTRACKED: [&str; 6] = [
    "ls-files",
    "--others",
    "--exclude-standard",
    "--directory",
    "--no-empty-directory",
    "-z",
];
const FILTER_KEYS: [&str; 4] = ["config", "--name-only", "--get-regexp", "^filter\\."];

/// Environnement du profil, commun à chaque sous-processus Git.
const PROFILE_ENV: [(&str, &str); 7] = [
    ("GIT_ATTR_NOSYSTEM", "1"),
    ("GIT_CONFIG_GLOBAL", ""),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_NO_LAZY_FETCH", "1"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("LC_ALL", "C"),
];

/// Variables du daemon transmises telles quelles au sous-processus.
const PASSTHROUGH_ENV: [&str; 12] = [
    "APPDATA",
    "HOME",
    "LOCALAPPDATA",
    "PATH",
    "PATHEXT",
    "PROGRAMDATA",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "TMPDIR",
    "USERPROFILE",
    "XDG_CONFIG_HOME",
];

/// Surcharges de configuration du profil, passées par `-c` après celles du dépôt.
const PROFILE_CONFIG: [(&str, &str); 2] =
    [("core.fsmonitor", "false"), ("protocol.allow", "never")];

/// Échec d'une collecte d'inventaire.
#[derive(Debug, PartialEq, Eq, Error)]
pub(crate) enum InventoryError {
    /// Le programme Git est introuvable ou non exécutable.
    #[error("Git système introuvable ou non exécutable")]
    GitUnavailable,
    /// La version rendue par Git n'est pas reconnue ou est inférieure à 2.39.
    #[error("version Git non reconnue ou inférieure à 2.39")]
    VersionUnsupported,
    /// La racine enregistrée n'est pas lisible comme dépôt Git.
    #[error("la racine enregistrée n'est pas un dépôt Git lisible")]
    UnreadableRepository,
    /// La collecte a dépassé son délai.
    #[error("la collecte a dépassé son délai")]
    DeadlineReached,
    /// Une sortie Git dépasse le volume que la collecte accepte de lire.
    #[error("une sortie Git dépasse le volume admis")]
    StreamLimitReached,
    /// Une commande Git s'est terminée par un code non attendu.
    #[error("commande Git en échec")]
    GitFailed,
    /// Une sortie Git ne correspond pas au format documenté.
    #[error("sortie Git inattendue")]
    UnexpectedOutput,
}

/// Inventaire collecté d'un dépôt rattaché.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeInventory {
    /// Format d'objet observé.
    pub(crate) object_format: GitObjectFormat,
    /// Empreinte du profil Git effectif de la collecte.
    pub(crate) git_profile_digest: Sha256Digest,
    /// Worktrees représentables, triés par les octets de `path`.
    pub(crate) items: Vec<WorktreeItem>,
    /// Couverture de la collecte.
    pub(crate) coverage: InventoryCoverage,
}

/// Classe la propriété d'un worktree par son chemin observé.
pub(crate) fn classify_ownership(
    path: &str,
    managed: Option<&HashSet<String>>,
) -> WorktreeOwnership {
    match managed {
        Some(paths) if paths.contains(path) => WorktreeOwnership::Managed,
        Some(_) => WorktreeOwnership::External,
        None => WorktreeOwnership::Unknown,
    }
}

/// Vérifie que la racine enregistrée est elle-même un dépôt, sans laisser Git remonter vers un
/// dépôt parent.
pub(crate) fn ensure_repository_root(root: &Path) -> Result<(), InventoryError> {
    let present = |name: &str| root.join(name).try_exists().unwrap_or(false);
    let main_or_linked = present(".git");
    let bare = present("HEAD") && present("objects") && present("refs");
    if main_or_linked || bare {
        Ok(())
    } else {
        Err(InventoryError::UnreadableRepository)
    }
}

/// Lit le libellé de version rendu par `git --version`, seule valeur de version retenue dans
/// l'empreinte du profil, et refuse toute version inférieure au minimum qualifié.
fn parse_git_version(stdout: &[u8]) -> Result<String, InventoryError> {
    let unsupported = || InventoryError::VersionUnsupported;
    let text = std::str::from_utf8(stdout).map_err(|_| unsupported())?;
    let label = text.lines().next().ok_or_else(unsupported)?;
    let number = label.strip_prefix("git version ").ok_or_else(unsupported)?;
    let number: String = number
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let mut components = number.split('.');
    let mut component = || {
        components
            .next()
            .and_then(|part| part.parse::<u16>().ok())
            .unwrap_or(0)
    };
    let major = component();
    let minor = component();
    if number.is_empty() || (major, minor) < MIN_GIT_VERSION {
        return Err(unsupported());
    }
    Ok(label.trim().to_owned())
}

/// Profil de lecture dont l'empreinte est exposée : toute différence de version, de format
/// d'objet ou de surcharges change la valeur de `git_profile_digest`.
#[derive(Serialize)]
struct GitReadProfile<'a> {
    profile: &'a str,
    git_version: &'a str,
    object_format: GitObjectFormat,
    environment: BTreeMap<&'a str, &'a str>,
    config: BTreeMap<&'a str, &'a str>,
}

fn profile_digest(
    version_label: &str,
    object_format: GitObjectFormat,
) -> Result<Sha256Digest, InventoryError> {
    let profile = GitReadProfile {
        profile: READ_PROFILE_ID,
        git_version: version_label,
        object_format,
        environment: PROFILE_ENV.into_iter().collect(),
        config: PROFILE_CONFIG.into_iter().collect(),
    };
    canonical_sha256(&profile).map_err(|_| InventoryError::UnexpectedOutput)
}

/// Worktree tel que `git worktree list --porcelain -z` le rend.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawWorktree {
    /// Chemin observé, en octets valides en UTF-8.
    path: String,
    /// `HEAD` rendu par Git ; l'identifiant nul est celui d'une branche unborn.
    head: String,
    /// Branche attachée ; `None` pour un `HEAD` détaché.
    branch: Option<String>,
}

/// Sortie analysée de la liste des worktrees.
#[derive(Debug, Default, PartialEq, Eq)]
struct WorktreeListing {
    /// Worktrees représentables dans le contrat fermé.
    worktrees: Vec<RawWorktree>,
    /// Au moins un worktree n'a pas pu être représenté.
    omitted: bool,
}

/// Enregistrement en cours de lecture de la liste des worktrees.
#[derive(Debug, Default)]
struct RecordBuilder {
    path: Option<String>,
    path_seen: bool,
    bare: bool,
    head: Option<String>,
    branch: Option<String>,
    detached: bool,
}

/// Analyse la sortie de `git worktree list --porcelain -z` : enregistrements terminés par un
/// champ vide, chemins bruts non cités.
fn parse_worktree_listing(stdout: &[u8]) -> Result<WorktreeListing, InventoryError> {
    let mut listing = WorktreeListing::default();
    let mut current: Option<RecordBuilder> = None;
    for field in stdout.split(|&byte| byte == 0) {
        if field.is_empty() {
            if let Some(record) = current.take() {
                close_record(record, &mut listing)?;
            }
            continue;
        }
        if let Some(raw_path) = field.strip_prefix(b"worktree ") {
            if let Some(record) = current.take() {
                close_record(record, &mut listing)?;
            }
            current = Some(RecordBuilder {
                path: std::str::from_utf8(raw_path).ok().map(ToOwned::to_owned),
                path_seen: true,
                ..RecordBuilder::default()
            });
            continue;
        }
        let record = current.as_mut().ok_or(InventoryError::UnexpectedOutput)?;
        let text = std::str::from_utf8(field).map_err(|_| InventoryError::UnexpectedOutput)?;
        // Verrouillage et élagabilité ne changent ni le chemin ni la branche observée.
        let informational = matches!(text, "locked" | "prunable")
            || text.starts_with("locked ")
            || text.starts_with("prunable ");
        if informational {
            continue;
        }
        if text == "bare" {
            record.bare = true;
        } else if let Some(hex) = text.strip_prefix("HEAD ") {
            record.head = Some(hex.to_owned());
        } else if text == "detached" {
            record.detached = true;
        } else if let Some(name) = text.strip_prefix("branch ") {
            record.branch = Some(name.to_owned());
        } else {
            return Err(InventoryError::UnexpectedOutput);
        }
    }
    if let Some(record) = current.take() {
        close_record(record, &mut listing)?;
    }
    Ok(listing)
}

fn close_record(
    record: RecordBuilder,
    listing: &mut WorktreeListing,
) -> Result<(), InventoryError> {
    // Un dépôt bare n'a pas de checkout : il ne crée aucun item de HEAD détaché.
    if record.bare {
        return Ok(());
    }
    if !record.path_seen {
        return Err(InventoryError::UnexpectedOutput);
    }
    let Some(head) = record.head else {
        return Err(InventoryError::UnexpectedOutput);
    };
    if record.branch.is_some() == record.detached {
        return Err(InventoryError::UnexpectedOutput);
    }
    // Un chemin ou une branche que le contrat fermé ne peut pas porter fait omettre le worktree.
    let Some(path) = record.path else {
        listing.omitted = true;
        return Ok(());
    };
    match record.branch {
        Some(branch) if !is_worktree_branch_ref(&branch) => listing.omitted = true,
        Some(branch) => listing.worktrees.push(RawWorktree {
            path,
            head,
            branch: Some(branch),
        }),
        None => listing.worktrees.push(RawWorktree {
            path,
            head,
            branch: None,
        }),
    }
    Ok(())
}

/// Opérations en cours observées dans le répertoire Git privé d'un worktree.
#[derive(Debug, Default, PartialEq, Eq)]
struct OperationState {
    operations: Vec<GitOperation>,
    complete: bool,
    reservations_known: bool,
}

impl OperationState {
    /// État rendu quand le répertoire Git privé n'a pas pu être lu.
    fn unreadable() -> Self {
        Self {
            operations: Vec::new(),
            complete: false,
            reservations_known: false,
        }
    }
}

/// Lit les marqueurs que Git consulte lui-même pour décrire un worktree. Rien n'est inventé :
/// une opération que le contrat ne représente pas, ou un marqueur illisible, laisse
/// `complete` à `false`.
fn detect_operations(gitdir: &Path) -> OperationState {
    let mut unreadable = false;
    let mut present = |name: &str| match gitdir.join(name).try_exists() {
        Ok(seen) => seen,
        Err(_) => {
            unreadable = true;
            false
        }
    };
    // `git am` laisse `rebase-apply/applying` : ce n'est pas un rebase, et ni cette opération
    // ni `git revert` n'ont de valeur dans le contrat fermé.
    let applying = present("rebase-apply/applying");
    let reverting = present("REVERT_HEAD");
    let observed = [
        (GitOperation::Merge, present("MERGE_HEAD")),
        (
            GitOperation::Rebase,
            !applying && (present("rebase-apply") || present("rebase-merge")),
        ),
        (GitOperation::Bisect, present("BISECT_LOG")),
        (GitOperation::CherryPick, present("CHERRY_PICK_HEAD")),
    ];
    let operations: Vec<GitOperation> = observed
        .into_iter()
        .filter(|(_, seen)| *seen)
        .map(|(operation, _)| operation)
        .collect();
    let reserves_branch =
        operations.contains(&GitOperation::Rebase) || operations.contains(&GitOperation::Bisect);
    OperationState {
        operations,
        complete: !unreadable && !applying && !reverting,
        reservations_known: !reserves_branch,
    }
}

/// Surcharge de configuration qui rend un pilote de filtre inerte.
fn filter_overrides(config_names: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(config_names).ok()?;
    let mut drivers = BTreeSet::new();
    for key in text.lines().filter(|line| !line.is_empty()) {
        let rest = key.strip_prefix("filter.")?;
        let (driver, _) = rest.rsplit_once('.')?;
        let plain = !driver.is_empty()
            && driver.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'_'
            });
        if !plain {
            return None;
        }
        drivers.insert(driver.to_owned());
    }
    let mut overrides = Vec::with_capacity(drivers.len() * 8);
    for driver in drivers {
        for subkey in ["clean", "process", "smudge"] {
            overrides.push(String::from("-c"));
            overrides.push(format!("filter.{driver}.{subkey}="));
        }
        overrides.push(String::from("-c"));
        overrides.push(format!("filter.{driver}.required=false"));
    }
    Some(overrides)
}

/// Recherche d'entrées de sous-module dans un flux de `ls-files --stage -z`.
#[derive(Debug, Default)]
struct GitlinkScan {
    record: Vec<u8>,
    found: bool,
}

impl GitlinkScan {
    /// Absorbe un fragment du flux ; les enregistrements sont terminés par un octet nul.
    fn push(&mut self, data: &[u8]) -> Result<(), InventoryError> {
        let mut rest = data;
        while let Some(end) = rest.iter().position(|&byte| byte == 0) {
            let (head, tail) = rest.split_at(end);
            self.record.extend_from_slice(head);
            self.found |= self.record.starts_with(b"160000 ");
            self.record.clear();
            rest = tail.get(1..).unwrap_or(&[]);
        }
        self.record.extend_from_slice(rest);
        if self.record.len() > MAX_PATH_BYTES {
            return Err(InventoryError::UnexpectedOutput);
        }
        Ok(())
    }

    /// Conclut sur les enregistrements absorbés, y compris le dernier s'il n'est pas terminé.
    fn found(&self) -> bool {
        self.found || self.record.starts_with(b"160000 ")
    }
}

/// Résultat d'une lecture de la sortie standard d'un sous-processus Git.
struct Capture<T> {
    value: T,
    /// La sortie a été lue jusqu'à sa fin.
    read_to_end: bool,
    /// Rien de la sortie n'a été laissé de côté.
    complete: bool,
}

async fn read_up_to<R>(mut reader: R, cap: usize) -> Result<Capture<Vec<u8>>, InventoryError>
where
    R: AsyncRead + Unpin,
{
    let mut kept = Vec::new();
    let mut complete = true;
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|_| InventoryError::GitFailed)?;
        if read == 0 {
            return Ok(Capture {
                value: kept,
                read_to_end: true,
                complete,
            });
        }
        let data = buffer.split_at(read).0;
        let room = cap.saturating_sub(kept.len());
        kept.extend_from_slice(data.get(..room).unwrap_or(&[]));
        complete &= data.len() <= room;
    }
}

async fn scan_gitlinks(
    mut reader: ChildStdout,
    limit: u64,
) -> Result<Capture<bool>, InventoryError> {
    let mut scan = GitlinkScan::default();
    let mut scanned = 0u64;
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|_| InventoryError::GitFailed)?;
        if read == 0 {
            return Ok(Capture {
                value: scan.found(),
                read_to_end: true,
                complete: true,
            });
        }
        let data = buffer.split_at(read).0;
        scanned = scanned.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        if scanned > limit {
            return Err(InventoryError::StreamLimitReached);
        }
        scan.push(data)?;
        if scan.found() {
            return Ok(Capture {
                value: true,
                read_to_end: false,
                complete: true,
            });
        }
    }
}

fn arguments(command: &[&str]) -> Vec<String> {
    command.iter().map(|value| (*value).to_owned()).collect()
}

fn with_overrides(overrides: &[String], command: &[&str]) -> Vec<String> {
    let mut full = overrides.to_vec();
    full.extend(arguments(command));
    full
}

/// Conserve l'échéance dépassée, seul échec qui arrête toute la collecte, et remplace tout autre
/// échec par la cause retenue pour l'appelant.
fn unless_deadline(error: InventoryError, fallback: InventoryError) -> InventoryError {
    if error == InventoryError::DeadlineReached {
        error
    } else {
        fallback
    }
}

/// Lance Git avec le profil de lecture : programme et arguments séparés, jamais d'interpréteur.
fn spawn_git(arguments: &[String], directory: Option<&Path>) -> Result<Child, InventoryError> {
    let mut command = Command::new(GIT_PROGRAM);
    command.arg("--no-replace-objects");
    for (key, value) in PROFILE_CONFIG {
        command.arg("-c").arg(format!("{key}={value}"));
    }
    command.args(arguments).env_clear();
    for name in PASSTHROUGH_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for (key, value) in PROFILE_ENV {
        command.env(key, value);
    }
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.spawn().map_err(|_| InventoryError::GitUnavailable)
}

/// Lance les commandes d'une collecte sous un même échéancier.
struct Inspector {
    deadline: Instant,
}

impl Inspector {
    /// Ouvre une collecte bornée à son échéance.
    const fn bounded_by(deadline: Instant) -> Self {
        Self { deadline }
    }

    /// Lance une commande Git et consume sa sortie standard. Un processus qui dépasse l'échéance,
    /// ou dont la lecture est interrompue, est arrêté et attendu avant de rendre la main.
    async fn run<T, F, Fut>(
        &self,
        arguments: &[String],
        directory: Option<&Path>,
        accepted: &[i32],
        consume: F,
    ) -> Result<(Capture<T>, Option<i32>), InventoryError>
    where
        F: FnOnce(ChildStdout) -> Fut,
        Fut: Future<Output = Result<Capture<T>, InventoryError>>,
    {
        let mut child = spawn_git(arguments, directory)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(InventoryError::UnexpectedOutput)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(InventoryError::UnexpectedOutput)?;
        let mut diagnostics = tokio::spawn(read_up_to(stderr, MAX_STDERR_BYTES));

        let captured = tokio::time::timeout_at(self.deadline, consume(stdout))
            .await
            .unwrap_or(Err(InventoryError::DeadlineReached));
        let kill_first = match &captured {
            Ok(capture) => !capture.read_to_end,
            Err(_) => true,
        };
        let exited = tokio::time::timeout_at(self.deadline + CHILD_GRACE, async {
            if kill_first {
                let _ = child.kill().await;
            }
            child.wait().await
        })
        .await;
        let stderr_tail =
            match tokio::time::timeout_at(self.deadline + CHILD_GRACE, &mut diagnostics).await {
                Ok(Ok(Ok(capture))) => capture.value,
                Ok(Ok(Err(_))) | Ok(Err(_)) | Err(_) => {
                    diagnostics.abort();
                    Vec::new()
                }
            };
        let capture = captured?;
        // Un processus qui ne s'est pas terminé dans le délai de grâce laisse la collecte sans réponse.
        let Ok(Ok(status)) = exited else {
            return Err(InventoryError::DeadlineReached);
        };
        let code = status.code();
        if !code.is_some_and(|code| accepted.contains(&code)) {
            tracing::debug!(
                command = arguments.first().map(String::as_str).unwrap_or("git"),
                code = code.unwrap_or(-1),
                diagnostic_bytes = stderr_tail.len(),
                "commande Git refusée"
            );
            return Err(InventoryError::GitFailed);
        }
        if !capture.complete {
            return Err(InventoryError::StreamLimitReached);
        }
        Ok((capture, code))
    }

    /// Rend la sortie complète d'une commande, lue jusqu'à sa fin.
    async fn output(
        &self,
        arguments: &[String],
        directory: Option<&Path>,
    ) -> Result<Vec<u8>, InventoryError> {
        let (capture, _) = self
            .run(arguments, directory, &[0], |stdout| {
                read_up_to(stdout, MAX_LISTING_BYTES)
            })
            .await?;
        Ok(capture.value)
    }

    /// Rend si une commande `--quiet` a observé des changements ; `None` quand la réponse de Git
    /// ne permet pas de conclure.
    async fn changed(
        &self,
        arguments: &[String],
        directory: &Path,
    ) -> Result<Option<bool>, InventoryError> {
        match self
            .run(arguments, Some(directory), &[0, 1], |stdout| {
                read_up_to(stdout, 0)
            })
            .await
        {
            Ok((_, Some(0))) => Ok(Some(false)),
            Ok((_, Some(1))) => Ok(Some(true)),
            Ok((_, _)) => Ok(None),
            Err(InventoryError::DeadlineReached) => Err(InventoryError::DeadlineReached),
            Err(_) => Ok(None),
        }
    }

    /// Rend si une commande a listé au moins une entrée ; `None` quand elle a échoué sans rien
    /// rendre.
    async fn listed(
        &self,
        arguments: &[String],
        directory: &Path,
    ) -> Result<Option<bool>, InventoryError> {
        match self
            .run(arguments, Some(directory), &[0], |stdout| {
                read_up_to(stdout, MAX_LISTING_BYTES)
            })
            .await
        {
            Ok((capture, _)) => Ok(Some(!capture.value.is_empty())),
            Err(InventoryError::StreamLimitReached) => Ok(Some(true)),
            Err(InventoryError::DeadlineReached) => Err(InventoryError::DeadlineReached),
            Err(_) => Ok(None),
        }
    }

    /// Rend `true` dès qu'un sous-module est présent, ou quand la réponse de Git ne permet pas
    /// de l'exclure.
    async fn has_gitlinks(
        &self,
        arguments: &[String],
        directory: &Path,
    ) -> Result<bool, InventoryError> {
        match self
            .run(arguments, Some(directory), &[0], |stdout| {
                scan_gitlinks(stdout, MAX_STREAMED_BYTES)
            })
            .await
        {
            Ok((capture, _)) => Ok(capture.value),
            Err(InventoryError::DeadlineReached) => Err(InventoryError::DeadlineReached),
            Err(_) => Ok(true),
        }
    }

    async fn git_version(&self) -> Result<String, InventoryError> {
        let output = self
            .output(&arguments(&["--version"]), None)
            .await
            .map_err(|error| unless_deadline(error, InventoryError::VersionUnsupported))?;
        parse_git_version(&output)
    }

    async fn object_format(&self, root: &Path) -> Result<GitObjectFormat, InventoryError> {
        let output = self
            .output(
                &arguments(&["rev-parse", "--show-object-format"]),
                Some(root),
            )
            .await
            .map_err(|error| unless_deadline(error, InventoryError::UnreadableRepository))?;
        match std::str::from_utf8(&output)
            .map_err(|_| InventoryError::UnreadableRepository)?
            .trim()
        {
            "sha1" => Ok(GitObjectFormat::Sha1),
            "sha256" => Ok(GitObjectFormat::Sha256),
            _ => Err(InventoryError::UnreadableRepository),
        }
    }

    async fn worktree_listing(&self, root: &Path) -> Result<WorktreeListing, InventoryError> {
        let output = self
            .output(
                &arguments(&["worktree", "list", "--porcelain", "-z"]),
                Some(root),
            )
            .await
            .map_err(|error| unless_deadline(error, InventoryError::UnreadableRepository))?;
        parse_worktree_listing(&output)
    }

    /// Rend le répertoire Git privé d'un worktree ; `None` quand il n'a pas pu être établi.
    async fn private_gitdir(&self, worktree: &Path) -> Result<Option<PathBuf>, InventoryError> {
        match self
            .output(
                &arguments(&["rev-parse", "--absolute-git-dir"]),
                Some(worktree),
            )
            .await
        {
            Ok(output) => Ok(std::str::from_utf8(&output)
                .ok()
                .map(|text| PathBuf::from(text.trim_end_matches(['\r', '\n']).to_owned()))),
            Err(InventoryError::DeadlineReached) => Err(InventoryError::DeadlineReached),
            Err(_) => Ok(None),
        }
    }

    /// Surcharges rendant inertes les pilotes de filtre du worktree ; `None` quand ils n'ont pas
    /// pu être tous identifiés.
    async fn filters_of(&self, worktree: &Path) -> Result<Option<Vec<String>>, InventoryError> {
        let keys = arguments(&FILTER_KEYS);
        match self
            .run(&keys, Some(worktree), &[0, 1], |stdout| {
                read_up_to(stdout, MAX_LISTING_BYTES)
            })
            .await
        {
            Ok((capture, _)) => Ok(filter_overrides(&capture.value)),
            Err(InventoryError::DeadlineReached) => Err(InventoryError::DeadlineReached),
            Err(_) => Ok(None),
        }
    }

    /// Établit l'état `dirty` d'un worktree sans lire le contenu des sous-modules.
    async fn read_dirty(
        &self,
        worktree: &Path,
        unborn: bool,
    ) -> Result<WorktreeDirty, InventoryError> {
        let Some(overrides) = self.filters_of(worktree).await? else {
            return Ok(WorktreeDirty::Unknown);
        };
        let changed = if unborn {
            // Sans objet `HEAD`, tout ce que l'index retient est un ajout non engagé.
            let unstaged = self
                .changed(&with_overrides(&overrides, &DIFF_FILES), worktree)
                .await?;
            match unstaged {
                Some(true) => Some(true),
                Some(false) => {
                    self.listed(&arguments(&["ls-files", "--cached", "-z"]), worktree)
                        .await?
                }
                None => None,
            }
        } else {
            self.changed(&with_overrides(&overrides, &DIFF_INDEX), worktree)
                .await?
        };
        match changed {
            Some(true) => return Ok(WorktreeDirty::Dirty),
            None => return Ok(WorktreeDirty::Unknown),
            Some(false) => {}
        }
        let untracked = self
            .listed(&with_overrides(&overrides, &UNTRACKED), worktree)
            .await?;
        match untracked {
            Some(true) => return Ok(WorktreeDirty::Dirty),
            None => return Ok(WorktreeDirty::Unknown),
            Some(false) => {}
        }
        // Les sous-modules ne sont pas inspectés : leur contenu peut avoir changé sans que rien
        // ici ne le montre.
        if self
            .has_gitlinks(&with_overrides(&overrides, &STAGED_ENTRIES), worktree)
            .await?
        {
            return Ok(WorktreeDirty::Unknown);
        }
        Ok(WorktreeDirty::Clean)
    }

    async fn observe(
        &self,
        raw: &RawWorktree,
        object_format: GitObjectFormat,
        managed: Option<&HashSet<String>>,
    ) -> Result<WorktreeItem, InventoryError> {
        let worktree = Path::new(&raw.path);
        let head_oid = match GitObjectId::parse(object_format, &raw.head) {
            Ok(oid) if oid.is_null() => None,
            Ok(oid) => Some(oid),
            Err(_) => return Err(InventoryError::UnexpectedOutput),
        };
        let operations = match self.private_gitdir(worktree).await? {
            Some(gitdir) => tokio::task::spawn_blocking(move || detect_operations(&gitdir))
                .await
                .map_err(|_| InventoryError::GitFailed)?,
            None => OperationState::unreadable(),
        };
        let dirty = self.read_dirty(worktree, head_oid.is_none()).await?;
        Ok(WorktreeItem {
            path: raw.path.clone(),
            head_oid,
            branch_ref: raw.branch.clone(),
            dirty,
            operations: operations.operations,
            operations_complete: operations.complete,
            operation_branches: Vec::new(),
            operation_branches_complete: operations.reservations_known,
            ownership: classify_ownership(&raw.path, managed),
        })
    }
}

/// Collecte l'inventaire d'un dépôt rattaché. `managed` vaut `None` quand le registre des
/// allocations n'a pas pu être lu.
pub(crate) async fn collect(
    root: &Path,
    managed: Option<&HashSet<String>>,
) -> Result<WorktreeInventory, InventoryError> {
    ensure_repository_root(root)?;
    let inspector = Inspector::bounded_by(Instant::now() + INVENTORY_TIMEOUT);
    let version_label = inspector.git_version().await?;
    let object_format = inspector.object_format(root).await?;
    let git_profile_digest = profile_digest(&version_label, object_format)?;
    let listing = inspector.worktree_listing(root).await?;
    if listing.worktrees.len() > MAX_WORKTREES {
        return Err(InventoryError::StreamLimitReached);
    }
    let mut items = Vec::with_capacity(listing.worktrees.len());
    for raw in &listing.worktrees {
        items.push(inspector.observe(raw, object_format, managed).await?);
    }
    items.sort_by(|earlier, later| earlier.path.as_bytes().cmp(later.path.as_bytes()));
    let status = if listing.omitted || items.iter().any(WorktreeItem::is_uncertain) {
        InventoryCoverageStatus::Incomplete
    } else {
        InventoryCoverageStatus::Complete
    };
    Ok(WorktreeInventory {
        object_format,
        git_profile_digest,
        items,
        coverage: InventoryCoverage { status },
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    const OID: &str = "49412001df8ca2164f25d9d3d2ac39a061cc024f";

    fn raw(path: &str, branch: Option<&str>) -> RawWorktree {
        RawWorktree {
            path: path.to_owned(),
            head: String::from(OID),
            branch: branch.map(ToOwned::to_owned),
        }
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : Git 2.39 est le minimum qualifié (ADR-09).
    #[test]
    fn git_version_requires_the_qualified_minimum() {
        assert_eq!(
            parse_git_version(b"git version 2.39.5\n").unwrap(),
            "git version 2.39.5"
        );
        assert_eq!(
            parse_git_version(b"git version 2.39\n").unwrap(),
            "git version 2.39"
        );
        assert_eq!(
            parse_git_version(b"git version 2.53.0 (Apple Git-154)\n").unwrap(),
            "git version 2.53.0 (Apple Git-154)"
        );
        for refused in [
            &b"git version 2.38.3"[..],
            b"git version 1.9.1\n",
            b"git version\n",
            b"",
            b"2.39.5",
        ] {
            assert_eq!(
                parse_git_version(refused),
                Err(InventoryError::VersionUnsupported),
                "accepté : {refused:?}"
            );
        }
        assert_eq!(
            parse_git_version(&[0xff, 0xfe]),
            Err(InventoryError::VersionUnsupported)
        );
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : le profil Git effectif est épinglé dans l'empreinte.
    #[test]
    fn profile_digest_pins_version_format_and_profile() {
        let sha1 = profile_digest("git version 2.39.5", GitObjectFormat::Sha1).unwrap();
        assert_eq!(sha1.as_prefixed_str().len(), 71);
        assert_eq!(
            sha1,
            profile_digest("git version 2.39.5", GitObjectFormat::Sha1).unwrap()
        );
        assert_ne!(
            sha1,
            profile_digest("git version 2.39.5 (Apple Git-154)", GitObjectFormat::Sha1).unwrap()
        );
        assert_ne!(
            sha1,
            profile_digest("git version 2.39.5", GitObjectFormat::Sha256).unwrap()
        );
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : la liste des worktrees se lit au format machine NUL.
    #[test]
    fn worktree_listing_reads_the_nul_terminated_records_of_git() {
        let stdout = concat!(
            "worktree /r/main\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0branch refs/heads/integration\0\0",
            "worktree /r/linked\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0detached\0\0",
            "worktree /r/locked\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0branch refs/heads/locked\0locked added with --lock\0\0",
            "worktree /r/prunable\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0detached\0prunable gitdir file points to non-existent location\0\0",
            "worktree /r/unborn\0HEAD 0000000000000000000000000000000000000000\0branch refs/heads/brandnew\0\0",
        );
        let listing = parse_worktree_listing(stdout.as_bytes()).unwrap();
        assert!(!listing.omitted);
        assert_eq!(
            listing.worktrees,
            vec![
                raw("/r/main", Some("refs/heads/integration")),
                raw("/r/linked", None),
                raw("/r/locked", Some("refs/heads/locked")),
                raw("/r/prunable", None),
                RawWorktree {
                    path: String::from("/r/unborn"),
                    head: "0".repeat(40),
                    branch: Some(String::from("refs/heads/brandnew")),
                },
            ]
        );
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : un dépôt bare ne crée aucun item de HEAD détaché.
    #[test]
    fn worktree_listing_skips_bare_repositories() {
        let bare = concat!(
            "worktree /srv/bare.git\0bare\0\0",
            "worktree /srv/wt\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0detached\0\0",
        );
        let listing = parse_worktree_listing(bare.as_bytes()).unwrap();
        assert!(!listing.omitted);
        assert_eq!(listing.worktrees, vec![raw("/srv/wt", None)]);

        let listing = parse_worktree_listing(b"worktree /srv/bare.git\0bare\0\0").unwrap();
        assert!(listing.worktrees.is_empty());
        assert!(!listing.omitted);
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : ce que le contrat fermé ne peut pas porter est
    // omis, jamais inventé.
    #[test]
    fn worktree_listing_omits_what_the_closed_contract_cannot_represent() {
        // Chemin qui n'est pas une suite d'octets UTF-8 : le worktree est omis, jamais inventé.
        let mut foreign_path = b"worktree /r/caf".to_vec();
        foreign_path.push(0xe9);
        foreign_path.extend_from_slice(
            concat!(
                "\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0detached\0\0",
                "worktree /r/ok\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0detached\0\0",
            )
            .as_bytes(),
        );
        let listing = parse_worktree_listing(&foreign_path).unwrap();
        assert!(listing.omitted);
        assert_eq!(listing.worktrees, vec![raw("/r/ok", None)]);

        // `branch_ref` ne peut porter qu'un nom complet de branche locale.
        let foreign_branch = concat!(
            "worktree /r/x\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0branch refs/remotes/origin/main\0\0",
            "worktree /r/y\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0branch main\0\0",
        );
        let listing = parse_worktree_listing(foreign_branch.as_bytes()).unwrap();
        assert!(listing.worktrees.is_empty());
        assert!(listing.omitted);
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : une sortie Git hors format documenté n'est pas
    // interprétée.
    #[test]
    fn worktree_listing_refuses_undocumented_output() {
        for malformed in [
            &b"HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0"[..],
            b"worktree /r/a\0\0",
            b"worktree /r/a\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0branch refs/heads/a\0detached\0\0",
            b"worktree /r/a\0HEAD 49412001df8ca2164f25d9d3d2ac39a061cc024f\0branch refs/heads/a\0future-field 1\0\0",
            b"worktree /r/a\0HEAD \xff\0detached\0\0",
        ] {
            assert_eq!(
                parse_worktree_listing(malformed),
                Err(InventoryError::UnexpectedOutput),
                "accepté : {malformed:?}"
            );
        }
        assert_eq!(parse_worktree_listing(b""), Ok(WorktreeListing::default()));
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : un sous-module laisse l'état du worktree inconnu,
    // jamais propre.
    #[test]
    fn gitlink_scan_follows_record_boundaries_across_chunks() {
        let mut scan = GitlinkScan::default();
        scan.push(b"100644 49412001df8ca2164f25d9d3d2ac39a061cc024f 0\ta.txt\0160")
            .unwrap();
        assert!(!scan.found());
        scan.push(b"0000 49412001df8ca2164f25d9d3d2ac39a061cc024f 1\tvendor/lib\0")
            .unwrap();
        assert!(scan.found());

        let mut scan = GitlinkScan::default();
        scan.push(b"100644 160000 0\t160000 look-alike\0").unwrap();
        assert!(!scan.found());
        scan.push(b"100755 49412001df8ca2164f25d9d3d2ac39a061cc024f 0\ta b/c/d")
            .unwrap();
        scan.push(b"\0160000").unwrap();
        assert!(!scan.found());
        scan.push(b" 49412001df8ca2164f25d9d3d2ac39a061cc024f 0\tsub\0")
            .unwrap();
        assert!(scan.found());
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : la lecture d'un index reste bornée en mémoire.
    #[test]
    fn gitlink_scan_bounds_one_record() {
        let mut scan = GitlinkScan::default();
        assert_eq!(
            scan.push(&vec![b'x'; MAX_PATH_BYTES + 1]),
            Err(InventoryError::UnexpectedOutput)
        );
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : aucun filtre du dépôt n'est exécuté pendant la
    // lecture.
    #[test]
    fn filter_overrides_neutralize_every_driver_of_the_repository() {
        let overrides = filter_overrides(
            b"filter.lfs.clean\nfilter.lfs.smudge\nfilter.lfs.process\nfilter.lfs.required\nfilter.doc-pdf.clean\n",
        )
        .unwrap();
        let neutralized = |driver: &str| {
            [
                String::from("-c"),
                format!("filter.{driver}.clean="),
                String::from("-c"),
                format!("filter.{driver}.process="),
                String::from("-c"),
                format!("filter.{driver}.smudge="),
                String::from("-c"),
                format!("filter.{driver}.required=false"),
            ]
        };
        let mut expected = neutralized("doc-pdf").to_vec();
        expected.extend(neutralized("lfs"));
        assert_eq!(overrides, expected);
        assert_eq!(filter_overrides(b""), Some(Vec::new()));
        assert_eq!(
            filter_overrides(b"filter.lfs.clean\n").map(|list| list.len()),
            Some(8)
        );
        for refused in [
            &b"filter.a=b.clean"[..],
            b"filter.\n",
            b"filter.lfs\n",
            b"user.name\n",
            b"filter.l\xe9.clean\n",
        ] {
            assert_eq!(filter_overrides(refused), None, "accepté : {refused:?}");
        }
        let full = with_overrides(&overrides, &["status"]);
        assert_eq!(full.len(), overrides.len() + 1);
        assert_eq!(full.last().map(String::as_str), Some("status"));
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : les opérations en cours suivent les marqueurs
    // consultés par Git.
    #[test]
    fn operations_follow_the_markers_git_consults() {
        let known = OperationState {
            operations: Vec::new(),
            complete: true,
            reservations_known: true,
        };
        let empty = tempdir().unwrap();
        assert_eq!(detect_operations(empty.path()), known);

        std::fs::write(empty.path().join("MERGE_HEAD"), OID).unwrap();
        std::fs::create_dir(empty.path().join("rebase-merge")).unwrap();
        std::fs::write(empty.path().join("BISECT_LOG"), "git bisect start\n").unwrap();
        std::fs::write(empty.path().join("CHERRY_PICK_HEAD"), OID).unwrap();
        assert_eq!(
            detect_operations(empty.path()),
            OperationState {
                operations: vec![
                    GitOperation::Merge,
                    GitOperation::Rebase,
                    GitOperation::Bisect,
                    GitOperation::CherryPick,
                ],
                complete: true,
                reservations_known: false,
            }
        );

        // `git am` et `git revert` n'ont pas de valeur dans le contrat fermé.
        let am = tempdir().unwrap();
        std::fs::create_dir_all(am.path().join("rebase-apply/applying")).unwrap();
        assert_eq!(
            detect_operations(am.path()),
            OperationState {
                operations: Vec::new(),
                complete: false,
                reservations_known: true,
            }
        );
        let revert = tempdir().unwrap();
        std::fs::write(revert.path().join("REVERT_HEAD"), OID).unwrap();
        assert!(!detect_operations(revert.path()).complete);

        let rebase = tempdir().unwrap();
        std::fs::create_dir(rebase.path().join("rebase-apply")).unwrap();
        let state = detect_operations(rebase.path());
        assert_eq!(state.operations, vec![GitOperation::Rebase]);
        assert!(state.complete);
        assert!(!state.reservations_known);
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : seul le dépôt enregistré est lu, jamais un dépôt
    // parent.
    #[test]
    fn repository_root_is_read_at_the_registered_location_only() {
        let empty = tempdir().unwrap();
        assert_eq!(
            ensure_repository_root(empty.path()),
            Err(InventoryError::UnreadableRepository)
        );

        let main = tempdir().unwrap();
        std::fs::create_dir(main.path().join(".git")).unwrap();
        assert_eq!(ensure_repository_root(main.path()), Ok(()));

        let bare = tempdir().unwrap();
        for name in ["HEAD", "objects", "refs"] {
            std::fs::write(bare.path().join(name), "").unwrap();
        }
        assert_eq!(ensure_repository_root(bare.path()), Ok(()));
        std::fs::remove_file(bare.path().join("refs")).unwrap();
        assert_eq!(
            ensure_repository_root(bare.path()),
            Err(InventoryError::UnreadableRepository)
        );
    }

    // T-F010-001 / AC-F010-001 / REQ-GIT-001 : la propriété suit le registre des allocations.
    #[test]
    fn ownership_follows_the_allocation_registry() {
        let managed: HashSet<String> = [String::from("/w/managed")].into_iter().collect();
        assert_eq!(
            classify_ownership("/w/managed", Some(&managed)),
            WorktreeOwnership::Managed
        );
        assert_eq!(
            classify_ownership("/w/human", Some(&managed)),
            WorktreeOwnership::External
        );
        assert_eq!(
            classify_ownership("/w/managed", None),
            WorktreeOwnership::Unknown
        );
    }
}
