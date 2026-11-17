// SPDX-License-Identifier: Apache-2.0
//! # unleash-cli
//!
//! Responsabilité exclusive : Interface en ligne de commande d'OVPT Unleash (`unleash`),
//! passant exclusivement par [`unleash_client::UnleashClient`] et le protocole IPC `unleash.ipc.v2`.
//!
//! ## Garanties et frontières
//! - `#![forbid(unsafe_code)]` strict.
//! - Aucune dépendance vers `rusqlite`, `unleash-store` ou `unleash-daemon`.
//! - Aucune règle métier locale : toute transition et révision appartient au daemon.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use serde_json::{Map, Value, json};
use thiserror::Error;
use unleash_client::{ClientConfig, ClientError, EventStreamReader, UnleashClient};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision, Sequence};
use unleash_platform::{PlatformError, SOCKET_FILENAME, canonical_rendezvous_dir};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, SubscribeEventsFrame,
    SubscribeEventsRequest,
};
use uuid::Uuid;

static UUID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Erreur typée retournée par les commandes de la CLI Unleash.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CliError {
    /// Erreur plateforme lors de la résolution du socket par défaut.
    #[error("Erreur plateforme CLI : {0}")]
    Platform(#[from] PlatformError),

    /// Erreur du client IPC `unleash-client`.
    #[error("Erreur client IPC : {0}")]
    Client(#[from] ClientError),

    /// Argument utilisateur invalide.
    #[error("Argument CLI invalide : {0}")]
    InvalidArgument(String),

    /// Erreur de sérialisation JSON.
    #[error("Erreur JSON CLI : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur I/O locale.
    #[error("Erreur I/O CLI : {0}")]
    Io(#[from] std::io::Error),
}

/// Options et sous-commandes de la CLI `unleash`.
#[derive(Debug, Parser)]
#[command(name = "unleash", version, about = "OVPT Unleash CLI")]
pub struct Cli {
    /// Chemin du socket Unix du daemon (défaut : `$UNLEASH_SOCKET_PATH` ou rendez-vous canonique UID).
    #[arg(long, global = true)]
    pub socket: Option<PathBuf>,

    /// Émet une sortie JSON structurée sur stdout.
    #[arg(long, global = true)]
    pub json: bool,

    /// Délai maximal par requête IPC en millisecondes.
    #[arg(long, global = true, default_value_t = 30_000)]
    pub timeout_ms: u64,

    /// Sous-commande à exécuter.
    #[command(subcommand)]
    pub command: Commands,
}

/// Sous-commandes principales de `unleash`.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Interroge `GetDaemonStatus`.
    Status,
    /// Interroge `GetHealth`.
    Health,
    /// Interroge `GetStateSnapshot`.
    Snapshot {
        /// Filtre optionnel par projet.
        #[arg(long)]
        project_id: Option<Uuid>,
    },
    /// Inspecte un reçu durable par `command_id` (`GetCommandReceipt`).
    Receipt {
        /// Identifiant UUID de la commande.
        command_id: Uuid,
    },
    /// Gestion des projets.
    Project {
        /// Action sur les projets.
        #[command(subcommand)]
        action: ProjectCommand,
    },
    /// Pilotage des missions.
    Mission {
        /// Action sur les missions.
        #[command(subcommand)]
        action: MissionCommand,
    },
    /// Contrôle du cycle de vie daemon.
    Daemon {
        /// Action sur le daemon.
        #[command(subcommand)]
        action: DaemonCommand,
    },
}

/// Sous-commandes `unleash project`.
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Crée un projet via `CreateProject`.
    Create {
        /// Nom du projet.
        #[arg(long)]
        name: String,
        /// Identifiant explicite optionnel.
        #[arg(long)]
        project_id: Option<Uuid>,
        /// Poids d'équité (`1..=10`).
        #[arg(long, default_value_t = 1)]
        weight: i64,
        /// Identifiant d'idempotence optionnel.
        #[arg(long)]
        command_id: Option<Uuid>,
    },
}

/// Sous-commandes `unleash mission`.
#[derive(Debug, Subcommand)]
pub enum MissionCommand {
    /// Assigne/crée une mission via `AssignMission`.
    Assign {
        /// Identifiant du projet propriétaire.
        #[arg(long)]
        project_id: Uuid,
        /// Identifiant explicite de la mission.
        #[arg(long)]
        mission_id: Option<Uuid>,
        /// Politique de complétion (`mechanical`, `human`, `owner_agent`).
        #[arg(long, default_value = "mechanical")]
        completion_policy: String,
        /// État initial (`active`, `ready`, `proposed`).
        #[arg(long, default_value = "active")]
        state: String,
        /// Identifiant d'idempotence optionnel.
        #[arg(long)]
        command_id: Option<Uuid>,
    },
    /// Met une mission en pause (`PauseMission`).
    Pause {
        /// Identifiant UUID de la mission.
        mission_id: Uuid,
        /// Projet optionnel associé.
        #[arg(long)]
        project_id: Option<Uuid>,
        /// Révision attendue explicite.
        #[arg(long)]
        expected_revision: Option<u64>,
        /// Motif optionnel.
        #[arg(long)]
        reason: Option<String>,
        /// Identifiant d'idempotence optionnel.
        #[arg(long)]
        command_id: Option<Uuid>,
    },
    /// Reprend une mission en pause (`ResumeMission`).
    Resume {
        /// Identifiant UUID de la mission.
        mission_id: Uuid,
        /// Projet optionnel associé.
        #[arg(long)]
        project_id: Option<Uuid>,
        /// Révision attendue explicite.
        #[arg(long)]
        expected_revision: Option<u64>,
        /// Motif optionnel.
        #[arg(long)]
        reason: Option<String>,
        /// Identifiant d'idempotence optionnel.
        #[arg(long)]
        command_id: Option<Uuid>,
    },
    /// Annule une mission (`CancelMission`).
    Cancel {
        /// Identifiant UUID de la mission.
        mission_id: Uuid,
        /// Projet optionnel associé.
        #[arg(long)]
        project_id: Option<Uuid>,
        /// Révision attendue explicite.
        #[arg(long)]
        expected_revision: Option<u64>,
        /// Motif optionnel.
        #[arg(long)]
        reason: Option<String>,
        /// Identifiant d'idempotence optionnel.
        #[arg(long)]
        command_id: Option<Uuid>,
    },
}

/// Sous-commandes `unleash daemon`.
#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    /// Arrête proprement le daemon (`StopDaemon`).
    Stop {
        /// Révision attendue explicite de `daemon_state`.
        #[arg(long)]
        expected_revision: Option<u64>,
        /// Motif d'arrêt.
        #[arg(long)]
        reason: Option<String>,
        /// Délai de drainage souhaité en ms.
        #[arg(long)]
        drain_timeout_ms: Option<u64>,
        /// Retour immédiat après réception du reçu `accepted` sans attendre `DaemonStopped`.
        #[arg(long)]
        no_wait: bool,
        /// Identifiant d'idempotence optionnel.
        #[arg(long)]
        command_id: Option<Uuid>,
    },
}

