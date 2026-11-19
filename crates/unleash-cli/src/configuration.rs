// SPDX-License-Identifier: Apache-2.0
//! Chargement de définitions locales et transport des mutations de configuration.

use std::io::Read;
use std::path::{Path, PathBuf};

use clap::Args;
use serde_json::{Map, Value, json};
use unleash_client::{ClientError, UnleashClient};
use unleash_domain::{AuthorityGeneration, CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{
    CommandEnvelope, ErrorCode, MAX_FRAME_BYTES, ProtocolVersion, from_json_str_strict,
};
use uuid::Uuid;

use crate::{CliError, next_uuid, query_simple};

/// Fichier de configuration et identité explicite d'une mutation revisionnée.
#[derive(Debug, Args)]
pub struct ConfigurationArgs {
    /// Fichier contenant la définition JSON complète à enregistrer.
    #[arg(long)]
    file: PathBuf,
    /// Révision de la configuration que cette définition remplace.
    #[arg(long)]
    expected_revision: u64,
    /// Identifiant stable pour l'inspection ou le rejeu d'une commande.
    #[arg(long, requires = "authority_generation")]
    command_id: Option<Uuid>,
    /// Lignée d'origine pour rejouer le même fichier et la même révision.
    #[arg(long, requires = "command_id")]
    authority_generation: Option<Uuid>,
}

#[derive(Clone, Copy)]
pub(super) enum ConfigurationTarget {
    Project(Uuid),
    TerminalRuntime,
}

fn read_definition(path: &Path) -> Result<Map<String, Value>, CliError> {
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_FRAME_BYTES as u64 + 1)
        .read_to_string(&mut text)?;
    if text.len() > MAX_FRAME_BYTES {
        return Err(CliError::InvalidArgument(
            "Le fichier dépasse la taille maximale d'une trame IPC".to_owned(),
        ));
    }
    from_json_str_strict(&text).map_err(|_| {
        CliError::InvalidArgument(
            "Le fichier doit contenir un objet JSON sans clés dupliquées".to_owned(),
        )
    })
}

impl ConfigurationArgs {
    fn command(
        &self,
        target: ConfigurationTarget,
        mut definition: Map<String, Value>,
        current_generation: AuthorityGeneration,
    ) -> Result<CommandEnvelope, CliError> {
        let (command_id, generation) = match (self.command_id, self.authority_generation) {
            (None, None) => (next_uuid(), current_generation),
            (Some(command), Some(generation)) => {
                (command, AuthorityGeneration::from_uuid(generation))
            }
            _ => {
                return Err(CliError::InvalidArgument(
                    "Associer --command-id à --authority-generation pour fixer l'identité"
                        .to_owned(),
                ));
            }
        };
        let (operation, aggregate_id, project_id) = match target {
            ConfigurationTarget::Project(id) => {
                if !definition.contains_key("primary_root_path") {
                    return Err(CliError::InvalidArgument(
                        "La définition complète du projet doit indiquer primary_root_path, éventuellement à null".to_owned(),
                    ));
                }
                if definition
                    .get("repositories")
                    .and_then(Value::as_array)
                    .is_some_and(|repositories| {
                        repositories
                            .iter()
                            .any(|repository| repository.get("default_branch").is_none())
                    })
                {
                    return Err(CliError::InvalidArgument(
                        "Chaque dépôt doit indiquer default_branch, éventuellement à null"
                            .to_owned(),
                    ));
                }
                ("ReviseProject", id, Some(ProjectId::from_uuid(id)))
            }
            ConfigurationTarget::TerminalRuntime => {
                definition = Map::from_iter([
                    ("scope_kind".to_owned(), json!("user")),
                    ("scope_id".to_owned(), json!("default")),
                    ("patch".to_owned(), json!({"terminal_runtime": definition})),
                ]);
                ("SetSettings", generation.as_uuid(), None)
            }
        };
        Ok(CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(next_uuid()),
            command_id: CommandId::from_uuid(command_id),
            authority_generation: generation,
            actor_ref: None,
            project_id,
            aggregate_id,
            expected_revision: Revision::new(self.expected_revision),
            command_type: operation.to_owned(),
            payload: definition,
        })
    }

    pub(super) async fn apply(
        &self,
        client: &mut UnleashClient,
        target: ConfigurationTarget,
    ) -> Result<Value, CliError> {
        let file = self.file.clone();
        let definition = tokio::task::spawn_blocking(move || read_definition(&file))
            .await
            .map_err(|error| {
                CliError::InvalidArgument(format!("Lecture de configuration interrompue ({error})"))
            })??;
        let command = self.command(target, definition, client.hello().authority_generation)?;
        let receipt = client.send_command(&command).await.map_err(|source| {
            if matches!(&source, ClientError::Remote(error) if error.code != ErrorCode::OutcomeUnknown) {
                CliError::Client(source)
            } else {
                CliError::ConfigurationReceiptMissing {
                    aggregate_id: command.aggregate_id,
                    command_id: command.command_id,
                    authority_generation: command.authority_generation,
                    source: Box::new(source),
                }
            }
        })?;
        Ok(json!({"authority_generation": command.authority_generation, "receipt": receipt}))
    }
}

pub(super) async fn read_terminal_settings(client: &mut UnleashClient) -> Result<Value, CliError> {
    let payload = Map::from_iter([
        ("scope_kind".to_owned(), json!("user")),
        ("scope_id".to_owned(), json!("default")),
    ]);
    let response = query_simple(client, "ReadSettings", None, payload).await?;
    let revision = serde_json::from_value::<Revision>(
        response.result.get("revision").cloned().unwrap_or_default(),
    )?;
    Ok(json!({
        "watermark": response.watermark,
        "authority_generation": client.hello().authority_generation,
        "result": {
            "revision": revision,
            "terminal_runtime": response.result.pointer("/extra_typed/terminal_runtime"),
        },
    }))
}
