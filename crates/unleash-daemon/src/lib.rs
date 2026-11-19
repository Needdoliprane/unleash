// SPDX-License-Identifier: Apache-2.0
//! # unleash-daemon
//!
//! Responsabilité exclusive : Orchestration locale et autorité unique du daemon utilisateur
//! Unleash (`unleashd`). Compose les adaptateurs OS (`unleash-platform`), la persistance WAL
//! (`unleash-store`), l'inventaire passif navigateur (`unleash-browser`), le protocole IPC v2
//! (`unleash-protocol`) et la machine d'état domaine (`unleash-domain`).
//!
//! ## Garanties et frontières
//! - `#![forbid(unsafe_code)]` strict.
//! - Verrou d'autorité exclusif par UID effectif avant toute ouverture de base ou socket.
//! - Barrière de démarrage (`starting -> recovering -> ready`) refusant toute admission métier
//!   avant qualification `SQLite`, verrou OS et classification de reprise.
//! - Aucune écriture d'état opérationnel dans `.claude/`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod auth;
mod backup_service;
mod broker_activity;
#[allow(
    dead_code,
    reason = "Raccordement runtime différé à la publication des événements et au producteur qualifié"
)]
mod broker_journal;
pub mod config;
pub mod daemon;
pub mod diagnostic;
pub mod error;
pub mod executors;
pub mod ipc;
mod provider_profile_service;
pub mod recovery;
pub mod subscriptions;
mod terminal_service;

pub use auth::{AuthVerifierPort, AuthenticatedSession, Capability, PeerUidAuthVerifier};
pub use config::{DaemonConfig, PostCommitAction, PostCommitHook, StoreOverrides};
pub use daemon::{DaemonHandle, DaemonStartOutcome, UnleashDaemon};
pub use error::DaemonError;
pub use executors::{
    DrainReport, ExecutorDrainPort, ExecutorRegistration, ExecutorState,
    RendezvousExecutorController,
};
pub use recovery::RecoveryGate;
pub use subscriptions::SubscriptionHub;
