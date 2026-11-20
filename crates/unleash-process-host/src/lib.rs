// SPDX-License-Identifier: Apache-2.0
//! # unleash-process-host
//!
//! Responsabilité exclusive : Détenteur propriétaire d'une unique ressource PTY ou d'un service
//! enfant direct pour OVPT Unleash (`F011`, `ADR-05`, `Q-05`, `REQ-TERM-002..008`).
//!
//! ## Garanties fournies
//! - **Ancrage non réutilisable (`anchor`)** : l'enfant direct est lancé avec `setsid()`, sous
//!   qualification `SIGCHLD` (jamais `SIG_IGN` ni `SA_NOCLDWAIT`), et signaux/récolte sont
//!   strictement sérialisés par le seul [`OwnedChildAnchor`] (avec `pidfd` sous Linux). Tout appel
//!   sur simple PID observé, enfant récolté ou divergence d'identité retourne `IdentityUnverified`.
//! - **Manifeste durable et secret protégé (`manifest`)** : manifeste privé `0600` hors `.claude/`,
//!   secret de contrôle séparé dans `control.secret` `0600` (`secrecy`/`zeroize`), jamais en `argv`
//!   ni dans les logs.
//! - **Fencing strict (`fencing`)** : rejet immédiat de toute `authority_generation` étrangère
//!   (`STALE_AUTHORITY_GENERATION`) et de tout `daemon_epoch`/`fence` inférieur dans la même lignée,
//!   avec rotation atomique de capacité lors du handshake de récupération.
//! - **Spool de logs borné (`spool`)** : segments immuables avec offset, séquence, SHA-256,
//!   watermark durable vs octets volatils, redaction de canaris même coupés sur plusieurs lectures
//!   PTY ou multi-lignes, et gaps explicites à saturation.
//! - **Entrées PTY numérotées (`pty_input`)** : marqueur durable avant écriture au maître et reçu
//!   durable après ; une coupure entre écriture et reçu classe l'entrée en `OutcomeUnknown` sans
//!   jamais la répéter automatiquement.
//! - **Mode `strict` Q-05 (`isolation`)** : génération et application fail-closed d'un profil
//!   Seatbelt `/usr/bin/sandbox-exec` sur macOS, et construction de plan `bubblewrap` `--unshare-all` /
//!   `--clearenv` sur Linux (`linux-bubblewrap`, sans filtre `seccomp` ni proxy monté en M0) ;
//!   tout défaut de backend ou d'obligation retourne explicitement `IsolationUnavailable` sans
//!   jamais basculer vers `standard`.
//! - **Zéro modèle** : aucun client modèle, aucune clé API, aucune autonomie d'orchestration.
//!
//! ## Politique `unsafe`
//! Le workspace impose `unsafe_code = "deny"`. Seuls les appels système strictement ciblés
//! dans `anchor` (`sigaction(SIGCHLD)` dans [`inspect_sigchld_disposition`]/[`ensure_sigchld_qualified`],
//! `pre_exec(setsid)` dans [`configure_setsid_pre_exec`] et `proc_listallpids` pour l'énumération
//! des membres de groupe sous macOS) lèvent `#[allow(unsafe_code)]` localement avec justification
//! `// SAFETY:` ; l'observation non destructive `waitid(WNOWAIT)` passe par `rustix::process::waitid`.

#![warn(missing_docs)]

pub mod anchor;
mod control_release;
mod control_transfer;
pub mod error;
pub mod fencing;
pub mod host;
pub mod isolation;
pub mod manifest;
pub mod protocol;
pub mod pty_input;
pub mod spool;

pub use anchor::{
    AnchorVerificationReport, ChildExitStatus, HostSignal, OsProcessInspector, OwnedChildAnchor,
    OwnedChildHandle, ProcessInspector, SigchldQualification, StopObservation,
    configure_setsid_pre_exec, ensure_sigchld_qualified, inspect_sigchld_disposition,
};
pub use control_release::{
    ControlReacquisitionAck, ControlReacquisitionRequest, ControlReacquisitionResponse,
    ControlReleaseAck, ControlReleaseKey, ControlReleaseRequest, ControlReleaseResponse,
    HostControlState,
};
pub use control_transfer::{ControlTransferAck, ControlTransferRequest, ControlTransferResponse};
pub use error::ProcessHostError;
pub use fencing::{FencingGate, HostCommandHeader};
pub use host::{
    HOST_READY_PREFIX, HOST_SOCKET_FILENAME, HostClient, LostHostReconciliation, ProcessHostServer,
    StdioBarrierCutpointObserver, StrictServiceLaunchObservation, attest_strict_service_launch,
    reconcile_after_lost_host,
};
pub use isolation::{
    IsolationBackendProbe, IsolationMode, IsolationProfileKind, LinuxBwrapPlan,
    OsIsolationBackendProbe, PreparedLaunchCommand, SandboxProbeOutcome, StrictIsolationSpec,
    build_clean_child_env, build_linux_bwrap_plan, prepare_launch_command,
    render_macos_seatbelt_profile,
};
pub use manifest::{
    CANARY_SECRETS_FILENAME, CONTROL_SECRET_FILENAME, EnvironmentMode, HOST_MANIFEST_FILENAME,
    HostBootstrapSpec, HostControlSecret, HostLaunchState, HostManifest, ResourceKind,
    effective_environment_digest_v1, explicit_environment_v1, load_canary_secrets,
    load_control_secret, store_canary_secrets, store_control_secret,
};
pub use protocol::{HostRequest, HostResponse};
pub use pty_input::{
    HostCutpointObserver, HostInputCutpoint, INPUT_JOURNAL_FILENAME, InputEntryState,
    InputJournalEntry, NoopHostCutpointObserver, PtyInputReceipt, PtyInputTracker,
};
pub use spool::{
    LogGap, LogReadResult, LogReadSegment, LogSegmentMeta, LogSpool, SPOOL_INDEX_FILENAME,
    SecretRedactor, SpoolConfig, SpoolIndex,
};
