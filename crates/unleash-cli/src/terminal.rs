// SPDX-License-Identifier: Apache-2.0
//! Adaptation des arguments CLI au contrat IPC de contrôle du terminal.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};
use unleash_client::{ClientError, UnleashClient};
use unleash_domain::{AuthorityGeneration, CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{CommandEnvelope, ProtocolVersion};
use uuid::Uuid;

use crate::configuration::{ConfigurationArgs, ConfigurationTarget};
use crate::{CliError, next_uuid};

/// Commandes de `unleash process`.
#[derive(Debug, Subcommand)]
pub enum ProcessCommand {
    /// Lit le profil terminal et la révision des réglages utilisateur.
    Settings,
    /// Enregistre la configuration complète des profils, limites et baux du terminal.
    Configure {
        /// Fichier JSON `terminal_runtime` et révision des réglages utilisateur.
        #[command(flatten)]
        configuration: ConfigurationArgs,
    },
    /// Demande une acquisition et affiche son reçu.
    Acquire {
        /// Allocation ou contrôle d'un terminal enregistré.
        #[command(subcommand)]
        target: AcquireTerminalTarget,
    },
}

/// Cible d'une acquisition de terminal.
#[derive(Debug, Subcommand)]
pub enum AcquireTerminalTarget {
    /// Demande une allocation pour un programme et une mission.
    New(NewTerminalArgs),
    /// Demande le contrôle d'une génération déjà enregistrée.
    Registered(RegisteredTerminalArgs),
}

#[derive(Debug, Args)]
struct AcquisitionScope {
    /// Projet portant la mission ou le terminal.
    #[arg(long)]
    project_id: Uuid,
    /// Révision de la portée du projet.
    #[arg(long)]
    scope_revision: u64,
    /// Identifiant de commande pour un envoi à identité fixée.
    #[arg(long, requires_all = ["authority_generation", "terminal_id"])]
    command_id: Option<Uuid>,
    /// Lignée d'origine à fournir avec les autres arguments identiques lors d'un rejeu.
    #[arg(long, requires = "command_id")]
    authority_generation: Option<Uuid>,
}

/// Arguments de la variante d'allocation.
#[derive(Debug, Args)]
pub struct NewTerminalArgs {
    #[command(flatten)]
    scope: AcquisitionScope,
    /// Identifiant du terminal, généré si omis.
    #[arg(long)]
    terminal_id: Option<Uuid>,
    /// Mission active qui possède la demande.
    #[arg(long)]
    mission_id: Uuid,
    /// Usage stable de ce terminal dans la mission.
    #[arg(long)]
    purpose_key: String,
    /// Répertoire relatif à la racine du projet.
    #[arg(long, default_value = ".")]
    cwd: String,
    /// Chemin du programme autorisé sur la machine du daemon.
    #[arg(long)]
    program: String,
    /// Profil d'environnement enregistré dans les réglages du daemon.
    #[arg(long)]
    environment_profile_id: String,
    /// Nombre initial de lignes du PTY.
    #[arg(long, default_value_t = 24)]
    rows: u16,
    /// Nombre initial de colonnes du PTY.
    #[arg(long, default_value_t = 80)]
    cols: u16,
    /// Délai d'observation du démarrage du programme.
    #[arg(long, default_value_t = 10_000)]
    readiness_timeout_ms: u64,
    /// Arguments du programme, après le séparateur `--`.
    #[arg(last = true)]
    arguments: Vec<String>,
}

/// Arguments de la variante de contrôle d'une génération enregistrée.
#[derive(Debug, Args)]
pub struct RegisteredTerminalArgs {
    #[command(flatten)]
    scope: AcquisitionScope,
    /// Identifiant du terminal indiqué dans le snapshot.
    #[arg(long)]
    terminal_id: Uuid,
    /// Génération du processus indiquée dans le snapshot.
    #[arg(long)]
    process_run_uuid: Uuid,
    /// Révision de contrôle indiquée dans le snapshot.
    #[arg(long)]
    control_revision: u64,
}

impl NewTerminalArgs {
    fn payload(&self) -> Map<String, Value> {
        Map::from_iter([
            ("mission_id".to_owned(), json!(self.mission_id)),
            ("invocation_id".to_owned(), Value::Null),
            ("purpose_key".to_owned(), json!(self.purpose_key)),
            ("reuse_policy".to_owned(), json!("reuse_compatible")),
            ("scope_id".to_owned(), json!(self.scope.project_id)),
            (
                "scope_revision".to_owned(),
                json!(self.scope.scope_revision),
            ),
            ("worktree_id".to_owned(), Value::Null),
            ("cwd".to_owned(), json!(self.cwd)),
            ("program".to_owned(), json!(self.program)),
            ("arguments".to_owned(), json!(self.arguments)),
            (
                "environment_profile_id".to_owned(),
                json!(self.environment_profile_id),
            ),
            (
                "pty".to_owned(),
                json!({"rows": self.rows, "cols": self.cols}),
            ),
            (
                "readiness".to_owned(),
                json!({"kind": "process_started", "timeout_ms": self.readiness_timeout_ms}),
            ),
        ])
    }
}

impl RegisteredTerminalArgs {
    fn payload(&self) -> Map<String, Value> {
        Map::from_iter([
            ("terminal_id".to_owned(), json!(self.terminal_id)),
            ("process_run_uuid".to_owned(), json!(self.process_run_uuid)),
            (
                "scope_revision".to_owned(),
                json!(self.scope.scope_revision),
            ),
            ("control_mode".to_owned(), json!("exclusive")),
        ])
    }
}

impl AcquireTerminalTarget {
    fn envelope(
        &self,
        generation: AuthorityGeneration,
    ) -> Result<(ProjectId, CommandEnvelope), CliError> {
        let (scope, terminal, revision, payload) = match self {
            Self::New(args) => (
                &args.scope,
                args.terminal_id,
                Revision::INITIAL,
                args.payload(),
            ),
            Self::Registered(args) => (
                &args.scope,
                Some(args.terminal_id),
                Revision::new(args.control_revision),
                args.payload(),
            ),
        };
        let (command, generation, terminal) = match (
            scope.command_id,
            scope.authority_generation,
            terminal,
        ) {
            (None, None, terminal) => (next_uuid(), generation, terminal.unwrap_or_else(next_uuid)),
            (Some(command), Some(generation), Some(terminal)) => (
                command,
                AuthorityGeneration::from_uuid(generation),
                terminal,
            ),
            _ => {
                return Err(CliError::InvalidArgument(
                        "Un envoi à identité fixée exige --command-id, --authority-generation et --terminal-id".to_owned(),
                    ));
            }
        };
        let project_id = ProjectId::from_uuid(scope.project_id);
        let envelope = CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(next_uuid()),
            command_id: CommandId::from_uuid(command),
            authority_generation: generation,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: terminal,
            expected_revision: revision,
            command_type: "AcquireTerminal".to_owned(),
            payload,
        };
        Ok((project_id, envelope))
    }
}