/// Exécute une commande CLI parsée et retourne la valeur JSON résultat.
///
/// # Errors
/// Retourne [`CliError`] en cas d'erreur de transport, protocole ou argument.
pub async fn run_cli(cli: &Cli) -> Result<Value, CliError> {
    let socket_path = resolve_socket_path(cli.socket.as_deref())?;
    let timeout = Duration::from_millis(cli.timeout_ms.max(100));
    let client_cfg = ClientConfig::new(socket_path, ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .with_timeouts(timeout, timeout);
    let mut client = UnleashClient::connect(client_cfg).await?;

    match &cli.command {
        Commands::Status => {
            let resp = query_simple(&mut client, "GetDaemonStatus", None, Map::new()).await?;
            Ok(resp.result)
        }
        Commands::Health => {
            let resp = query_simple(&mut client, "GetHealth", None, Map::new()).await?;
            Ok(resp.result)
        }
        Commands::Snapshot { project_id } => {
            let (snapshot, watermark) = client
                .get_state_snapshot(
                    RequestId::from_uuid(next_uuid()),
                    project_id.map(ProjectId::from_uuid),
                    Map::new(),
                )
                .await?;
            Ok(json!({
                "watermark": watermark,
                "snapshot": snapshot,
            }))
        }
        Commands::Receipt { command_id } => {
            let inspection = client
                .get_command_receipt(
                    RequestId::from_uuid(next_uuid()),
                    CommandId::from_uuid(*command_id),
                )
                .await?;
            Ok(serde_json::to_value(inspection)?)
        }
        Commands::Project {
            action:
                ProjectCommand::Create {
                    name,
                    project_id,
                    weight,
                    command_id,
                },
        } => {
            let pid = ProjectId::from_uuid(project_id.unwrap_or_else(next_uuid));
            let cmd_id = CommandId::from_uuid(command_id.unwrap_or_else(next_uuid));
            let mut payload = Map::new();
            payload.insert(String::from("name"), Value::String(name.clone()));
            payload.insert(String::from("weight"), Value::from(*weight));
            let env = CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(next_uuid()),
                command_id: cmd_id,
                authority_generation: client.hello().authority_generation,
                actor_ref: None,
                project_id: Some(pid),
                aggregate_id: pid.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: String::from("CreateProject"),
                payload,
            };
            let receipt = client.send_command(&env).await?;
            Ok(serde_json::to_value(receipt)?)
        }
        Commands::Mission { action } => run_mission_command(&mut client, action).await,
        Commands::Daemon { action } => run_daemon_command(&mut client, action).await,
    }
}

