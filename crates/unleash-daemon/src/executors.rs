// SPDX-License-Identifier: Apache-2.0
//! Inventaire et drainage des exécuteurs persistants dans le répertoire de rendez-vous (`0600`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use unleash_domain::StopOutcome;
use unleash_platform::{
    OsProcessIdentity, ProcessVerification, read_private_file_to_string, verify_process_identity,
    write_private_file_atomic,
};
use unleash_protocol::from_json_str_strict;

use crate::error::DaemonError;

const HOSTS_MANIFEST_FILENAME: &str = "hosts.json";

/// État d'un exécuteur ou host enregistré sous l'autorité locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorState {
    /// Actif et rattaché à l'autorité courante.
    Active,
    /// Neutralisé proprement.
    Neutralized,
    /// État final non observable (`OutcomeUnknown`).
    Unknown,
}

/// Entrée d'exécuteur persistée dans le registre privé du rendez-vous (`0600`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRegistration {
    /// Identifiant logique de l'exécuteur.
    pub executor_id: String,
    /// Racine de données à laquelle l'exécuteur est rattaché.
    pub data_root: PathBuf,
    /// PID OS optionnel.
    pub pid: Option<u32>,
    /// Identité de boot OS au lancement.
    pub boot_identity: Option<String>,
    /// Identité de démarrage OS du PID.
    pub start_identity: Option<String>,
    /// Identité complète OS si connue.
    pub os_identity: Option<OsProcessIdentity>,
    /// État courant.
    pub state: ExecutorState,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostsManifest {
    executors: Vec<ExecutorRegistration>,
}

/// Rapport final produit lors du drainage d'arrêt du daemon (`DaemonStopped`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainReport {
    /// Résultat agrégé (`complete`, `partial` ou `unknown`).
    pub outcome: StopOutcome,
    /// Services conservés ou encore actifs dans leur contrat.
    pub retained_services: Vec<String>,
    /// Services dont l'état final n'a pas pu être confirmé.
    pub unknown_services: Vec<String>,
}

/// Port abstrait de gestion et de drainage des exécuteurs/process-hosts.
pub trait ExecutorDrainPort: Send + Sync {
    /// Vérifie s'il existe des exécuteurs non neutralisés rattachés à une autre racine que `requested_root`.
    ///
    /// # Errors
    /// Retourne [`DaemonError`] en cas d'échec d'accès au registre privé.
    fn active_hosts_for_other_root(
        &self,
        rendezvous_dir: &Path,
        requested_root: &Path,
    ) -> Result<Option<(PathBuf, Vec<String>)>, DaemonError>;

    /// Tente de drainer et neutraliser les services actifs dans la limite de `timeout`.
    fn drain_on_stop(&self, rendezvous_dir: &Path, timeout: Duration) -> DrainReport;

    /// Nombre de dispatches réellement exécutés (utile pour les doubles espions en test).
    fn dispatch_count(&self) -> u64 {
        0
    }
}

/// Contrôleur par défaut s'appuyant sur le manifeste privé `hosts.json` du rendez-vous.
#[derive(Debug, Default, Clone, Copy)]
pub struct RendezvousExecutorController;

impl RendezvousExecutorController {
    /// Enregistre ou met à jour un exécuteur dans `rendezvous_dir/hosts.json` (`0600`).
    ///
    /// # Errors
    /// Retourne [`DaemonError`] en cas d'erreur de chemin `.claude/` ou d'I/O atomique.
    pub fn upsert_registration(
        rendezvous_dir: &Path,
        reg: ExecutorRegistration,
    ) -> Result<(), DaemonError> {
        let path = rendezvous_dir.join(HOSTS_MANIFEST_FILENAME);
        let mut manifest = load_hosts_manifest(&path)?;
        if let Some(existing) = manifest
            .executors
            .iter_mut()
            .find(|e| e.executor_id == reg.executor_id)
        {
            *existing = reg;
        } else {
            manifest.executors.push(reg);
        }
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        write_private_file_atomic(&path, &bytes)?;
        Ok(())
    }
}

impl ExecutorDrainPort for RendezvousExecutorController {
    fn active_hosts_for_other_root(
        &self,
        rendezvous_dir: &Path,
        requested_root: &Path,
    ) -> Result<Option<(PathBuf, Vec<String>)>, DaemonError> {
        let path = rendezvous_dir.join(HOSTS_MANIFEST_FILENAME);
        let mut manifest = load_hosts_manifest(&path)?;
        let requested_canon =
            std::fs::canonicalize(requested_root).unwrap_or_else(|_| requested_root.to_path_buf());

        let mut changed = false;
        let mut blocking_root: Option<PathBuf> = None;
        let mut blocking_ids = Vec::new();

        for exec in &mut manifest.executors {
            if exec.state == ExecutorState::Neutralized {
                continue;
            }
            let exec_canon =
                std::fs::canonicalize(&exec.data_root).unwrap_or_else(|_| exec.data_root.clone());
            if exec_canon == requested_canon {
                continue;
            }
            let still_alive = match &exec.os_identity {
                Some(os_id) => matches!(
                    verify_process_identity(os_id),
                    Ok(ProcessVerification::Match)
                ),
                None => exec.state == ExecutorState::Active,
            };
            if still_alive {
                if blocking_root.is_none() {
                    blocking_root = Some(exec_canon);
                }
                blocking_ids.push(exec.executor_id.clone());
            } else {
                exec.state = ExecutorState::Neutralized;
                changed = true;
            }
        }

        if changed {
            let bytes = serde_json::to_vec_pretty(&manifest)?;
            write_private_file_atomic(&path, &bytes)?;
        }

        if let Some(root) = blocking_root
            && !blocking_ids.is_empty()
        {
            return Ok(Some((root, blocking_ids)));
        }
        Ok(None)
    }

    fn drain_on_stop(&self, rendezvous_dir: &Path, _timeout: Duration) -> DrainReport {
        let path = rendezvous_dir.join(HOSTS_MANIFEST_FILENAME);
        let Ok(manifest) = load_hosts_manifest(&path) else {
            return DrainReport {
                outcome: StopOutcome::Unknown,
                retained_services: Vec::new(),
                unknown_services: vec![String::from("hosts_manifest_unreadable")],
            };
        };

        let mut retained = Vec::new();
        let mut unknown = Vec::new();
        for exec in manifest.executors {
            match exec.state {
                ExecutorState::Neutralized => {}
                ExecutorState::Active => retained.push(exec.executor_id),
                ExecutorState::Unknown => unknown.push(exec.executor_id),
            }
        }

        let outcome = if !unknown.is_empty() {
            StopOutcome::Unknown
        } else if !retained.is_empty() {
            StopOutcome::Partial
        } else {
            StopOutcome::Complete
        };

        DrainReport {
            outcome,
            retained_services: retained,
            unknown_services: unknown,
        }
    }
}

fn load_hosts_manifest(path: &Path) -> Result<HostsManifest, DaemonError> {
    match read_private_file_to_string(path)? {
        Some(raw) => from_json_str_strict::<HostsManifest>(&raw).map_err(DaemonError::from),
        None => Ok(HostsManifest::default()),
    }
}
