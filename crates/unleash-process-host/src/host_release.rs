//! Native retirement under the existing operation gate; replies never retain it.
use super::{
    HostControlSecret, HostInputCutpoint, Ordering, ProcessHostError, ProcessInspector,
    ProcessVerification, ServerShared, UnixStream, lock_operation, store_control_secret,
    verify_secret, write_frame,
};
use crate::control_release::ReleaseEnvelope;
use crate::{
    ControlReacquisitionAck, ControlReacquisitionRequest, ControlReacquisitionResponse,
    ControlReleaseAck, ControlReleaseRequest, ControlReleaseResponse, HostControlState,
    OsProcessInspector,
};

enum ReleaseOutcome {
    Retired(ControlReleaseAck),
    Inspected(ControlReleaseAck),
}

pub(super) fn require_input(shared: &ServerShared) -> Result<(), ProcessHostError> {
    let manifest = shared
        .manifest
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if manifest
        .control_state
        .as_ref()
        .is_some_and(|state| !state.permits_input())
    {
        return Err(ProcessHostError::AuthenticationFailed {
            reason: "terminal control has been retired or requires reconciliation".into(),
        });
    }
    Ok(())
}

fn identity(
    shared: &ServerShared,
) -> Result<unleash_platform::OsProcessIdentity, ProcessHostError> {
    let report = shared.anchor.verification_report();
    let expected = shared.anchor.expected_identity();
    if report.reaped
        || !report.sigchld_qualified
        || report.identity_status != "match"
        || OsProcessInspector.verify_identity(&expected)? != ProcessVerification::Match
    {
        return Err(ProcessHostError::IdentityUnverified {
            reason: "terminal control requires the same live anchored child".into(),
        });
    }
    Ok(expected)
}

fn rotate(shared: &ServerShared, value: &str) -> Result<(), ProcessHostError> {
    let next = HostControlSecret::new(value)?;
    if verify_secret(shared, value).is_ok() {
        return Err(ProcessHostError::InvalidArgument {
            reason: "terminal control requires a distinct replacement credential".into(),
        });
    }
    store_control_secret(&shared.control_secret_path, &next)?;
    *shared
        .control_secret
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = next;
    shared
        .auth_generation
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            value.checked_add(1)
        })
        .map_err(|_| ProcessHostError::LaunchOutcomeUnknown {
            reason: "native authentication generation exhausted".into(),
        })?;
    Ok(())
}

fn control_state(shared: &ServerShared) -> Option<HostControlState> {
    shared
        .manifest
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .control_state
        .clone()
}

fn persist_state(shared: &ServerShared, state: HostControlState) -> Result<(), ProcessHostError> {
    let mut manifest = shared
        .manifest
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Set memory first: an uncertain disk write cannot reopen input control.
    manifest.control_state = Some(state);
    manifest.save_atomic(&shared.manifest_path)
}

fn release(
    request: &ControlReleaseRequest,
    shared: &ServerShared,
) -> Result<ReleaseOutcome, ProcessHostError> {
    bounded_credentials(&request.current_secret, &request.next_secret)?;
    verify_secret(shared, &request.current_secret)?;
    let key = &request.key;
    match control_state(shared) {
        Some(HostControlState::Released { ack }) if ack.key == *key => {
            verify_secret(shared, &request.next_secret)?;
            identity(shared)?;
            return Ok(ReleaseOutcome::Inspected(*ack));
        }
        Some(HostControlState::Active { .. }) => {}
        _ => {
            return Err(ProcessHostError::LaunchOutcomeUnknown {
                reason: "native retirement is unqualified, incomplete or from another effect"
                    .into(),
            });
        }
    }
    let os_identity = identity(shared)?;
    let current = shared
        .fencing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .current();
    let mut candidate = *shared
        .fencing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    candidate.check_recovery_handshake(&key.header, false)?;
    if candidate.current() != current {
        return Err(ProcessHostError::StaleEpochOrFence {
            reason: "retirement requires the exact current native control".into(),
        });
    }
    let (initial, host_id, process_id, terminal) = {
        let manifest = shared
            .manifest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            manifest.allocation_context_digest,
            manifest.host_id,
            manifest.process_id,
            manifest.terminal_id,
        )
    };
    let terminal_id = terminal.ok_or_else(|| ProcessHostError::InvalidArgument {
        reason: "retirement requires a registered PTY".into(),
    })?;
    let (last_confirmed_input_seq, unknown_input_seqs) = shared.inputs.summary();
    if initial != Some(key.initial_context_digest) || !unknown_input_seqs.is_empty() {
        return Err(ProcessHostError::IdentityUnverified {
            reason: "retirement source or pending native input differs".into(),
        });
    }
    let _validated = HostControlSecret::new(request.next_secret.clone())?;
    if verify_secret(shared, &request.next_secret).is_ok() {
        return Err(ProcessHostError::InvalidArgument {
            reason: "retirement requires a new credential".into(),
        });
    }
    persist_state(shared, HostControlState::Retiring { key: key.clone() })?;
    rotate(shared, &request.next_secret)?;
    let ack = ControlReleaseAck {
        key: key.clone(),
        host_id,
        process_id,
        terminal_id,
        os_identity,
        durable_log_seq: shared.spool.flush_and_sync_durable()?,
        last_confirmed_input_seq,
        unknown_input_seqs,
    };
    commit_retired(shared, &ack)?;
    Ok(ReleaseOutcome::Retired(ack))
}

