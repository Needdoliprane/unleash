//! Producteur authentifié F012 de configuration, sans réseau, coffre ou sonde.
//! Toute I/O du Store et la publication des objets restent sur `spawn_blocking`.

use crate::auth::{AuthenticatedSession, Capability};
use std::sync::Arc;
use unleash_domain::DataLocality;
use unleash_protocol::{
    CommandEnvelope, ErrorCode, IpcError, ProviderAuthentication, ProviderPricing, RetryClass,
    SetProviderProfilePayload, canonical_sha256, canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    CommitOutcome, PrepareObjectRequest, ProviderProfileConfiguration, ProviderProfilePreparation,
    ProviderSecretBinding, PublishObjectReferenceRequest, SecretRefState, SecretReferenceInput,
    StoreError, UnleashStore,
};

const OWNER: &str = "provider_profile_preparation";

fn invalid() -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: "Configuration de fournisseur invalide".to_owned(),
    }
}

pub(crate) fn caller_for(
    session: &AuthenticatedSession,
    command: &CommandEnvelope,
) -> Result<CallerAuthority, IpcError> {
    let error = |code, message: &str| {
        IpcError::new(code, None, message, RetryClass::Never).with_command_id(command.command_id)
    };
    if !session.capabilities.contains(&Capability::Administer) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "Preuve administrative requise",
        ));
    }
    if command
        .actor_ref
        .as_ref()
        .is_some_and(|actor| actor != &session.actor_ref)
    {
        return Err(error(
            ErrorCode::AuthenticationFailed,
            "Identité de session incohérente",
        ));
    }
    if session
        .allowed_projects
        .as_ref()
        .is_some_and(|allowed| command.project_id.is_none_or(|id| !allowed.contains(&id)))
    {
        return Err(error(
            ErrorCode::ScopeDenied,
            "Projet hors du périmètre autorisé",
        ));
    }
    Ok(CallerAuthority {
        actor_id: session.actor_id.clone(),
        role: CallerRole::HumanOperator,
    })
}

/// Le chemin commun a déjà autorisé la commande ; cette frontière répète les
/// contrôles d'identité avant le premier accès à la préparation dynamique.
pub(crate) async fn commit(
    store: Arc<UnleashStore>,
    session: AuthenticatedSession,
    command: CommandEnvelope,
) -> Result<CommitOutcome, IpcError> {
    let caller = caller_for(&session, &command)?;
    let command_id = command.command_id;
    if let Some(receipt) = lookup(&store, &command, &caller).await? {
        return Ok(CommitOutcome::IdempotentReplay(receipt));
    }
    if let Err(error) = crate::ipc::ensure_principal_registered(&store, &session).await {
        if let Some(receipt) = lookup(&store, &command, &caller).await? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }
        return Err(error);
    }
    tokio::task::spawn_blocking(move || commit_blocking(&store, &command, &caller))
        .await
        .map_err(|_| {
            IpcError::new(
                ErrorCode::InternalFailure,
                None,
                "Issue de configuration indéterminée ; inspecter le reçu",
                RetryClass::AfterReconciliation,
            )
            .with_command_id(command_id)
        })?
        .map_err(|error| crate::error::store_error_to_ipc_error(&error, Some(command_id)))
}

