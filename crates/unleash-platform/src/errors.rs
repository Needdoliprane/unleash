// SPDX-License-Identifier: Apache-2.0
//! Erreurs typées de la couche d'adaptation OS `unleash-platform`.

use std::path::PathBuf;
use thiserror::Error;

use crate::paths::PlatformOsKind;

/// Erreur produite par les adaptateurs système de `unleash-platform`.
///
/// Aucune variante ne doit jamais contenir de valeur secrète en clair.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PlatformError {
    /// Chemin interdit dans le dossier documentaire `.claude/` (`REQ-CORE-008`, `INV-046`).
    #[error("Chemin runtime interdit sous '.claude/' (REQ-CORE-008) : '{path}'")]
    ForbiddenDocumentationPath {
        /// Chemin rejeté.
        path: String,
    },

    /// Chemin invalide ou non canonisable.
    #[error("Chemin système invalide '{path}' : {reason}")]
    InvalidPath {
        /// Chemin rejeté.
        path: String,
        /// Motif du rejet.
        reason: String,
    },

    /// Chemin de socket Unix dépassant la limite OS `SUN_LEN` (103 octets max sur macOS).
    #[error("Chemin de socket Unix trop long ({len} octets, max {max}) : '{path}'")]
    SocketPathTooLong {
        /// Chemin rejeté.
        path: String,
        /// Longueur en octets observée.
        len: usize,
        /// Plafond admissible en octets.
        max: usize,
    },

    /// Lien symbolique interdit pour un répertoire de rendez-vous, un verrou ou un socket.
    #[error("Lien symbolique interdit pour le chemin sécurisé : '{path}'")]
    SymlinkNotAllowed {
        /// Chemin contenant un lien symbolique non autorisé.
        path: String,
    },

    /// Permissions POSIX insuffisamment restrictives.
    #[error(
        "Permissions non conformes sur '{path}' : attendu {expected_mode:#o}, observé {actual_mode:#o}"
    )]
    InsecurePermissions {
        /// Chemin inspecté.
        path: String,
        /// Mode POSIX attendu (ex: `0o700` ou `0o600`).
        expected_mode: u32,
        /// Mode POSIX réellement observé.
        actual_mode: u32,
    },

    /// Propriétaire POSIX différent de l'UID effectif courant.
    #[error(
        "Propriétaire invalide sur '{path}' : UID attendu {expected_uid}, UID observé {actual_uid}"
    )]
    OwnershipMismatch {
        /// Chemin inspecté.
        path: String,
        /// UID effectif attendu.
        expected_uid: u32,
        /// UID propriétaire observé.
        actual_uid: u32,
    },

    /// L'objet visé n'est pas un répertoire.
    #[error("Le chemin attendu comme répertoire n'en est pas un : '{path}'")]
    NotADirectory {
        /// Chemin inspecté.
        path: String,
    },

    /// L'objet visé n'est pas un fichier régulier.
    #[error("Le chemin attendu comme fichier régulier n'en est pas un : '{path}'")]
    NotARegularFile {
        /// Chemin inspecté.
        path: String,
    },

    /// L'objet visé n'est pas un socket Unix.
    #[error("Le chemin attendu comme socket Unix n'en est pas un : '{path}'")]
    NotASocket {
        /// Chemin inspecté.
        path: String,
    },

    /// Racine runtime requise par le rendez-vous canonique par défaut absente ou non qualifiée :
    /// le démarrage échoue (`failed_start`) avant toute création (`REQ-CORE-001`).
    #[error(
        "Racine runtime '{root}' inutilisable pour le rendez-vous canonique '{rendezvous_dir}' : {reason}. \
         Remédiation : {remediation}"
    )]
    RuntimeRootUnavailable {
        /// Racine runtime attendue (Linux : `/run/user/<uid>`), jamais créée par Unleash.
        root: String,
        /// Chemin du rendez-vous canonique qui dépend de cette racine.
        rendezvous_dir: String,
        /// Motif du refus (absence, type, lien symbolique, propriétaire, mode).
        reason: String,
        /// Remédiation indiquée à l'utilisateur.
        remediation: String,
    },

    /// Emplacements canoniques `ADR-14` non arrêtés pour cette plateforme : aucun candidat de
    /// rendez-vous, aucune disposition, et aucun repli (`REQ-CORE-001`).
    #[error(
        "Emplacements canoniques non arrêtés pour la plateforme {os:?} : \
         aucun repli n'est tenté"
    )]
    LayoutUndecided {
        /// Plateforme hors décisions arrêtées (Windows, hors matrice qualifiée V1).
        os: PlatformOsKind,
    },

    /// Une autre autorité daemon est déjà active pour cet UID sur une autre racine (`ResourceBusy/AUTHORITY_ALREADY_ACTIVE`).
    #[error(
        "Autorité déjà active pour cet utilisateur sur la racine '{}' (racine demandée '{}', socket '{}')",
        active_data_root.display(),
        requested_data_root.display(),
        socket_path.display()
    )]
    AuthorityAlreadyActive {
        /// Racine de données détenue par l'autorité active.
        active_data_root: PathBuf,
        /// Racine de données demandée par le second démarrage.
        requested_data_root: PathBuf,
        /// Socket de rendez-vous de l'autorité active.
        socket_path: PathBuf,
    },

    /// Une instance daemon est déjà active pour cet UID sur la même racine (`ResourceBusy/DAEMON_ALREADY_RUNNING`).
    #[error(
        "Une instance daemon est déjà active sur la racine '{}' (socket '{}')",
        active_data_root.display(),
        socket_path.display()
    )]
    DaemonAlreadyRunning {
        /// Racine de données active.
        active_data_root: PathBuf,
        /// Socket de rendez-vous actif.
        socket_path: PathBuf,
    },

    /// L'UID du pair connecté sur le socket Unix ne correspond pas à l'UID attendu (`AuthenticationFailed`).
    #[error("Identité peer Unix refusée : UID attendu {expected_uid}, UID observé {actual_uid}")]
    PeerUidMismatch {
        /// UID attendu.
        expected_uid: u32,
        /// UID du pair extrait par le noyau.
        actual_uid: u32,
    },

    /// Processus OS introuvable lors de l'inspection.
    #[error("Processus OS introuvable (PID {pid})")]
    ProcessNotFound {
        /// Identifiant PID recherché.
        pid: u32,
    },

    /// Permission refusée lors d'une opération système.
    #[error("Permission système refusée ({context})")]
    PermissionDenied {
        /// Contexte de l'opération refusée.
        context: String,
    },

    /// Échec d'un appel système ou FFI OS ciblé.
    #[error("Échec appel OS '{op}' : {source}")]
    OsCall {
        /// Nom de l'appel système ou FFI.
        op: &'static str,
        /// Erreur OS sous-jacente.
        #[source]
        source: std::io::Error,
    },

    /// Erreur de sérialisation ou désérialisation JSON (métadonnées de verrou).
    #[error("Erreur JSON plateforme : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur d'I/O standard.
    #[error("Erreur I/O plateforme : {0}")]
    Io(#[from] std::io::Error),
}
