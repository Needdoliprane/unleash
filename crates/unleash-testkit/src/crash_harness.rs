//! Oracle d'effets externes sur disque local et harnais de crash par processus enfant.
//!
//! Fournit :
//! - [`ExternalEffectOracle`] : journal NDJSON atomique (`O_APPEND` + `fsync`) stocké dans un
//!   répertoire temporaire local (sous `/tmp`), totalement indépendant de la base de données cible.
//! - [`ChildCrashHarness`] et [`ChildCrashContext`] : exécution d'un processus enfant réel jusqu'à
//!   un point de coupure nommé (`C0`–`C12`), observation de sa mort par [`CrashKillMode::AbortProcess`]
//!   (`SIGABRT`) ou [`CrashKillMode::Sigkill`] (`SIGKILL`) via tube `stdout`/`stdin` sans aucun
//!   `sleep` de synchronisation.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use thiserror::Error;

use crate::cutpoints::{
    Cutpoint, CutpointAction, CutpointBarrier, CutpointController, CutpointError,
};
use crate::temp::{TempDirError, local_test_dir, validate_no_claude_component};

/// Nom du fichier journal atomique maintenu par [`ExternalEffectOracle`].
pub const EFFECTS_FILENAME: &str = "effects.ndjson";

const ENV_CRASH_CHILD: &str = "UNLEASH_CRASH_CHILD";
const ENV_CRASH_CUTPOINT: &str = "UNLEASH_CRASH_CUTPOINT";
const ENV_CRASH_HIT: &str = "UNLEASH_CRASH_HIT";
const ENV_CRASH_MODE: &str = "UNLEASH_CRASH_MODE";
const ENV_CRASH_ORACLE_DIR: &str = "UNLEASH_CRASH_ORACLE_DIR";

const REACHED_LINE_PREFIX: &str = "UNLEASH_CRASH_REACHED:";
const SIGKILL_BRIDGE_TIMEOUT: Duration = Duration::from_secs(30);

/// Erreur produite par [`ExternalEffectOracle`] ou [`ChildCrashHarness`].
#[derive(Debug, Error)]
pub enum CrashHarnessError {
    /// Erreur I/O lors de la lecture, écriture ou synchronisation du journal ou des tubes IPC.
    #[error("Erreur I/O du harnais de crash : {0}")]
    Io(#[from] std::io::Error),

    /// Erreur de création ou validation du répertoire temporaire local.
    #[error("Erreur répertoire temporaire du harnais : {0}")]
    TempDir(#[from] TempDirError),

    /// Erreur de sérialisation ou de désérialisation d'une entrée d'oracle.
    #[error("Erreur JSON de l'oracle externe : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur liée au contrôleur ou à la barrière de point de coupure.
    #[error("Erreur de point de coupure dans le harnais : {0}")]
    Cutpoint(#[from] CutpointError),

    /// Séquence incohérente détectée dans le journal de l'oracle externe.
    #[error("Séquence oracle corrompue : attendue {expected}, observée {actual}")]
    CorruptedSequence {
        /// Séquence attendue (monotone à partir de 1).
        expected: u64,
        /// Séquence observée sur disque.
        actual: u64,
    },

    /// Configuration enfant invalide ou incomplète dans les variables d'environnement.
    #[error("Configuration du processus enfant de crash invalide ({var}) : {reason}")]
    InvalidChildEnv {
        /// Variable d'environnement en défaut.
        var: &'static str,
        /// Détail de l'erreur.
        reason: String,
    },

    /// Le processus enfant s'est terminé sans atteindre le point de coupure attendu en mode `Sigkill`.
    #[error(
        "Le processus enfant n'a pas atteint {expected} avant sa sortie (status={status}, stderr={stderr})"
    )]
    CutpointNotReached {
        /// Point de coupure attendu.
        expected: Cutpoint,
        /// Statut de sortie du processus enfant.
        status: String,
        /// Sortie d'erreur capturée.
        stderr: String,
    },

    /// Le processus enfant s'est terminé normalement avec succès au lieu de crasher.
    #[error("Le processus enfant est sorti avec succès (code 0) au lieu de crasher sur {expected}")]
    UnexpectedCleanExit {
        /// Point de coupure où un crash était attendu.
        expected: Cutpoint,
    },
}

/// Données d'entrée pour enregistrer un nouvel effet dans [`ExternalEffectOracle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewExternalEffect {
    /// Clé logique ou d'idempotence de l'effet externe.
    pub effect_key: String,
    /// Catégorie de l'effet (ex. `"provider_call"`, `"git_push"`, `"file_write"`).
    pub kind: String,
    /// Horodatage simulé ou observé en millisecondes UTC.
    pub timestamp_ms: i64,
    /// Identifiant d'action corrélé éventuel.
    pub action_id: Option<String>,
    /// Indique si le receipt de retour vers l'orchestrateur a été perdu (scénario `C5`).
    pub receipt_lost: bool,
}

