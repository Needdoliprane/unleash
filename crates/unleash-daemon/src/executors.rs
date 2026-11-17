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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use unleash_platform::{BootIdentity, current_process_identity};
    use unleash_testkit::local_test_dir;

    use super::*;

    fn mk_rdv(tag: &str) -> tempfile::TempDir {
        let dir = local_test_dir(tag).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn upsert_registration_creates_0600_manifest_and_is_idempotent() {
        let rdv = mk_rdv("exec-upsert");
        let root_a = local_test_dir("exec-root-a").unwrap();

        RendezvousExecutorController::upsert_registration(
            rdv.path(),
            ExecutorRegistration {
                executor_id: String::from("host-1"),
                data_root: root_a.path().to_path_buf(),
                pid: Some(1234),
                boot_identity: None,
                start_identity: None,
                os_identity: None,
                state: ExecutorState::Active,
            },
        )
        .unwrap();

        let manifest_path = rdv.path().join(HOSTS_MANIFEST_FILENAME);
        let mode = std::fs::metadata(&manifest_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        RendezvousExecutorController::upsert_registration(
            rdv.path(),
            ExecutorRegistration {
                executor_id: String::from("host-1"),
                data_root: root_a.path().to_path_buf(),
                pid: Some(1234),
                boot_identity: None,
                start_identity: None,
                os_identity: None,
                state: ExecutorState::Neutralized,
            },
        )
        .unwrap();

        let loaded = load_hosts_manifest(&manifest_path).unwrap();
        assert_eq!(loaded.executors.len(), 1);
        assert_eq!(loaded.executors[0].state, ExecutorState::Neutralized);
    }

    #[test]
    fn active_hosts_for_other_root_blocks_live_host_and_auto_neutralizes_stale_host() {
        let rdv = mk_rdv("exec-other-root");
        let root_old = local_test_dir("exec-old").unwrap();
        let root_new = local_test_dir("exec-new").unwrap();
        let ctrl = RendezvousExecutorController;

        assert!(
            ctrl.active_hosts_for_other_root(rdv.path(), root_new.path())
                .unwrap()
                .is_none()
        );

        let mut stale_os = current_process_identity().unwrap();
        stale_os.boot_identity = BootIdentity::new("stale-boot-identity-before-reboot").unwrap();

        RendezvousExecutorController::upsert_registration(
            rdv.path(),
            ExecutorRegistration {
                executor_id: String::from("host-stale"),
                data_root: root_old.path().to_path_buf(),
                pid: Some(stale_os.pid),
                boot_identity: None,
                start_identity: None,
                os_identity: Some(stale_os),
                state: ExecutorState::Active,
            },
        )
        .unwrap();

        // Le processus identifié sur un autre boot est auto-neutralisé sans bloquer le changement.
        assert!(
            ctrl.active_hosts_for_other_root(rdv.path(), root_new.path())
                .unwrap()
                .is_none()
        );
        let manifest_after_stale =
            load_hosts_manifest(&rdv.path().join(HOSTS_MANIFEST_FILENAME)).unwrap();
        assert_eq!(
            manifest_after_stale.executors[0].state,
            ExecutorState::Neutralized
        );

        // Un processus vivant identifié sur une autre racine bloque le basculement.
        let live_os = current_process_identity().unwrap();
        RendezvousExecutorController::upsert_registration(
            rdv.path(),
            ExecutorRegistration {
                executor_id: String::from("host-live"),
                data_root: root_old.path().to_path_buf(),
                pid: Some(live_os.pid),
                boot_identity: Some(live_os.boot_identity.to_string()),
                start_identity: Some(live_os.start_identity.to_string()),
                os_identity: Some(live_os),
                state: ExecutorState::Active,
            },
        )
        .unwrap();

        let blocked = ctrl
            .active_hosts_for_other_root(rdv.path(), root_new.path())
            .unwrap()
            .expect("hôte actif sur autre racine doit bloquer");
        assert_eq!(blocked.1, vec![String::from("host-live")]);

        // Même racine ne bloque pas elle-même.
        assert!(
            ctrl.active_hosts_for_other_root(rdv.path(), root_old.path())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn drain_on_stop_classifies_complete_partial_unknown_and_corrupt_manifest() {
        let rdv = mk_rdv("exec-drain");
        let root = local_test_dir("exec-drain-root").unwrap();
        let ctrl = RendezvousExecutorController;

        let empty_rep = ctrl.drain_on_stop(rdv.path(), Duration::from_secs(1));
        assert_eq!(empty_rep.outcome, StopOutcome::Complete);
        assert!(empty_rep.retained_services.is_empty());
        assert!(empty_rep.unknown_services.is_empty());

        RendezvousExecutorController::upsert_registration(
            rdv.path(),
            ExecutorRegistration {
                executor_id: String::from("svc-active"),
                data_root: root.path().to_path_buf(),
                pid: None,
                boot_identity: None,
                start_identity: None,
                os_identity: None,
                state: ExecutorState::Active,
            },
        )
        .unwrap();
        let partial_rep = ctrl.drain_on_stop(rdv.path(), Duration::from_secs(1));
        assert_eq!(partial_rep.outcome, StopOutcome::Partial);
        assert_eq!(
            partial_rep.retained_services,
            vec![String::from("svc-active")]
        );
        assert!(partial_rep.unknown_services.is_empty());

        RendezvousExecutorController::upsert_registration(
            rdv.path(),
            ExecutorRegistration {
                executor_id: String::from("svc-unknown"),
                data_root: root.path().to_path_buf(),
                pid: None,
                boot_identity: None,
                start_identity: None,
                os_identity: None,
                state: ExecutorState::Unknown,
            },
        )
        .unwrap();
        let unknown_rep = ctrl.drain_on_stop(rdv.path(), Duration::from_secs(1));
        assert_eq!(unknown_rep.outcome, StopOutcome::Unknown);
        assert_eq!(
            unknown_rep.retained_services,
            vec![String::from("svc-active")]
        );
        assert_eq!(
            unknown_rep.unknown_services,
            vec![String::from("svc-unknown")]
        );

        let manifest_path = rdv.path().join(HOSTS_MANIFEST_FILENAME);
        write_private_file_atomic(&manifest_path, b"{invalid-json").unwrap();
        let corrupt_rep = ctrl.drain_on_stop(rdv.path(), Duration::from_secs(1));
        assert_eq!(corrupt_rep.outcome, StopOutcome::Unknown);
        assert_eq!(
            corrupt_rep.unknown_services,
            vec![String::from("hosts_manifest_unreadable")]
        );
    }
}
