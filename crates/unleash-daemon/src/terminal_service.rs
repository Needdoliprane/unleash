//! Authenticated terminal configuration and allocation orchestration.

use crate::ipc::DaemonRuntimeState;
use std::sync::Arc;
use unleash_protocol::{CommandEnvelope, ErrorCode, IpcError, RetryClass};
use unleash_store::terminal_allocations::{PreparedTerminalEnvironment, TerminalRuntimeSettings};

fn invalid(command: &CommandEnvelope) -> IpcError {
    IpcError::new(
        ErrorCode::InvalidArgument,
        None,
        "Invalid terminal runtime configuration",
        RetryClass::Never,
    )
    .with_command_id(command.command_id)
}

pub(crate) async fn prepare_configuration_objects(
    state: &Arc<DaemonRuntimeState>,
    actor: &unleash_domain::ActorId,
    command: &CommandEnvelope,
) -> Result<Option<Vec<PreparedTerminalEnvironment>>, IpcError> {
    if command.command_type != "SetSettings" {
        return Ok(None);
    }
    let Some(raw) = command
        .payload
        .get("patch")
        .and_then(|v| v.get("terminal_runtime"))
    else {
        return Ok(None);
    };
    if command.project_id.is_some()
        || command.aggregate_id != state.authority_generation.as_uuid()
        || command
            .payload
            .get("scope_kind")
            .and_then(serde_json::Value::as_str)
            != Some("user")
        || command
            .payload
            .get("scope_id")
            .and_then(serde_json::Value::as_str)
            != Some("default")
    {
        return Err(invalid(command));
    }
    let config: TerminalRuntimeSettings =
        serde_json::from_value(raw.clone()).map_err(|_| invalid(command))?;
    config.validate().map_err(|_| invalid(command))?;
    let store = state.store.clone();
    let expected = command.expected_revision;
    let id = command.command_id;
    let actor = actor.clone();
    tokio::task::spawn_blocking(move || {
        // A known command is resolved by CommitCommand's exact digest/idempotence check.
        // Neither a stale CAS nor a changed payload may publish objects before that check.
        if store.inspect_command_receipt(&actor, id).is_ok() {
            return Ok(None);
        }
        let current = store.read_settings(unleash_store::SettingsScopeKind::User, "default")?;
        if current.revision != expected {
            return Err(unleash_store::StoreError::RevisionConflict {
                current_revision: current.revision,
                expected_revision: expected,
                command_id: Some(id),
            });
        }
        if let Some(old) = current.extra_typed.get("terminal_runtime") {
            let old: TerminalRuntimeSettings = serde_json::from_value(old.clone())?;
            for profile in &config.profiles {
                if let Some(prior) = old
                    .profiles
                    .iter()
                    .find(|p| p.profile_id == profile.profile_id)
                    && (profile.version < prior.version
                        || (profile.version == prior.version && profile != prior))
                {
                    return Err(unleash_store::StoreError::InvalidArgument {
                        reason: None,
                        message: "terminal profile version is immutable".into(),
                    });
                }
            }
        }
        let prepared = config
            .profiles
            .iter()
            .map(|profile| store.prepare_terminal_environment(profile, id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok::<_, unleash_store::StoreError>(Some(prepared))
    })
    .await
    .map_err(|_| invalid(command))?
    .map_err(|e| crate::error::store_error_to_ipc_error(&e, Some(id)))
}
