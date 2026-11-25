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
pub mod execution_scheduler;
pub mod executors;
mod git_inspector;
pub mod ipc;
mod managed_workers;
mod provider_probe_connect_service;
mod provider_probe_generation_service;
mod provider_profile_service;
mod provider_profile_validation_service;
pub mod recovery;
pub mod subscriptions;
pub mod terminal_host_launcher;
mod terminal_service;
pub mod worker_launch;
pub mod worktree_inventory;
#[cfg(test)]
mod worker_shutdown_tests;

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
pub use worktree_inventory::{IntegrationReadiness, integration_readiness};

#[cfg(test)]
mod start_assignment_tests;
