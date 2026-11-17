// SPDX-License-Identifier: Apache-2.0
//! # unleash-client
//!
//! Responsabilité exclusive : Point d'entrée unique des clients (`CLI`, `TUI`, hôte `GUI` Tauri)
//! vers le daemon Unleash via socket Unix, codec borné 8 MiB d'`unleash-protocol` et protocole
//! `unleash.ipc.v2` (`F001`, `F014`, `F015`).
//!
//! ## Garanties et frontières
//! - `#![forbid(unsafe_code)]` strict.
//! - Aucun accès `SQLite`, aucune règle métier ni transition d'état locale.
//! - Validation préalable du répertoire `0700`, du socket `0600` et de l'UID peer noyau.
//! - File locale persistable de corrélation conservant `authority_generation` et `command_id`
//!   sur reconnexion/retry (`request_id` frais par tentative, aucune réécriture silencieuse
//!   après `StaleFence/STALE_AUTHORITY_GENERATION`).
//! - Reprise de flux `SubscribeEvents` par `(stream_id, sequence)` et rechargement atomique
//!   `GetStateSnapshot` + watermark sur `ResyncRequired`.
//! - Upload segmenté `ClientUploadSegment` sous la limite d'import 10 MiB.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod client;
pub mod correlation;
pub mod errors;
pub mod events;
pub mod transport;
pub mod upload;

pub use client::UnleashClient;
pub use correlation::{CommandCorrelationQueue, CorrelatedCommand, CorrelationState};
pub use errors::ClientError;
pub use events::{EventFollowerConfig, EventStreamReader, EventSyncUpdate, EventSynchronizer};
pub use transport::{ClientConfig, IpcConnection};
pub use upload::{UploadSummary, build_upload_segments};