pub(super) fn commit_retired(
    shared: &ServerShared,
    ack: &ControlReleaseAck,
) -> Result<(), ProcessHostError> {
    if let Err(error) = persist_state(
        shared,
        HostControlState::Released {
            ack: Box::new(ack.clone()),
        },
    ) {
        // A failed manifest commit must not leave a replayable success in RAM.
        shared
            .manifest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .control_state = Some(HostControlState::Retiring {
            key: ack.key.clone(),
        });
        return Err(error);
    }
    Ok(())
}

enum ReacquisitionSource {
    Released(Box<ControlReleaseAck>),
    Inspected(Box<ControlReacquisitionAck>),
}
fn reacquisition_source(
    request: &ControlReacquisitionRequest,
    shared: &ServerShared,
) -> Result<ReacquisitionSource, ProcessHostError> {
    let transfer = &request.transfer;
    match control_state(shared) {
        Some(HostControlState::Active {
            last_reacquisition: Some(ack),
        }) if ack.released_by == request.released_by
            && ack.transfer.header == transfer.header
            && ack.transfer.dispatch_id == transfer.dispatch_id
            && ack.transfer.initial_context_digest == transfer.initial_context_digest =>
        {
            verify_secret(shared, &transfer.next_secret)?;
            identity(shared)?;
            let fence = shared
                .fencing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .current();
            if fence.epoch != ack.transfer.header.daemon_epoch
                || fence.fence != ack.transfer.header.resource_fence
            {
                return Err(ProcessHostError::StaleEpochOrFence {
                    reason: "reactivation acknowledgement is no longer the current native control"
                        .into(),
                });
            }
            Ok(ReacquisitionSource::Inspected(ack))
        }
        Some(HostControlState::Released { ack }) if ack.key.dispatch_id == request.released_by => {
            Ok(ReacquisitionSource::Released(ack))
        }
        _ => Err(ProcessHostError::LaunchOutcomeUnknown {
            reason: "reactivation requires the exact qualified retirement".into(),
        }),
    }
}

