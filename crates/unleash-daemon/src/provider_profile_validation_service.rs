//! Validation F012 locale ; aucun transport fournisseur ni accès au coffre.

use crate::auth::{AuthenticatedSession, Capability};
use serde::Deserialize;
use std::sync::Arc;
use unleash_domain::{ProjectId, ProviderProbeId};
use unleash_protocol::{
    CommandEnvelope, ErrorCode, IpcError, ProviderTestPhase, QueryEnvelope, QueryResponse,
    RetryClass, TestProviderProfilePayload, canonical_sha256, canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, CallerAuthority, CommandMutation, CommitCommandRequest, CommitOutcome,
    PrepareObjectRequest, ProviderValidationPreparation, PublishObjectReferenceRequest, StoreError,
    UnleashStore,
};

const OWNER: &str = "provider_probe_validation_preparation";

fn unavailable() -> IpcError {
    IpcError::new(
        ErrorCode::InternalFailure,
        None,
        "Issue de validation locale indéterminée ; inspecter le reçu",
        RetryClass::AfterReconciliation,
    )
}

async fn lookup(
    store: &Arc<UnleashStore>,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<Option<unleash_protocol::CommandReceipt>, IpcError> {
    let store = store.clone();
    let command = command.clone();
    let caller = caller.clone();
    let id = command.command_id;
    tokio::task::spawn_blocking(move || store.lookup_provider_validation_command(&command, &caller))
        .await
        .map_err(|_| unavailable().with_command_id(id))?
        .map_err(|error| crate::error::store_error_to_ipc_error(&error, Some(id)))
}

fn unresolved_effect_mandate(command: &CommandEnvelope) -> IpcError {
    // No qualified producer currently binds an active probe to a network mandate.
    // A configuration receipt is a cause for local validation only. In particular,
    // do not derive an empty or permissive authority from that receipt or from the
    // caller's administrative capability, and do not prepare an effect intention.
    IpcError::new(
        ErrorCode::NotFoundOrHidden,
        None,
        "Mandat d'effet fournisseur introuvable",
        RetryClass::Never,
    )
    .with_command_id(command.command_id)
}

/// Rejoue avant tout accès aux octets et utilise le pipeline commun pour la mutation.
pub(crate) async fn commit(
    store: Arc<UnleashStore>,
    session: AuthenticatedSession,
    command: CommandEnvelope,
) -> Result<CommitOutcome, IpcError> {
    let caller = crate::provider_profile_service::caller_for(&session, &command)?;
    if let Some(receipt) = lookup(&store, &command, &caller).await? {
        return Ok(CommitOutcome::IdempotentReplay(receipt));
    }
    let payload: TestProviderProfilePayload = serde_json::from_value(serde_json::Value::Object(
        command.payload.clone(),
    ))
    .map_err(|_| {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Payload de validation locale invalide",
            RetryClass::Never,
        )
        .with_command_id(command.command_id)
    })?;
    if payload.phase() == ProviderTestPhase::Connect {
        return Err(unresolved_effect_mandate(&command));
    }
    let phase = payload.phase();
    if !matches!(
        phase,
        ProviderTestPhase::ValidateConfig | ProviderTestPhase::GenerationProbe
    ) {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Phase fournisseur non prise en charge par la validation locale",
            RetryClass::Never,
        )
        .with_command_id(command.command_id));
    }
    if let Err(error) = crate::ipc::ensure_principal_registered(&store, &session).await {
        if let Some(receipt) = lookup(&store, &command, &caller).await? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }
        return Err(error);
    }
    let id = command.command_id;
    tokio::task::spawn_blocking(move || {
        if phase == ProviderTestPhase::GenerationProbe {
            store.commit_provider_generation_preparation(&command, &caller)
        } else {
            commit_blocking(&store, &command, &caller)
        }
    })
    .await
    .map_err(|_| unavailable().with_command_id(id))?
    .map_err(|error| crate::error::store_error_to_ipc_error(&error, Some(id)))
}

