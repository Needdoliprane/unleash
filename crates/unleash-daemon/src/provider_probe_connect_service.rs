//! Authenticated entry preparation for the two passive transport phases.
//!
//! Only the original, acknowledged claim can start this one-attempt transport.

mod admission;
mod observation;
mod owner;
mod runtime;

use std::fmt;
use std::sync::Arc;

use crate::auth::AuthenticatedSession;
use unleash_domain::{CommandId, ReceiptId};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, ErrorCode, IpcError, ProviderTestPhase, RetryClass,
    TestProviderProfilePayload,
};
use unleash_store::{CallerAuthority, CommitOutcome, StoreError, UnleashStore};

/// The original command remains intact for its canonical digest and receipt.
pub(crate) struct UnresolvedConnectRequest {
    pub(crate) command: CommandEnvelope,
    pub(crate) caller: CallerAuthority,
    pub(crate) request: TestProviderProfilePayload,
    pub(crate) mandate_ref: ReceiptId,
}

impl fmt::Debug for UnresolvedConnectRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnresolvedConnectRequest")
            .field("command_id", &self.command.command_id)
            .finish_non_exhaustive()
    }
}

/// An unresolved request must pass the real writer before starting a broker.
pub(crate) enum ConnectEntry {
    Replay(CommandReceipt),
    RequiresResolution(Box<UnresolvedConnectRequest>),
}

impl fmt::Debug for ConnectEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Replay(receipt) => f
                .debug_struct("ConnectEntry::Replay")
                .field("receipt_id", &receipt.receipt_id)
                .finish(),
            Self::RequiresResolution(request) => f
                .debug_tuple("ConnectEntry::RequiresResolution")
                .field(request)
                .finish(),
        }
    }
}

fn unavailable(command_id: CommandId) -> IpcError {
    IpcError::new(
        ErrorCode::InternalFailure,
        None,
        "Lecture de rejeu connect indisponible ; inspecter le reçu",
        RetryClass::AfterReconciliation,
    )
    .with_command_id(command_id)
}

async fn lookup(
    store: &Arc<UnleashStore>,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<Option<CommandReceipt>, IpcError> {
    let command_id = command.command_id;
    let store = Arc::clone(store);
    let caller = caller.clone();
    let envelope = command.clone();
    tokio::task::spawn_blocking(move || {
        // This existing port classifies the TestProviderProfile PK, lineage and
        // exact wire digest. It does not require a local-validation preparation.
        store.lookup_provider_validation_command(&envelope, &caller)
    })
    .await
    .map_err(|_| unavailable(command_id))?
    .map_err(|error| crate::error::store_error_to_ipc_error(&error, Some(command_id)))
}

fn decode(command: &CommandEnvelope) -> Result<TestProviderProfilePayload, IpcError> {
    let invalid = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Payload de transport fournisseur invalide",
            RetryClass::Never,
        )
        .with_command_id(command.command_id)
    };
    let request: TestProviderProfilePayload =
        serde_json::from_value(serde_json::Value::Object(command.payload.clone()))
            .map_err(|_| invalid())?;
    if !matches!(
        request.phase(),
        ProviderTestPhase::Connect | ProviderTestPhase::Inventory
    ) {
        return Err(invalid());
    }
    Ok(request)
}

fn mandate_reference(
    command: &CommandEnvelope,
    request: &TestProviderProfilePayload,
) -> Result<ReceiptId, IpcError> {
    let hidden = || {
        IpcError::new(
            ErrorCode::NotFoundOrHidden,
            None,
            "Mandat d'effet fournisseur introuvable",
            RetryClass::Never,
        )
        .with_command_id(command.command_id)
    };
    let reference: ReceiptId = request.mandate_ref().parse().map_err(|_| hidden())?;
    if reference.as_uuid().is_nil() || reference.to_string() != request.mandate_ref() {
        return Err(hidden());
    }
    // A syntactically canonical UUID has no authority by itself. The writer
    // still resolves current principal/scope/profile/grant and every bound.
    Ok(reference)
}