/// Enregistrement durable committé dans le journal atomique [`ExternalEffectOracle`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEffectRecord {
    /// Numéro de séquence monotone (indexé à partir de 1).
    pub sequence: u64,
    /// Clé logique ou d'idempotence de l'effet externe.
    pub effect_key: String,
    /// Catégorie d'effet externe.
    pub kind: String,
    /// Horodatage associé en millisecondes UTC.
    pub timestamp_ms: i64,
    /// Identifiant d'action corrélé éventuel.
    pub action_id: Option<String>,
    /// Identifiant de reçu émis par le service externe simulé.
    pub receipt_id: String,
    /// Vrai si le reçu a été perdu après accomplissement de l'effet (scénario `C5`).
    pub receipt_lost: bool,
}

/// Oracle externe indépendant conservant son propre compteur et journal d'effets sur disque local.
///
/// Chaque écriture ajoute une ligne JSON atomique et appelle `sync_all()` (`fsync`) avant retour.
/// Les lectures relisent systématiquement le fichier disque afin qu'un parent observe fidèlement
/// les effets réellement accomplis avant la mort d'un processus enfant.
pub struct ExternalEffectOracle {
    dir: PathBuf,
    log_path: PathBuf,
    write_lock: Mutex<()>,
}

impl fmt::Debug for ExternalEffectOracle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalEffectOracle")
            .field("dir", &self.dir)
            .field("log_path", &self.log_path)
            .finish_non_exhaustive()
    }
}

impl ExternalEffectOracle {
    /// Crée un répertoire temporaire local sous `/tmp` et y initialise un [`ExternalEffectOracle`].
    ///
    /// # Errors
    /// - [`CrashHarnessError::TempDir`] si la création sous `/tmp` échoue.
    /// - [`CrashHarnessError::Io`] si la synchronisation du répertoire échoue.
    pub fn create_in_temp() -> Result<(TempDir, Self), CrashHarnessError> {
        let dir = local_test_dir("oracle")?;
        let oracle = Self::open(dir.path())?;
        Ok((dir, oracle))
    }