async fn run_mission_command(
    client: &mut UnleashClient,
    action: &MissionCommand,
) -> Result<Value, CliError> {
    match action {
        MissionCommand::Assign {
            project_id,
            mission_id,
            completion_policy,
            state,
            command_id,
        } => {
            let pid = ProjectId::from_uuid(*project_id);
            let mid = mission_id.unwrap_or_else(next_uuid);
            let cmd_id = CommandId::from_uuid(command_id.unwrap_or_else(next_uuid));
            let mut payload = Map::new();
            payload.insert(
                String::from("completion_policy"),
                Value::String(completion_policy.clone()),
            );
            payload.insert(String::from("state"), Value::String(state.clone()));
            let env = CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(next_uuid()),
                command_id: cmd_id,
                authority_generation: client.hello().authority_generation,
                actor_ref: None,
                project_id: Some(pid),
                aggregate_id: mid,
                expected_revision: Revision::INITIAL,
                command_type: String::from("AssignMission"),
                payload,
            };
            let receipt = client.send_command(&env).await?;
            Ok(serde_json::to_value(receipt)?)
        }
        MissionCommand::Pause {
            mission_id,
            project_id,
            expected_revision,
            reason,
            command_id,
        } => {
            run_mission_transition(
                client,
                "PauseMission",
                *mission_id,
                *project_id,
                *expected_revision,
                reason.as_deref(),
                *command_id,
            )
            .await
        }
        MissionCommand::Resume {
            mission_id,
            project_id,
            expected_revision,
            reason,
            command_id,
        } => {
            run_mission_transition(
                client,
                "ResumeMission",
                *mission_id,
                *project_id,
                *expected_revision,
                reason.as_deref(),
                *command_id,
            )
            .await
        }
        MissionCommand::Cancel {
            mission_id,
            project_id,
            expected_revision,
            reason,
            command_id,
        } => {
            run_mission_transition(
                client,
                "CancelMission",
                *mission_id,
                *project_id,
                *expected_revision,
                reason.as_deref(),
                *command_id,
            )
            .await
        }
    }
}