pub(super) async fn run_process_command(
    client: &mut UnleashClient,
    action: &ProcessCommand,
) -> Result<Value, CliError> {
    match action {
        ProcessCommand::Settings => crate::configuration::read_terminal_settings(client).await,
        ProcessCommand::Configure { configuration } => {
            configuration
                .apply(client, ConfigurationTarget::TerminalRuntime)
                .await
        }
        ProcessCommand::Acquire { target } => acquire(client, target).await,
    }
}

async fn acquire(
    client: &mut UnleashClient,
    target: &AcquireTerminalTarget,
) -> Result<Value, CliError> {
    let (project_id, envelope) = target.envelope(client.hello().authority_generation)?;
    let receipt = client
        .send_command(&envelope)
        .await
        .map_err(|source| match source {
            ClientError::Remote(ref error)
                if error.code != unleash_protocol::ErrorCode::OutcomeUnknown =>
            {
                CliError::Client(source)
            }
            source => CliError::TerminalReceiptMissing {
                terminal_id: envelope.aggregate_id,
                project_id,
                command_id: envelope.command_id,
                authority_generation: envelope.authority_generation,
                source: Box::new(source),
            },
        })?;
    Ok(json!({
        "project_id": project_id,
        "terminal_id": envelope.aggregate_id,
        "authority_generation": envelope.authority_generation,
        "receipt": receipt,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{Cli, Commands};
    use clap::Parser;

    const PROJECT: &str = "00000000-0000-4000-8000-000000000050";
    const TERMINAL: &str = "00000000-0000-4000-8000-000000000051";
    const PROCESS_RUN: &str = "00000000-0000-4000-8000-000000000052";
    const COMMAND: &str = "00000000-0000-4000-8000-000000000053";
    const ORIGIN: &str = "00000000-0000-4000-8000-000000000054";

    fn target(arguments: impl IntoIterator<Item = &'static str>) -> AcquireTerminalTarget {
        let cli = Cli::try_parse_from(
            ["unleash", "process", "acquire"]
                .into_iter()
                .chain(arguments),
        )
        .unwrap();
        let Commands::Process {
            action: ProcessCommand::Acquire { target },
        } = cli.command
        else {
            unreachable!("la commande d'acquisition est parsée");
        };
        target
    }

    #[test]
    fn terminal_allocation_maps_program_arguments_and_owner_scope() {
        let target = target([
            "new",
            "--project-id",
            PROJECT,
            "--scope-revision",
            "3",
            "--mission-id",
            PROCESS_RUN,
            "--purpose-key",
            "test",
            "--program",
            "/usr/bin/printf",
            "--environment-profile-id",
            "default",
            "--",
            "-v",
            "result",
        ]);
        let generation = AuthorityGeneration::from_uuid(ORIGIN.parse().unwrap());
        let (project, command) = target.envelope(generation).unwrap();
        assert_eq!(project.to_string(), PROJECT);
        assert_eq!(command.authority_generation, generation);
        assert_eq!(command.expected_revision, Revision::INITIAL);
        assert_eq!(command.actor_ref, None);
        assert_eq!(command.payload.get("scope_id"), Some(&json!(PROJECT)));
        assert_eq!(command.payload.get("scope_revision"), Some(&json!(3)));
        assert_eq!(command.payload.get("invocation_id"), Some(&Value::Null));
        assert_eq!(command.payload.get("worktree_id"), Some(&Value::Null));
        assert_eq!(command.payload.get("cwd"), Some(&json!(".")));
        assert_eq!(
            command.payload.get("arguments"),
            Some(&json!(["-v", "result"]))
        );
        assert_eq!(
            command.payload.get("reuse_policy"),
            Some(&json!("reuse_compatible"))
        );
    }

    #[test]
    fn terminal_registered_replay_pins_original_identity() {
        let target = target([
            "registered",
            "--project-id",
            PROJECT,
            "--scope-revision",
            "3",
            "--terminal-id",
            TERMINAL,
            "--process-run-uuid",
            PROCESS_RUN,
            "--control-revision",
            "2",
            "--command-id",
            COMMAND,
            "--authority-generation",
            ORIGIN,
        ]);
        let other_generation = AuthorityGeneration::from_uuid(Uuid::from_u128(1));
        let (_, command) = target.envelope(other_generation).unwrap();
        let (_, replay) = target.envelope(other_generation).unwrap();
        assert_ne!(command.request_id, replay.request_id);
        assert_eq!(command.command_id.to_string(), COMMAND);
        assert_eq!(command.command_id, replay.command_id);
        assert_eq!(command.authority_generation.to_string(), ORIGIN);
        assert_eq!(command.aggregate_id.to_string(), TERMINAL);
        assert_eq!(command.expected_revision, Revision::new(2));
        assert_eq!(command.payload, replay.payload);
        assert_eq!(
            Value::Object(command.payload),
            json!({"terminal_id": TERMINAL, "process_run_uuid": PROCESS_RUN,
                "scope_revision": 3, "control_mode": "exclusive"})
        );
    }
}