    /// Ouvre (ou initialise) un oracle externe dans `dir` (jamais sous `.claude/`).
    ///
    /// # Errors
    /// - [`CrashHarnessError::TempDir`] si `dir` se trouve sous `.claude/` (`REQ-CORE-008`).
    /// - [`CrashHarnessError::Io`] si la création du répertoire échoue.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, CrashHarnessError> {
        let dir_path = dir.as_ref().to_path_buf();
        validate_no_claude_component(&dir_path)?;
        fs::create_dir_all(&dir_path)?;
        sync_dir_best_effort(&dir_path)?;
        let log_path = dir_path.join(EFFECTS_FILENAME);
        Ok(Self {
            dir: dir_path,
            log_path,
            write_lock: Mutex::new(()),
        })
    }

    /// Renvoie le chemin du répertoire hébergeant cet oracle.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Renvoie le chemin du fichier journal NDJSON de cet oracle.
    #[must_use]
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Enregistre un effet externe de façon atomique (`O_APPEND` + `sync_all`) et renvoie le reçu produit.
    ///
    /// # Errors
    /// - [`CrashHarnessError::Io`] ou [`CrashHarnessError::Json`] en cas d'erreur de persistance.
    /// - [`CrashHarnessError::CorruptedSequence`] si le journal existant présente une rupture de séquence.
    pub fn record_effect(
        &self,
        effect_key: impl Into<String>,
        kind: impl Into<String>,
        timestamp_ms: i64,
    ) -> Result<ExternalEffectRecord, CrashHarnessError> {
        self.record_effect_detailed(NewExternalEffect {
            effect_key: effect_key.into(),
            kind: kind.into(),
            timestamp_ms,
            action_id: None,
            receipt_lost: false,
        })
    }

    /// Enregistre un effet externe détaillé (avec `action_id` et indicateur `receipt_lost` pour `C5`)
    /// de façon atomique (`O_APPEND` + `sync_all`).
    ///
    /// # Errors
    /// - [`CrashHarnessError::Io`] ou [`CrashHarnessError::Json`] en cas d'erreur de persistance.
    /// - [`CrashHarnessError::CorruptedSequence`] si le journal existant présente une rupture de séquence.
    pub fn record_effect_detailed(
        &self,
        entry: NewExternalEffect,
    ) -> Result<ExternalEffectRecord, CrashHarnessError> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let existing = self.read_records_from_disk()?;
        let next_seq = u64::try_from(existing.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);

        let record = ExternalEffectRecord {
            sequence: next_seq,
            effect_key: entry.effect_key,
            kind: entry.kind,
            timestamp_ms: entry.timestamp_ms,
            action_id: entry.action_id,
            receipt_id: format!("oracle-rcpt-{next_seq}"),
            receipt_lost: entry.receipt_lost,
        };

        let mut line = serde_json::to_string(&record)?;
        line.push('\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        sync_dir_best_effort(&self.dir)?;

        Ok(record)
    }

    /// Relit l'intégralité des enregistrements committés depuis le disque.
    ///
    /// # Errors
    /// - [`CrashHarnessError::Io`] ou [`CrashHarnessError::Json`] si la lecture/décodage échoue.
    /// - [`CrashHarnessError::CorruptedSequence`] si les numéros de séquence ne sont pas contigus.
    pub fn records(&self) -> Result<Vec<ExternalEffectRecord>, CrashHarnessError> {
        self.read_records_from_disk()
    }

    /// Relit le compteur exact d'effets committés sur disque.
    ///
    /// # Errors
    /// - Propage les erreurs de [`Self::records`].
    pub fn count(&self) -> Result<u64, CrashHarnessError> {
        let recs = self.read_records_from_disk()?;
        Ok(u64::try_from(recs.len()).unwrap_or(u64::MAX))
    }

    /// Compte le nombre d'effets committés associés à la clé `effect_key`.
    ///
    /// # Errors
    /// - Propage les erreurs de [`Self::records`].
    pub fn count_for_key(&self, effect_key: &str) -> Result<u64, CrashHarnessError> {
        let recs = self.read_records_from_disk()?;
        let matching = recs.iter().filter(|r| r.effect_key == effect_key).count();
        Ok(u64::try_from(matching).unwrap_or(u64::MAX))
    }

    fn read_records_from_disk(&self) -> Result<Vec<ExternalEffectRecord>, CrashHarnessError> {
        if !self.log_path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&self.log_path)?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for (idx, line_res) in reader.lines().enumerate() {
            let line = line_res?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let record: ExternalEffectRecord = serde_json::from_str(trimmed)?;
            let expected = u64::try_from(idx).unwrap_or(u64::MAX).saturating_add(1);
            if record.sequence != expected {
                return Err(CrashHarnessError::CorruptedSequence {
                    expected,
                    actual: record.sequence,
                });
            }
            records.push(record);
        }
        Ok(records)
    }
}

fn sync_dir_best_effort(dir: &Path) -> Result<(), std::io::Error> {
    let dir_file = File::open(dir)?;
    match dir_file.sync_all() {
        Ok(()) => Ok(()),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(err) => Err(err),
    }
}

/// Méthode d'interruption brutale appliquée au processus enfant lorsqu'il atteint le point de coupure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashKillMode {
    /// Le processus enfant s'auto-interrompt via `std::process::abort()` (`SIGABRT`).
    AbortProcess,
    /// Le processus enfant signale son arrivée sur `stdout` et le parent le tue par `SIGKILL` (`Child::kill`).
    Sigkill,
}