async fn lookup(
    store: &Arc<UnleashStore>,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<Option<unleash_protocol::CommandReceipt>, IpcError> {
    let store = Arc::clone(store);
    let command = command.clone();
    let caller = caller.clone();
    let id = command.command_id;
    tokio::task::spawn_blocking(move || store.lookup_provider_profile_command(&command, &caller))
        .await
        .map_err(|_| {
            IpcError::new(
                ErrorCode::InternalFailure,
                None,
                "Lecture du reçu indisponible",
                RetryClass::AfterReconciliation,
            )
            .with_command_id(id)
        })?
        .map_err(|error| crate::error::store_error_to_ipc_error(&error, Some(id)))
}

fn commit_blocking(
    store: &UnleashStore,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<CommitOutcome, StoreError> {
    if let Some(receipt) = store.lookup_provider_profile_command(command, caller)? {
        return Ok(CommitOutcome::IdempotentReplay(receipt));
    }
    let result = prepare_and_commit(store, command, caller);
    if result.is_err() {
        // The winner may have committed during resolution, publication or cleanup.
        if let Some(receipt) = store.lookup_provider_profile_command(command, caller)? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }
    }
    result
}

fn secret_binding(
    store: &UnleashStore,
    payload: &SetProviderProfilePayload,
    command: &CommandEnvelope,
) -> Result<Option<ProviderSecretBinding>, StoreError> {
    let reference = match payload.auth() {
        ProviderAuthentication::None => return Ok(None),
        ProviderAuthentication::SecretRef { secret_ref } => secret_ref,
        _ => return Err(invalid()),
    };
    let mut matches = store
        .list_secret_references(Some("provider"))?
        .into_iter()
        .filter(|row| &row.opaque_secret_ref == reference);
    let row = matches
        .next()
        .ok_or_else(|| StoreError::NotFound("Référence fournisseur indisponible".to_owned()))?;
    if matches.next().is_some() {
        return Err(invalid());
    }
    let (scope_kind, scope_id) = command
        .project_id
        .map_or(("user", "default".to_owned()), |id| {
            ("project", id.to_string())
        });
    if row.namespace != "provider"
        || row.purpose != "api_key"
        || row.target_kind != "provider_profile"
        || row.target_id != command.aggregate_id.to_string()
        || row.scope_kind != scope_kind
        || row.scope_id != scope_id
        || row.secret_kind.trim().is_empty()
        || row.backend_kind.trim().is_empty()
        || row.status != SecretRefState::Active
        || row.local_revoked_at_ms.is_some()
    {
        return Err(StoreError::Unauthorized(
            "Liaison de référence fournisseur incompatible".to_owned(),
        ));
    }
    Ok(Some(ProviderSecretBinding {
        revision: row.revision,
        reference: SecretReferenceInput {
            secret_ref_id: row.secret_ref_id,
            secret_ref: row.opaque_secret_ref,
            namespace: row.namespace,
            purpose: row.purpose,
            secret_kind: row.secret_kind,
            target_kind: row.target_kind,
            target_id: row.target_id,
            scope_kind: row.scope_kind,
            scope_id: row.scope_id,
            backend_kind: row.backend_kind,
        },
    }))
}

fn configuration(
    store: &UnleashStore,
    command: &CommandEnvelope,
) -> Result<ProviderProfileConfiguration, StoreError> {
    let raw: SetProviderProfilePayload =
        serde_json::from_value(serde_json::Value::Object(command.payload.clone()))
            .map_err(|_| invalid())?;
    let payload = raw.with_normalized_endpoint().map_err(|_| invalid())?;
    match payload.pricing() {
        ProviderPricing::Catalog { .. } => {
            return Err(StoreError::NotFound(
                "Catalogue tarifaire de confiance indisponible".to_owned(),
            ));
        }
        ProviderPricing::NoneLocal if payload.locality() != DataLocality::LocalOnly => {
            return Err(invalid());
        }
        ProviderPricing::NoneLocal
        | ProviderPricing::UserImport { .. }
        | ProviderPricing::UserDeclared { .. } => {}
        _ => return Err(invalid()),
    }
    let secret_binding = secret_binding(store, &payload, command)?;
    let version = command
        .expected_revision
        .as_u64()
        .checked_add(1)
        .filter(|version| i64::try_from(*version).is_ok())
        .ok_or_else(invalid)?;
    Ok(ProviderProfileConfiguration {
        schema_version: 1,
        profile_id: command.aggregate_id,
        profile_version: version,
        project_id: command.project_id,
        payload,
        secret_binding,
        catalog_binding: None,
    })
}

fn prepare_and_commit(
    store: &UnleashStore,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<CommitOutcome, StoreError> {
    let configuration = configuration(store, command)?;
    let bytes = canonicalize_to_vec(&configuration)?;
    let hash = canonical_sha256(&configuration)?;
    let owner = store.config().id_generator.new_uuid().to_string();
    let handle = store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: u64::try_from(bytes.len()).map_err(|_| invalid())?,
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
            source_field: "configuration".to_owned(),
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
        target_aggregate: AggregateKind::ProviderProfile,
        operation_id: None,
        receipt_state: unleash_protocol::ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::SetProviderProfile {
            caller: caller.clone(),
            preparation: Box::new(ProviderProfilePreparation {
                configuration,
                content_object_hash: hash,
            }),
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    });
    store.retire_owned_object_reference(published, "provider_profile_preparation_finished")?;
    result
}