fn commit_blocking(
    store: &UnleashStore,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<CommitOutcome, StoreError> {
    if let Some(receipt) = store.lookup_provider_validation_command(command, caller)? {
        return Ok(CommitOutcome::IdempotentReplay(receipt));
    }
    let result = prepare_and_commit(store, command, caller);
    if result.is_err()
        && let Some(receipt) = store.lookup_provider_validation_command(command, caller)?
    {
        return Ok(CommitOutcome::IdempotentReplay(receipt));
    }
    result
}

fn prepare_and_commit(
    store: &UnleashStore,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<CommitOutcome, StoreError> {
    let observation = store.prepare_provider_profile_validation(command, caller)?;
    let bytes = canonicalize_to_vec(&observation)?;
    let hash = canonical_sha256(&observation)?;
    let owner = store.config().id_generator.new_uuid().to_string();
    let handle = store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: u64::try_from(bytes.len()).map_err(|_| StoreError::InvalidArgument {
            reason: None,
            message: "observation locale trop grande".to_owned(),
        })?,
        media_type: "application/json".to_owned(),
        owner_kind: OWNER.to_owned(),
        owner_id: owner.clone(),
    })?;
    let publication = (|| {
        store.write_prepared_object_bytes(&handle, &bytes)?;
        store.publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id.clone(),
            object_hash: hash,
            object_generation: handle.object_generation,
            owner_kind: OWNER.to_owned(),
            owner_id: owner,
            source_field: "observation".to_owned(),
        })
    })();
    let published = match publication {
        Ok(reference) => reference,
        Err(error) => {
            store.release_pin(&handle.pin_id)?;
            return Err(error);
        }
    };
    let result = store.commit_command(CommitCommandRequest {
        actor_id: caller.actor_id.clone(),
        envelope: command.clone(),
        target_aggregate: AggregateKind::ProviderProbe,
        operation_id: None,
        receipt_state: unleash_protocol::ReceiptState::Completed,
        result_ref: Some(format!(
            "provider-probe:{}",
            ProviderProbeId::from_uuid(command.aggregate_id)
        )),
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::ValidateProviderProfile {
            caller: caller.clone(),
            preparation: Box::new(ProviderValidationPreparation {
                observation,
                content_object_hash: hash,
            }),
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    });
    let retired = store.retire_owned_object_reference(published, "validation_candidate_finished");
    let outcome = result?;
    retired?;
    Ok(outcome)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileQuery {
    profile_id: uuid::Uuid,
    profile_version: u64,
}

fn invalid_query() -> IpcError {
    IpcError::new(
        ErrorCode::InvalidArgument,
        None,
        "Lecture de profil invalide",
        RetryClass::Never,
    )
}

fn require_profile_read(
    session: &AuthenticatedSession,
    project_id: Option<ProjectId>,
) -> Result<(), IpcError> {
    if !session.capabilities.contains(&Capability::QueryState) {
        return Err(IpcError::new(
            ErrorCode::CapabilityDenied,
            None,
            "Lecture de profil non autorisée",
            RetryClass::Never,
        ));
    }
    if session
        .allowed_projects
        .as_ref()
        .is_some_and(|allowed| project_id.is_none_or(|id| !allowed.contains(&id)))
    {
        return Err(IpcError::new(
            ErrorCode::ScopeDenied,
            None,
            "Projet hors du périmètre autorisé",
            RetryClass::Never,
        ));
    }
    Ok(())
}

/// Lecture exacte, sans effet et avec le watermark de la même projection `SQLite`.
pub(crate) async fn query(
    store: Arc<UnleashStore>,
    session: AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    require_profile_read(&session, query.project_id)?;
    let payload: ProfileQuery = serde_json::from_value(serde_json::Value::Object(query.payload))
        .map_err(|_| invalid_query())?;
    if payload.profile_id.is_nil()
        || !(1..=9_007_199_254_740_991).contains(&payload.profile_version)
    {
        return Err(invalid_query());
    }
    let projection = tokio::task::spawn_blocking(move || {
        store.get_provider_profile_snapshot(
            payload.profile_id,
            payload.profile_version,
            query.project_id,
        )
    })
    .await
    .map_err(|_| unavailable())?
    .map_err(|error| crate::error::store_error_to_ipc_error(&error, None))?;
    let profile = projection.value.ok_or_else(|| {
        IpcError::new(
            ErrorCode::NotFoundOrHidden,
            None,
            "Profil indisponible",
            RetryClass::Never,
        )
    })?;
    Ok(QueryResponse {
        request_id: query.request_id,
        watermark: Some(projection.cursor),
        result: serde_json::to_value(profile).map_err(|_| unavailable())?,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileListQuery {
    after_profile_id: Option<uuid::Uuid>,
    #[serde(default = "default_page_limit")]
    limit: u32,
}

const fn default_page_limit() -> u32 {
    50
}

pub(crate) async fn list(
    store: Arc<UnleashStore>,
    session: &AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    require_profile_read(session, query.project_id)?;
    let payload: ProfileListQuery =
        serde_json::from_value(serde_json::Value::Object(query.payload))
            .map_err(|_| invalid_query())?;
    let projection = tokio::task::spawn_blocking(move || {
        store.list_provider_profiles_snapshot(
            query.project_id,
            payload.after_profile_id,
            payload.limit,
        )
    })
    .await
    .map_err(|_| unavailable())?
    .map_err(|error| crate::error::store_error_to_ipc_error(&error, None))?;
    Ok(QueryResponse {
        request_id: query.request_id,
        watermark: Some(projection.cursor),
        result: serde_json::to_value(projection.value).map_err(|_| unavailable())?,
    })
}