impl CrashKillMode {
    const fn as_env_str(self) -> &'static str {
        match self {
            Self::AbortProcess => "abort",
            Self::Sigkill => "sigkill",
        }
    }

    fn from_env_str(s: &str) -> Result<Self, CrashHarnessError> {
        match s.trim() {
            "abort" => Ok(Self::AbortProcess),
            "sigkill" => Ok(Self::Sigkill),
            other => Err(CrashHarnessError::InvalidChildEnv {
                var: ENV_CRASH_MODE,
                reason: format!("mode inconnu '{other}'"),
            }),
        }
    }
}

/// Spécification d'exécution d'un processus enfant sous [`ChildCrashHarness`].
#[derive(Debug, Clone)]
pub struct ChildCrashSpec {
    /// Exécutable à lancer (binaire fixture ou `std::env::current_exe()`).
    pub program: PathBuf,
    /// Arguments passés en ligne de commande.
    pub args: Vec<String>,
    /// Variables d'environnement additionnelles.
    pub env: Vec<(String, String)>,
    /// Répertoire de travail éventuel du processus enfant.
    pub current_dir: Option<PathBuf>,
    /// Point de coupure cible.
    pub target_cutpoint: Cutpoint,
    /// Occurrence (indexée à partir de `1`) déclenchant le crash.
    pub target_hit: u64,
    /// Mode d'interruption forcée (`AbortProcess` ou `Sigkill`).
    pub kill_mode: CrashKillMode,
    /// Répertoire de l'oracle externe indépendant partagé avec le parent.
    pub oracle_dir: PathBuf,
}

/// Résultat observé par le parent après la mort du processus enfant.
#[derive(Debug, Clone)]
pub struct ChildCrashOutcome {
    /// Point de coupure ciblé.
    pub target_cutpoint: Cutpoint,
    /// Mode d'arrêt utilisé.
    pub kill_mode: CrashKillMode,
    /// Code de sortie si le processus s'est terminé par `exit`.
    pub exit_code: Option<i32>,
    /// Numéro de signal POSIX ayant tué le processus enfant (ex. `6` = `SIGABRT`, `9` = `SIGKILL`).
    pub signal: Option<i32>,
    /// Sortie standard capturée avant la mort du processus.
    pub stdout: String,
    /// Sortie d'erreur capturée.
    pub stderr: String,
    /// Répertoire de l'oracle externe pour relecture indépendante.
    pub oracle_dir: PathBuf,
}

impl ChildCrashOutcome {
    /// Indique si le processus enfant a été tué par un signal POSIX.
    #[must_use]
    pub const fn killed_by_signal(&self) -> bool {
        self.signal.is_some()
    }

    /// Indique si le signal observé est `SIGABRT` (signal 6 sur Unix).
    #[must_use]
    pub const fn is_sigabrt(&self) -> bool {
        matches!(self.signal, Some(6))
    }

    /// Indique si le signal observé est `SIGKILL` (signal 9 sur Unix).
    #[must_use]
    pub const fn is_sigkill(&self) -> bool {
        matches!(self.signal, Some(9))
    }

    /// Ouvre une vue fraîche de l'[`ExternalEffectOracle`] sur disque après la mort du processus.
    ///
    /// # Errors
    /// - Propage les erreurs de [`ExternalEffectOracle::open`].
    pub fn oracle(&self) -> Result<ExternalEffectOracle, CrashHarnessError> {
        ExternalEffectOracle::open(&self.oracle_dir)
    }

    /// Relit directement depuis le disque le nombre d'effets externes committés après le crash.
    ///
    /// # Errors
    /// - Propage les erreurs de [`ExternalEffectOracle::open`] et [`ExternalEffectOracle::count`].
    pub fn oracle_count(&self) -> Result<u64, CrashHarnessError> {
        self.oracle()?.count()
    }
}

/// Harnais parent lançant un processus enfant réel jusqu'à un cutpoint et vérifiant sa mort sans `sleep`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ChildCrashHarness;

