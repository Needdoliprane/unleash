// SPDX-License-Identifier: Apache-2.0
//! Configuration et consultation des profils fournisseurs.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};
use unleash_client::UnleashClient;
use unleash_domain::{ProjectId, RequestId};
use unleash_protocol::{ProtocolVersion, QueryEnvelope};
use uuid::Uuid;

use crate::configuration::{ConfigurationArgs, ConfigurationTarget};
use crate::{CliError, next_uuid};

/// Sous-commandes de `unleash provider`.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// Liste les dernières versions configurées dans une portée.
    List(ProviderListArgs),
    /// Enregistre une version du profil à partir d'un fichier JSON.
    Set(ProviderSetArgs),
    /// Affiche le profil versionné et les observations enregistrées par le daemon.
    Show(ProviderProfileArgs),
}

/// Identité et définition complète du profil à enregistrer.
#[derive(Debug, Args)]
pub struct ProviderSetArgs {
    /// UUID stable du profil à créer ou à réviser.
    #[arg(long, value_parser = parse_non_nil_uuid)]
    profile_id: Uuid,
    /// Projet du profil. Omettre pour un profil utilisateur.
    #[arg(long)]
    project_id: Option<Uuid>,
    #[command(flatten)]
    configuration: ConfigurationArgs,
}

/// Portée et pagination d'une page de profils.
#[derive(Debug, Args)]
pub struct ProviderListArgs {
    /// Projet à consulter. Omettre pour les profils utilisateur.
    #[arg(long)]
    project_id: Option<Uuid>,
    /// Curseur retourné par la page précédente.
    #[arg(long, value_parser = parse_non_nil_uuid)]
    after_profile_id: Option<Uuid>,
    /// Nombre maximal de profils. Valeur par défaut du daemon : 50.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
    limit: Option<u32>,
}

impl ProviderListArgs {
    fn query(&self) -> QueryEnvelope {
        let mut payload = Map::new();
        if let Some(after) = self.after_profile_id {
            payload.insert("after_profile_id".into(), json!(after));
        }
        if let Some(limit) = self.limit {
            payload.insert("limit".into(), json!(limit));
        }
        QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(next_uuid()),
            query_type: "ListProviderProfiles".into(),
            project_id: self.project_id.map(ProjectId::from_uuid),
            payload,
        }
    }
}

/// Version du profil à consulter et portée de sa configuration.
#[derive(Debug, Args)]
pub struct ProviderProfileArgs {
    /// Identité stable du profil renvoyée par la configuration.
    #[arg(long, value_parser = parse_non_nil_uuid)]
    profile_id: Uuid,
    /// Version exacte du profil.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9_007_199_254_740_991))]
    profile_version: u64,
    /// Projet du profil. Omettre pour un profil utilisateur.
    #[arg(long)]
    project_id: Option<Uuid>,
}

fn parse_non_nil_uuid(value: &str) -> Result<Uuid, String> {
    let id: Uuid = value
        .parse()
        .map_err(|error: uuid::Error| error.to_string())?;
    if id.is_nil() {
        return Err("Un UUID non nul est requis".into());
    }
    Ok(id)
}

impl ProviderProfileArgs {
    fn query(&self) -> QueryEnvelope {
        QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(next_uuid()),
            query_type: "GetProviderProfile".into(),
            project_id: self.project_id.map(ProjectId::from_uuid),
            payload: Map::from_iter([
                ("profile_id".into(), json!(self.profile_id)),
                ("profile_version".into(), json!(self.profile_version)),
            ]),
        }
    }
}

pub(super) async fn run_provider_command(
    client: &mut UnleashClient,
    action: &ProviderCommand,
) -> Result<Value, CliError> {
    let query = match action {
        ProviderCommand::List(page) => page.query(),
        ProviderCommand::Set(profile) => {
            return profile
                .configuration
                .apply(
                    client,
                    ConfigurationTarget::Provider {
                        profile_id: profile.profile_id,
                        project_id: profile.project_id.map(ProjectId::from_uuid),
                    },
                )
                .await;
        }
        ProviderCommand::Show(profile) => profile.query(),
    };
    let response = client.query(&query).await?;
    Ok(json!({"watermark": response.watermark, "result": response.result}))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{Cli, Commands};
    use clap::Parser;

    const PROFILE: &str = "00000000-0000-4000-8000-000000000051";
    const PROJECT: &str = "00000000-0000-4000-8000-000000000052";

    // T-F012-018 / REQ-MODEL-018 — portée et pagination sur le fil.
    #[test]
    fn list_queries_the_selected_scope_and_page() {
        for (options, scope, payload) in [
            (vec![], None, json!({})),
            (
                vec![
                    "--project-id",
                    PROJECT,
                    "--after-profile-id",
                    PROFILE,
                    "--limit",
                    "3",
                ],
                Some(ProjectId::from_uuid(PROJECT.parse().unwrap())),
                json!({"after_profile_id": PROFILE, "limit": 3}),
            ),
        ] {
            let cli =
                Cli::try_parse_from(["unleash", "provider", "list"].into_iter().chain(options))
                    .unwrap();
            let Commands::Provider {
                action: ProviderCommand::List(page),
            } = cli.command
            else {
                unreachable!("provider list must parse");
            };
            let query = page.query();
            assert_eq!(query.query_type, "ListProviderProfiles");
            assert_eq!(query.project_id, scope);
            assert_eq!(json!(query.payload), payload);
        }
        for (flag, value) in [
            ("--limit", "0"),
            ("--limit", "101"),
            ("--after-profile-id", "00000000-0000-0000-0000-000000000000"),
        ] {
            assert!(Cli::try_parse_from(["unleash", "provider", "list", flag, value]).is_err());
        }
    }

    #[test]
    fn show_queries_exact_profile_version_and_scope() {
        let base = [
            "unleash",
            "provider",
            "show",
            "--profile-id",
            PROFILE,
            "--profile-version",
            "3",
        ];
        for scope in [None, Some(PROJECT)] {
            let arguments = base
                .into_iter()
                .chain(scope.into_iter().flat_map(|id| ["--project-id", id]));
            let cli = Cli::try_parse_from(arguments).unwrap();
            let Commands::Provider {
                action: ProviderCommand::Show(profile),
            } = cli.command
            else {
                unreachable!("provider show must parse");
            };
            let query = profile.query();
            assert_eq!(query.query_type, "GetProviderProfile");
            assert_eq!(
                query.project_id,
                scope.map(|id| ProjectId::from_uuid(id.parse().unwrap())),
            );
            assert_eq!(
                json!(query.payload),
                json!({"profile_id": PROFILE, "profile_version": 3})
            );
        }
    }

    #[test]
    fn show_requires_a_non_nil_profile_and_exact_version() {
        for (profile, version) in [
            ("00000000-0000-0000-0000-000000000000", "1"),
            (PROFILE, "0"),
            (PROFILE, "9007199254740992"),
        ] {
            assert!(
                Cli::try_parse_from([
                    "unleash",
                    "provider",
                    "show",
                    "--profile-id",
                    profile,
                    "--profile-version",
                    version,
                ])
                .is_err()
            );
        }
    }
}