fn reacquire(
    request: &ControlReacquisitionRequest,
    shared: &ServerShared,
) -> Result<ControlReacquisitionAck, ProcessHostError> {
    let transfer = &request.transfer;
    bounded_credentials(&transfer.current_secret, &transfer.next_secret)?;
    verify_secret(shared, &transfer.current_secret)?;
    let source = match reacquisition_source(request, shared)? {
        ReacquisitionSource::Inspected(ack) => return Ok(*ack),
        ReacquisitionSource::Released(ack) => ack,
    };
    let os_identity = identity(shared)?;
    let mut candidate = *shared
        .fencing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if transfer.initial_context_digest != source.key.initial_context_digest
        || transfer.header.resource_fence <= candidate.current().fence
    {
        return Err(ProcessHostError::StaleEpochOrFence {
            reason: "reactivation requires its original allocation and a higher fence".into(),
        });
    }
    candidate.check_recovery_handshake(&transfer.header, false)?;
    let (last_confirmed_input_seq, unknown_input_seqs) = shared.inputs.summary();
    if !unknown_input_seqs.is_empty() {
        return Err(ProcessHostError::LaunchOutcomeUnknown {
            reason: "reactivation cannot cross unknown native input".into(),
        });
    }
    let effective_environment_digest = shared
        .manifest
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .effective_environment_digest
        .ok_or_else(|| ProcessHostError::IdentityUnverified {
            reason: "reactivation environment was not qualified".into(),
        })?;
    let _validated = HostControlSecret::new(transfer.next_secret.clone())?;
    if verify_secret(shared, &transfer.next_secret).is_ok() {
        return Err(ProcessHostError::InvalidArgument {
            reason: "reactivation requires a new credential".into(),
        });
    }
    persist_state(
        shared,
        HostControlState::Reactivating {
            released_by: request.released_by,
            header: transfer.header.clone(),
            dispatch_id: transfer.dispatch_id,
        },
    )?;
    rotate(shared, &transfer.next_secret)?;
    *shared
        .fencing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = candidate;
    let mut manifest = shared
        .manifest
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let ack = ControlReacquisitionAck {
        released_by: request.released_by,
        transfer: crate::ControlTransferAck {
            header: transfer.header.clone(),
            dispatch_id: transfer.dispatch_id,
            initial_context_digest: transfer.initial_context_digest,
            effective_environment_digest,
            os_identity,
            durable_log_seq: shared.spool.flush_and_sync_durable()?,
            last_confirmed_input_seq,
            unknown_input_seqs,
        },
    };
    manifest.daemon_epoch = transfer.header.daemon_epoch;
    manifest.highest_fence = transfer.header.resource_fence;
    manifest.control_state = Some(HostControlState::Active {
        last_reacquisition: Some(Box::new(ack.clone())),
    });
    if let Err(error) = manifest.save_atomic(&shared.manifest_path) {
        manifest.control_state = Some(HostControlState::Reactivating {
            released_by: request.released_by,
            header: transfer.header.clone(),
            dispatch_id: transfer.dispatch_id,
        });
        return Err(error);
    }
    Ok(ack)
}

fn bounded_credentials(current: &str, next: &str) -> Result<(), ProcessHostError> {
    // Existing generated capabilities are 64 ASCII bytes. Older valid bounded
    // capabilities remain readable; private frame size is a separate limit.
    if current.len() > 1024 || next.len() > 1024 {
        return Err(ProcessHostError::InvalidArgument {
            reason: "native control credential exceeds its bound".into(),
        });
    }
    Ok(())
}

pub(super) fn respond(
    stream: &mut UnixStream,
    request: &ReleaseEnvelope,
    shared: &ServerShared,
    bound_auth_gen: &mut u64,
) -> Result<(), ProcessHostError> {
    let (payload, retired) = {
        let _operation = lock_operation(shared);
        let result = match request {
            ReleaseEnvelope::ReleaseControlV1(request) => {
                let (response, retired) = match release(request, shared) {
                    Ok(ReleaseOutcome::Retired(ack)) => {
                        (ControlReleaseResponse::Ack { ack: Box::new(ack) }, true)
                    }
                    Ok(ReleaseOutcome::Inspected(ack)) => {
                        (ControlReleaseResponse::Ack { ack: Box::new(ack) }, false)
                    }
                    Err(error) => (
                        ControlReleaseResponse::Refused {
                            error: error.to_ipc_error(Some(request.key.header.command_id)),
                        },
                        false,
                    ),
                };
                let acknowledged = matches!(response, ControlReleaseResponse::Ack { .. });
                (serde_json::to_vec(&response)?, retired, acknowledged)
            }
            ReleaseEnvelope::AcquireReleasedControlV1(request) => {
                let response = match reacquire(request, shared) {
                    Ok(ack) => ControlReacquisitionResponse::Ack { ack: Box::new(ack) },
                    Err(error) => ControlReacquisitionResponse::Refused {
                        error: error.to_ipc_error(Some(request.transfer.header.command_id)),
                    },
                };
                let acknowledged = matches!(response, ControlReacquisitionResponse::Ack { .. });
                (serde_json::to_vec(&response)?, false, acknowledged)
            }
        };
        if result.2 {
            *bound_auth_gen = shared.auth_generation.load(Ordering::SeqCst);
        }
        (result.0, result.1)
    };
    if retired {
        shared
            .cutpoints
            .hit(HostInputCutpoint::ReleaseAppliedBeforeAck)?;
    }
    write_frame(stream, &payload)?;
    Ok(())
}