impl ChildCrashHarness {
    /// Construit une [`ChildCrashSpec`] ré-exécutant le binaire de test courant sur `test_name --exact`.
    ///
    /// # Errors
    /// - [`CrashHarnessError::Io`] si `std::env::current_exe()` échoue.
    pub fn for_current_exe_test(
        test_name: &str,
        target_cutpoint: Cutpoint,
        kill_mode: CrashKillMode,
        oracle_dir: PathBuf,
    ) -> Result<ChildCrashSpec, CrashHarnessError> {
        let program = std::env::current_exe()?;
        Ok(ChildCrashSpec {
            program,
            args: vec![
                test_name.to_owned(),
                String::from("--exact"),
                String::from("--nocapture"),
            ],
            env: Vec::new(),
            current_dir: None,
            target_cutpoint,
            target_hit: 1,
            kill_mode,
            oracle_dir,
        })
    }

    /// Lance le processus enfant décrit par `spec`, synchronise son arrivée sans aucun `sleep`
    /// (par `waitpid` en mode `AbortProcess` ou par tube `stdout` + `Child::kill()` en mode `Sigkill`),
    /// et retourne le statut d'arrêt observé.
    ///
    /// # Errors
    /// - [`CrashHarnessError::TempDir`] si `spec.oracle_dir` touche `.claude/`.
    /// - [`CrashHarnessError::CutpointNotReached`] si l'enfant termine en mode `Sigkill` sans signaler le cutpoint.
    /// - [`CrashHarnessError::UnexpectedCleanExit`] si l'enfant termine avec succès (code 0).
    /// - [`CrashHarnessError::Io`] pour toute erreur de spawn ou d'I/O sur les tubes.
    pub fn run_until_crash(spec: &ChildCrashSpec) -> Result<ChildCrashOutcome, CrashHarnessError> {
        validate_no_claude_component(&spec.oracle_dir)?;
        let hit_index = spec.target_hit.max(1);

        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env(ENV_CRASH_CHILD, "1")
            .env(ENV_CRASH_CUTPOINT, spec.target_cutpoint.as_str())
            .env(ENV_CRASH_HIT, hit_index.to_string())
            .env(ENV_CRASH_MODE, spec.kill_mode.as_env_str())
            .env(ENV_CRASH_ORACLE_DIR, &spec.oracle_dir);

        if let Some(dir) = &spec.current_dir {
            cmd.current_dir(dir);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;
        let stdin_keepalive = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("stdout pipe manquant sur le processus enfant"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("stderr pipe manquant sur le processus enfant"))?;

        let stderr_handle = std::thread::spawn(move || -> Result<String, std::io::Error> {
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            reader.read_to_string(&mut buf)?;
            Ok(buf)
        });

        let expected_marker = format!("{REACHED_LINE_PREFIX}{}", spec.target_cutpoint);
        let mut stdout_log = String::new();
        let mut reached_marker = false;
        let mut stdout_reader = BufReader::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            let n = match stdout_reader.read_line(&mut line) {
                Ok(n) => n,
                Err(err) => {
                    kill_and_reap_on_error(&mut child, stdin_keepalive, stderr_handle);
                    return Err(CrashHarnessError::Io(err));
                }
            };
            if n == 0 {
                break;
            }
            stdout_log.push_str(&line);
            if spec.kill_mode == CrashKillMode::Sigkill && line.trim() == expected_marker {
                reached_marker = true;
                if let Err(err) = child.kill() {
                    kill_and_reap_on_error(&mut child, stdin_keepalive, stderr_handle);
                    return Err(CrashHarnessError::Io(err));
                }
                break;
            }
        }

        let status = child.wait()?;
        drop(stdin_keepalive);

        let stderr_log = match stderr_handle.join() {
            Ok(Ok(s)) => s,
            Ok(Err(err)) => return Err(CrashHarnessError::Io(err)),
            Err(_) => String::from("<stderr thread panicked>"),
        };

        if spec.kill_mode == CrashKillMode::Sigkill && !reached_marker {
            return Err(CrashHarnessError::CutpointNotReached {
                expected: spec.target_cutpoint,
                status: status.to_string(),
                stderr: stderr_log,
            });
        }

        if status.success() {
            return Err(CrashHarnessError::UnexpectedCleanExit {
                expected: spec.target_cutpoint,
            });
        }

        let signal = extract_signal(status);
        Ok(ChildCrashOutcome {
            target_cutpoint: spec.target_cutpoint,
            kill_mode: spec.kill_mode,
            exit_code: status.code(),
            signal,
            stdout: stdout_log,
            stderr: stderr_log,
            oracle_dir: spec.oracle_dir.clone(),
        })
    }
}