async fn run_mission_transition(
    client: &mut UnleashClient,
    command_type: &str,
    mission_id: Uuid,
    project_id: Option<Uuid>,
    expected_revision: Option<u64>,
    reason: Option<&str>,
    command_id: Option<Uuid>,
) -> Result<Value, CliError> {
    let resolved_rev = match expected_revision {
        Some(r) => Revision::new(r),
        None => resolve_mission_revision(client, mission_id, project_id).await?,
    };
    let mut payload = Map::new();
    if let Some(r) = reason {
        payload.insert(String::from("reason"), Value::String(r.to_owned()));
    }
    let env = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_uuid(next_uuid()),
        command_id: CommandId::from_uuid(command_id.unwrap_or_else(next_uuid)),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: project_id.map(ProjectId::from_uuid),
        aggregate_id: mission_id,
        expected_revision: resolved_rev,
        command_type: command_type.to_owned(),
        payload,
    };
    let receipt = client.send_command(&env).await?;
    Ok(serde_json::to_value(receipt)?)
}

async fn resolve_mission_revision(
    client: &mut UnleashClient,
    mission_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Revision, CliError> {
    let (snapshot, _) = client
        .get_state_snapshot(
            RequestId::from_uuid(next_uuid()),
            project_id.map(ProjectId::from_uuid),
            Map::new(),
        )
        .await?;
    let needle = mission_id.to_string();
    let missions = snapshot
        .get("missions")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::InvalidArgument(String::from("Snapshot missions manquant")))?;
    for m in missions {
        if m.get("mission_id").and_then(Value::as_str) == Some(needle.as_str()) {
            let rev = m.get("revision").and_then(Value::as_u64).ok_or_else(|| {
                CliError::InvalidArgument(format!("Révision absente pour mission {needle}"))
            })?;
            return Ok(Revision::new(rev));
        }
    }
    Err(CliError::InvalidArgument(format!(
        "Mission {needle} introuvable dans le snapshot pour résoudre expected_revision"
    )))
}

async fn run_daemon_command(
    client: &mut UnleashClient,
    action: &DaemonCommand,
) -> Result<Value, CliError> {
    let DaemonCommand::Stop {
        expected_revision,
        reason,
        drain_timeout_ms,
        no_wait,
        command_id,
    } = action;

    let status_resp = query_simple(client, "GetDaemonStatus", None, Map::new()).await?;
    let daemon_rev = match expected_revision {
        Some(r) => *r,
        None => status_resp
            .result
            .get("revision")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                CliError::InvalidArgument(String::from("daemon_state.revision absente"))
            })?,
    };

    let auth = client.hello().authority_generation;
    let stream_id = client.hello().stream_id;
    let watermark_seq = status_resp
        .watermark
        .map_or(Sequence::new(0), |wm| wm.sequence);

    // Ouvrir l'abonnement AVANT StopDaemon afin d'observer DaemonStopped de façon déterministe
    let sub_reader: Option<EventStreamReader> = if *no_wait {
        None
    } else {
        Some(
            client
                .subscribe_events(&SubscribeEventsRequest {
                    stream_id,
                    after_sequence: watermark_seq,
                    project_id: None,
                    aggregate_id: Some(auth.as_uuid()),
                    batch_limit: 32,
                })
                .await?,
        )
    };

    let mut payload = Map::new();
    if let Some(r) = reason {
        payload.insert(String::from("reason"), Value::String(r.clone()));
    }
    if let Some(ms) = drain_timeout_ms {
        payload.insert(String::from("drain_timeout_ms"), Value::from(*ms));
    }
    let env = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_uuid(next_uuid()),
        command_id: CommandId::from_uuid(command_id.unwrap_or_else(next_uuid)),
        authority_generation: auth,
        actor_ref: None,
        project_id: None,
        aggregate_id: auth.as_uuid(),
        expected_revision: Revision::new(daemon_rev),
        command_type: String::from("StopDaemon"),
        payload,
    };
    let receipt = client.send_command(&env).await?;

    let (stop_outcome, retained_services, unknown_services) = match sub_reader {
        Some(mut reader) => wait_for_daemon_stopped(&mut reader).await,
        None => (Value::Null, Vec::new(), Vec::new()),
    };

    Ok(json!({
        "receipt": receipt,
        "stop_outcome": stop_outcome,
        "retained_services": retained_services,
        "unknown_services": unknown_services,
    }))
}

