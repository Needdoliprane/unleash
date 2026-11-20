// SPDX-License-Identifier: Apache-2.0
//! # unleash-platform
//!
//! Responsabilité exclusive : Adaptateurs OS locaux d'OVPT Unleash (`F001`, `F011`, `F017`, `ADR-14`).
//!
//! ## Capacités fournies
//! - Résolution des chemins canoniques `ADR-14` (macOS / Linux) et garde stricte interdisant
//!   toute écriture runtime dans `.claude/` (`REQ-CORE-008`, `INV-046`).
//! - Validation de sécurité du répertoire de rendez-vous (`0700`, propriétaire = UID effectif,
//!   pas de lien symbolique) et du socket Unix (`0600`, limite `SUN_LEN <= 103` octets).
//! - Verrou exclusif d'autorité OS (`flock`) par UID effectif, indépendant de `data_root`, profil
//!   et surcharges `XDG_RUNTIME_DIR`/`TMPDIR` (`REQ-CORE-001`, `AuthorityAlreadyActive`).
//! - Extraction de l'identité noyau du pair Unix (`LOCAL_PEERCRED` macOS / `SO_PEERCRED` Linux).
//! - Observation d'identité OS de boot (`boot_identity`) et de processus (`start_identity`)
//!   via `proc_pidinfo` (macOS) et `/proc` (Linux).
//! - Observation du listener TCP loopback détenu par un processus macOS attendu,
//!   avec identité complète et génération socket revérifiées ; sans preuve de pair ou de confinement.
//! - Abstraction du coffre de secrets OS ([`KeyringPort`]) et adaptateur local sécurisé de test
//!   utilisant `secrecy` et `zeroize`.
//!
//! ## Politique `unsafe`
//! Le workspace impose `unsafe_code = "deny"`. Seuls les appels FFI OS strictement ciblés
//! (`confstr`, `getsockopt(LOCAL_PEERCRED/LOCAL_PEERPID)`, `sysctl(KERN_BOOTTIME)`,
//! `proc_pidinfo(PROC_PIDTBSDINFO/PROC_PIDLISTFDS)` et `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)`)
//! utilisent `#[allow(unsafe_code)]` local avec justification
//! `// SAFETY:`.

#![warn(missing_docs)]

pub mod authority;
pub mod errors;
pub mod keyring;
pub mod paths;
pub mod peer_cred;
pub mod process_identity;
pub mod rendezvous;
pub mod tcp_listener;

pub use authority::{
    AUTHORITY_LOCK_FILENAME, AuthorityLock, AuthorityLockMetadata, SOCKET_FILENAME,
};
pub use errors::PlatformError;
pub use keyring::{
    KeyringBackendKind, KeyringError, KeyringPort, LocalSecureKeyring, SecretStatus,
    UnavailableOsKeyring,
};
pub use paths::{
    CLAUDE_DIR_NAME, EnvSnapshot, PlatformOsKind, PlatformPaths, assert_not_under_claude,
    canonical_rendezvous_dir, canonical_rendezvous_dir_with, effective_uid,
};
pub use peer_cred::{
    PeerCredentials, extract_peer_credentials, verify_peer_uid, verify_same_uid_peer,
};
pub use process_identity::{
    BootIdentity, IdentityBackend, OsProcessIdentity, ProcessVerification, StartIdentity,
    current_boot_identity, current_process_identity, inspect_process_identity,
    verify_process_identity,
};
pub use rendezvous::{
    ADMIN_PROOF_FILENAME, MACOS_SUN_LEN, MAX_UNIX_SOCKET_PATH_BYTES, bind_private_unix_listener,
    ensure_private_rendezvous_dir, read_admin_credential, read_private_file_to_string,
    validate_rendezvous_dir, validate_socket_file, validate_unix_socket_path_len,
    write_private_file_atomic,
};
pub use tcp_listener::{
    TcpConnectionObservation, TcpListenerError, TcpListenerObservation, observe_owned_tcp_listener,
    verify_owned_tcp_listener,
};
#[cfg(unix)]
pub use tcp_listener::{observe_owned_tcp_connection, verify_owned_tcp_connection};