fn kill_and_reap_on_error(
    child: &mut Child,
    stdin_keepalive: Option<std::process::ChildStdin>,
    stderr_handle: JoinHandle<Result<String, std::io::Error>>,
) {
    let _ = child.kill();
    let _ = child.wait();
    drop(stdin_keepalive);
    let _ = stderr_handle.join();
}

fn extract_signal(status: ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

/// Garde de pont entre un [`CutpointController`] dans le processus enfant et le harnais parent
/// en mode [`CrashKillMode::Sigkill`].
///
/// Lorsque la garde est détruite sans que le cutpoint cible n'ait été atteint, elle annule la
/// barrière et rejoint proprement le thread relais sans laisser de thread orphelin.
#[derive(Debug)]
pub struct ChildCrashBridge {
    barrier: std::sync::Arc<CutpointBarrier>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for ChildCrashBridge {
    fn drop(&mut self) {
        self.barrier.abort();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Contexte lu côté processus enfant lorsque celui-ci est lancé par [`ChildCrashHarness`].
#[derive(Debug)]
pub struct ChildCrashContext {
    target_cutpoint: Cutpoint,
    target_hit: u64,
    kill_mode: CrashKillMode,
    oracle: ExternalEffectOracle,
}

impl ChildCrashContext {
    /// Inspecte l'environnement du processus courant et renvoie `Ok(Some(ctx))` si
    /// `UNLEASH_CRASH_CHILD=1` est actif, ou `Ok(None)` lors d'une exécution normale.
    ///
    /// # Errors
    /// - [`CrashHarnessError::InvalidChildEnv`] si les variables du harnais sont mal formées.
    /// - [`CrashHarnessError::TempDir`] ou [`CrashHarnessError::Io`] si l'oracle ne peut être ouvert.
    pub fn from_env() -> Result<Option<Self>, CrashHarnessError> {
        let Ok(active) = std::env::var(ENV_CRASH_CHILD) else {
            return Ok(None);
        };
        if active.trim() != "1" {
            return Ok(None);
        }

        let cp_raw =
            std::env::var(ENV_CRASH_CUTPOINT).map_err(|_| CrashHarnessError::InvalidChildEnv {
                var: ENV_CRASH_CUTPOINT,
                reason: String::from("variable absente"),
            })?;
        let target_cutpoint =
            cp_raw
                .parse::<Cutpoint>()
                .map_err(|err| CrashHarnessError::InvalidChildEnv {
                    var: ENV_CRASH_CUTPOINT,
                    reason: err.to_string(),
                })?;

        let hit_raw = std::env::var(ENV_CRASH_HIT).unwrap_or_else(|_| String::from("1"));
        let target_hit =
            hit_raw
                .trim()
                .parse::<u64>()
                .map_err(|err| CrashHarnessError::InvalidChildEnv {
                    var: ENV_CRASH_HIT,
                    reason: err.to_string(),
                })?;
        if target_hit == 0 {
            return Err(CrashHarnessError::InvalidChildEnv {
                var: ENV_CRASH_HIT,
                reason: String::from("l'occurrence doit être >= 1"),
            });
        }

        let mode_raw =
            std::env::var(ENV_CRASH_MODE).map_err(|_| CrashHarnessError::InvalidChildEnv {
                var: ENV_CRASH_MODE,
                reason: String::from("variable absente"),
            })?;
        let kill_mode = CrashKillMode::from_env_str(&mode_raw)?;

        let oracle_dir =
            std::env::var_os(ENV_CRASH_ORACLE_DIR).ok_or(CrashHarnessError::InvalidChildEnv {
                var: ENV_CRASH_ORACLE_DIR,
                reason: String::from("variable absente"),
            })?;
        let oracle = ExternalEffectOracle::open(PathBuf::from(oracle_dir))?;

        Ok(Some(Self {
            target_cutpoint,
            target_hit,
            kill_mode,
            oracle,
        }))
    }

    /// Renvoie le point de coupure ciblé par ce processus enfant.
    #[must_use]
    pub const fn target_cutpoint(&self) -> Cutpoint {
        self.target_cutpoint
    }

    /// Renvoie le numéro d'occurrence (1-based) configuré.
    #[must_use]
    pub const fn target_hit(&self) -> u64 {
        self.target_hit
    }

    /// Renvoie le mode de crash demandé.
    #[must_use]
    pub const fn kill_mode(&self) -> CrashKillMode {
        self.kill_mode
    }

    /// Renvoie une référence vers l'[`ExternalEffectOracle`] partagé avec le parent.
    #[must_use]
    pub const fn oracle(&self) -> &ExternalEffectOracle {
        &self.oracle
    }

    /// Arme `controller` pour déclencher automatiquement le crash configuré (`AbortProcess` ou
    /// barrière synchronisée + notification `SIGKILL`) au passage du point ciblé, sans aucun `sleep`.
    ///
    /// # Errors
    /// - [`CrashHarnessError::Cutpoint`] si `target_hit` est invalide.
    pub fn arm_controller(
        &self,
        controller: &CutpointController,
    ) -> Result<Option<ChildCrashBridge>, CrashHarnessError> {
        match self.kill_mode {
            CrashKillMode::AbortProcess => {
                controller.set_action_on_hit(
                    self.target_cutpoint,
                    self.target_hit,
                    CutpointAction::AbortProcess,
                )?;
                Ok(None)
            }
            CrashKillMode::Sigkill => {
                let barrier = controller.arm_barrier_on_hit(
                    self.target_cutpoint,
                    self.target_hit,
                    SIGKILL_BRIDGE_TIMEOUT,
                )?;
                let barrier_for_thread = std::sync::Arc::clone(&barrier);
                let cutpoint = self.target_cutpoint;
                let handle = std::thread::spawn(move || {
                    if barrier_for_thread
                        .wait_arrived(1, SIGKILL_BRIDGE_TIMEOUT)
                        .is_ok()
                    {
                        notify_parent_and_await_sigkill(cutpoint);
                    }
                });
                Ok(Some(ChildCrashBridge {
                    barrier,
                    handle: Some(handle),
                }))
            }
        }
    }
}

fn notify_parent_and_await_sigkill(cutpoint: Cutpoint) {
    let marker = format!("{REACHED_LINE_PREFIX}{cutpoint}\n");
    let mut stdout = std::io::stdout();
    if stdout.write_all(marker.as_bytes()).is_ok() && stdout.flush().is_ok() {
        let mut buf = [0u8; 1];
        let _ = std::io::stdin().read(&mut buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oracle_rejects_claude_directory() {
        let err = ExternalEffectOracle::open(Path::new("/tmp/.claude/runtime-oracle"));
        assert!(matches!(
            err,
            Err(CrashHarnessError::TempDir(
                TempDirError::ForbiddenDocumentationPath { .. }
            ))
        ));
    }

    #[test]
    fn test_oracle_atomic_append_reopens_cleanly() {
        let (_dir, oracle) = ExternalEffectOracle::create_in_temp().unwrap();
        let rec1 = oracle
            .record_effect("cmd-1", "provider_call", 1_000)
            .unwrap();
        assert_eq!(rec1.sequence, 1);

        let rec2 = oracle
            .record_effect_detailed(NewExternalEffect {
                effect_key: String::from("cmd-2"),
                kind: String::from("provider_call"),
                timestamp_ms: 2_000,
                action_id: Some(String::from("act-2")),
                receipt_lost: true,
            })
            .unwrap();
        assert_eq!(rec2.sequence, 2);
        assert!(rec2.receipt_lost);

        let reopened = ExternalEffectOracle::open(oracle.dir()).unwrap();
        assert_eq!(reopened.count().unwrap(), 2);
        assert_eq!(reopened.count_for_key("cmd-1").unwrap(), 1);
        assert_eq!(reopened.count_for_key("cmd-2").unwrap(), 1);
    }
}