async fn wait_for_daemon_stopped(
    reader: &mut EventStreamReader,
) -> (Value, Vec<Value>, Vec<Value>) {
    let mut stop_outcome = Value::Null;
    let mut retained_services = Vec::new();
    let mut unknown_services = Vec::new();

    while let Ok(Some(frame)) = reader.next_frame().await {
        let SubscribeEventsFrame::Events { events, .. } = frame else {
            continue;
        };
        for ev in events {
            if ev.event_type != "DaemonStopped" {
                continue;
            }
            stop_outcome = ev
                .payload
                .get("stop_outcome")
                .cloned()
                .unwrap_or(Value::Null);
            if let Some(arr) = ev
                .payload
                .get("retained_services")
                .and_then(Value::as_array)
            {
                retained_services.clone_from(arr);
            }
            if let Some(arr) = ev.payload.get("unknown_services").and_then(Value::as_array) {
                unknown_services.clone_from(arr);
            }
            return (stop_outcome, retained_services, unknown_services);
        }
    }

    (stop_outcome, retained_services, unknown_services)
}

async fn query_simple(
    client: &mut UnleashClient,
    query_type: &str,
    project_id: Option<ProjectId>,
    payload: Map<String, Value>,
) -> Result<unleash_protocol::QueryResponse, CliError> {
    let q = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_uuid(next_uuid()),
        query_type: query_type.to_owned(),
        project_id,
        payload,
    };
    client.query(&q).await.map_err(CliError::from)
}

/// Résout le chemin du socket daemon depuis `--socket`, `UNLEASH_SOCKET_PATH` ou le rendez-vous canonique.
///
/// # Errors
/// Retourne [`CliError::Platform`] si la résolution par défaut du rendez-vous échoue.
pub fn resolve_socket_path(explicit: Option<&Path>) -> Result<PathBuf, CliError> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    if let Some(env_val) = std::env::var_os("UNLEASH_SOCKET_PATH")
        && !env_val.is_empty()
    {
        return Ok(PathBuf::from(env_val));
    }
    Ok(canonical_rendezvous_dir()?.join(SOCKET_FILENAME))
}

fn next_uuid() -> Uuid {
    let nanos = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos()),
    )
    .unwrap_or(1);
    let pid = u64::from(std::process::id());
    let seq = UUID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let hi = nanos ^ pid.rotate_left(32);
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..16].copy_from_slice(&seq.to_be_bytes());
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_all_core_m0_commands_and_json_flag() {
        let parsed = Cli::try_parse_from([
            "unleash",
            "--socket",
            "/tmp/test-daemon.sock",
            "--json",
            "status",
        ])
        .expect("parse status");
        assert!(parsed.json);
        assert_eq!(parsed.socket, Some(PathBuf::from("/tmp/test-daemon.sock")));
        assert!(matches!(parsed.command, Commands::Status));

        let mid = "00000000-0000-4000-8000-000000000001";
        let pause = Cli::try_parse_from([
            "unleash",
            "mission",
            "pause",
            mid,
            "--expected-revision",
            "2",
            "--reason",
            "inspection",
        ])
        .expect("parse mission pause");
        assert!(matches!(
            pause.command,
            Commands::Mission {
                action: MissionCommand::Pause {
                    expected_revision: Some(2),
                    ..
                }
            }
        ));

        let stop = Cli::try_parse_from(["unleash", "--json", "daemon", "stop", "--no-wait"])
            .expect("parse daemon stop");
        assert!(matches!(
            stop.command,
            Commands::Daemon {
                action: DaemonCommand::Stop { no_wait: true, .. }
            }
        ));
    }

    #[test]
    fn explicit_socket_precedes_env_and_generated_uuids_are_distinct() {
        let p =
            resolve_socket_path(Some(Path::new("/tmp/explicit.sock"))).expect("socket explicite");
        assert_eq!(p, PathBuf::from("/tmp/explicit.sock"));
        let a = next_uuid();
        let b = next_uuid();
        assert_ne!(a, b);
    }
}