/// Checks the real session and canonical replay before any dynamic preparation.
/// This function creates no principal, intent, probe, claim, object or reservation.
pub(crate) async fn begin(
    store: Arc<UnleashStore>,
    session: &AuthenticatedSession,
    command: CommandEnvelope,
) -> Result<ConnectEntry, IpcError> {
    let caller = crate::provider_profile_service::caller_for(session, &command)?;
    if let Some(receipt) = lookup(&store, &command, &caller).await? {
        return Ok(ConnectEntry::Replay(receipt));
    }
    let request = decode(&command)?;
    let mandate_ref = mandate_reference(&command, &request)?;
    Ok(ConnectEntry::RequiresResolution(Box::new(
        UnresolvedConnectRequest {
            command,
            caller,
            request,
            mandate_ref,
        },
    )))
}

/// Covers a preparation failure concurrent with another commit of the same wire.
/// This is the same authenticated classifier, never historical receipt inspection.
pub(crate) async fn replay_after_preparation_failure(
    store: &Arc<UnleashStore>,
    request: &UnresolvedConnectRequest,
    preparation_error: &StoreError,
) -> Result<CommitOutcome, IpcError> {
    if let Some(receipt) = lookup(store, &request.command, &request.caller).await? {
        return Ok(CommitOutcome::IdempotentReplay(receipt));
    }
    Err(crate::error::store_error_to_ipc_error(
        preparation_error,
        Some(request.command.command_id),
    ))
}

/// Prepare a real receipt, then detach one CAS-owned orchestration.
pub(crate) async fn commit(
    state: &Arc<crate::ipc::DaemonRuntimeState>,
    session: AuthenticatedSession,
    command: CommandEnvelope,
) -> Result<CommitOutcome, IpcError> {
    let command_id = command.command_id;
    let caller = crate::provider_profile_service::caller_for(&session, &command)?;
    let outcome = match begin(Arc::clone(&state.store), &session, command.clone()).await? {
        ConnectEntry::Replay(receipt) => CommitOutcome::IdempotentReplay(receipt),
        ConnectEntry::RequiresResolution(request) => {
            crate::ipc::ensure_principal_registered(&state.store, &session).await?;
            let store = Arc::clone(&state.store);
            let prepared = request.command.clone();
            let authority = request.caller.clone();
            let phase = request.request.phase();
            // begin's parsed phase and canonical mandate are part of the same wire.
            if request.request.mandate_ref() != request.mandate_ref.to_string() {
                return Err(unavailable(command_id));
            }
            let result = tokio::task::spawn_blocking(move || {
                let mutation = match phase {
                    ProviderTestPhase::Connect => {
                        let preparation =
                            store.prepare_provider_probe_connect(&prepared, &authority)?;
                        unleash_store::CommandMutation::PrepareProviderProbeConnect {
                            caller: authority.clone(),
                            preparation: Box::new(preparation),
                        }
                    }
                    ProviderTestPhase::Inventory => {
                        let preparation =
                            store.prepare_provider_probe_inventory(&prepared, &authority)?;
                        unleash_store::CommandMutation::PrepareProviderProbeInventory {
                            caller: authority.clone(),
                            preparation: Box::new(preparation),
                        }
                    }
                    _ => {
                        return Err(StoreError::EvidenceUnavailable(
                            "phase de transport absente".into(),
                        ));
                    }
                };
                store.commit_command(unleash_store::CommitCommandRequest {
                    actor_id: authority.actor_id.clone(),
                    envelope: prepared.clone(),
                    target_aggregate: unleash_store::AggregateKind::ProviderProbe,
                    operation_id: None,
                    receipt_state: unleash_protocol::ReceiptState::Accepted,
                    result_ref: Some(format!("provider-probe:{}", prepared.aggregate_id)),
                    retention_days_override: None,
                    fence_assertion: None,
                    mutation,
                    events: vec![],
                    capacity_reservations: vec![],
                    budget_reservations: vec![],
                })
            })
            .await
            .map_err(|_| unavailable(command_id))?;
            match result {
                Ok(outcome) => outcome,
                Err(error) => {
                    replay_after_preparation_failure(&state.store, &request, &error).await?
                }
            }
        }
    };
    runtime::start(state, command, caller, outcome.receipt().receipt_id);
    Ok(outcome)
}
