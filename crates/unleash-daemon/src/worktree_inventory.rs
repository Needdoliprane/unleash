// SPDX-License-Identifier: Apache-2.0
//! Inventaire des worktrees d'un dépôt rattaché : requête `ListWorktrees` et disponibilité
//! d'adoption d'une cible d'intégration (`REQ-GIT-001`, `AC-F010-001`, Q-10, ADR-09).
//!
//! L'inventaire expose les faits observés. La disponibilité d'une cible est évaluée à part, quand
//! la cible effective du workflow est connue : Unleash ne met jamais à jour une branche checkout
//! dans un worktree, y compris pendant un rebase ou un bisect, et une cible ainsi occupée est
//! annoncée `blocked_target_checkout` avant toute mission.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use unleash_domain::ProjectId;
use unleash_protocol::{
    ErrorCode, IpcError, InventoryCoverageStatus, ListWorktreesPayload, ListWorktreesResult,
    QueryEnvelope, QueryResponse, ReasonCode, RetryClass, WorktreeItem,
    is_worktree_branch_ref,
};
use unleash_store::StoreError;

use crate::git_inspector::{self, InventoryError};
use crate::ipc::DaemonRuntimeState;

/// Disponibilité d'une cible d'intégration à l'adoption d'un dépôt (Q-10, ADR-09).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IntegrationReadiness {
    /// La cible est checkout, ou réservée par un rebase ou un bisect, dans le worktree cité.
    BlockedTargetCheckout {
        /// Chemin observé du worktree qui occupe la cible.
        worktree: String,
    },
    /// La collecte ou une association est incomplète : la cible n'est pas supposée libre.
    Unknown,
    /// Aucun worktree observé ne checkout ni ne réserve la cible.
    NotCheckedOut,
}

/// Évalue la disponibilité d'une cible, donnée par son nom complet de branche, à partir d'un
/// inventaire. Une cible observée checkout est annoncée bloquée même si la collecte est
/// incomplète ; une collecte incomplète ne laisse jamais conclure que la cible est libre.
#[must_use]
pub fn integration_readiness(
    inventory: &ListWorktreesResult,
    target_ref: &str,
) -> IntegrationReadiness {
    if !is_worktree_branch_ref(target_ref) {
        return IntegrationReadiness::Unknown;
    }
    let occupied = |item: &WorktreeItem| {
        item.branch_ref.as_deref() == Some(target_ref)
            || item
                .operation_branches
                .iter()
                .any(|association| association.branch_ref == target_ref)
    };
    if let Some(item) = inventory.items.iter().find(|item| occupied(item)) {
        return IntegrationReadiness::BlockedTargetCheckout {
            worktree: item.path.clone(),
        };
    }
    if inventory.coverage.status == InventoryCoverageStatus::Complete {
        IntegrationReadiness::NotCheckedOut
    } else {
        IntegrationReadiness::Unknown
    }
}

fn invalid_argument(message: &str) -> IpcError {
    IpcError::new(ErrorCode::InvalidArgument, None, message, RetryClass::Never)
}

fn internal_failure(message: &str) -> IpcError {
    IpcError::new(ErrorCode::InternalFailure, None, message, RetryClass::Never)
}

fn hidden() -> IpcError {
    // Aucune information ne révèle si c'est le projet ou le dépôt qui manque.
    IpcError::new(
        ErrorCode::NotFoundOrHidden,
        None,
        "Dépôt introuvable ou masqué dans le projet",
        RetryClass::Never,
    )
}

/// Projette un échec de collecte vers l'erreur commune du contrat F010.
fn to_ipc_error(error: InventoryError) -> IpcError {
    match error {
        InventoryError::GitUnavailable | InventoryError::VersionUnsupported => {
            IpcError::from_reason(
                ReasonCode::GitVersionUnsupported,
                "Git système absent, non reconnu ou inférieur à 2.39 (ADR-09)",
            )
        }
        InventoryError::UnreadableRepository => invalid_argument(
            "La racine enregistrée du dépôt n'est pas lisible comme dépôt Git",
        ),
        InventoryError::DeadlineReached | InventoryError::StreamLimitReached => {
            IpcError::from_reason(
                ReasonCode::LocalResourceExhausted,
                "Inventaire au-delà des bornes locales de temps ou de volume",
            )
        }
        InventoryError::GitFailed | InventoryError::UnexpectedOutput => {
            internal_failure("Inspection Git impossible")
        }
    }
}

/// Serve `ListWorktrees` : résout le dépôt dans ceux que le projet a enregistrés, collecte
/// l'inventaire hors de toute transaction, puis contrôle le résultat contre le contrat fermé
/// avant de le restituer. La capacité et la portée projet ont déjà été vérifiées par la
/// requête ; `project_id` est requis et ne peut donc désigner qu'un projet autorisé.
///
/// # Errors
/// Retourne [`IpcError`] pour une requête invalide, un projet ou un dépôt introuvable ou masqué,
/// une collecte impossible, ou un résultat incohérent avec le contrat.
pub(crate) async fn query(
    state: &Arc<DaemonRuntimeState>,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    let project = query
        .project_id
        .ok_or_else(|| invalid_argument("ListWorktrees requiert project_id"))?;
    let payload: ListWorktreesPayload = serde_json::from_value(Value::Object(query.payload))
        .map_err(|_| invalid_argument("Payload ListWorktrees invalide"))?;
    payload
        .validate()
        .map_err(|_| invalid_argument("Paramètres ListWorktrees invalides"))?;
    // Aucune collecte n'est conservée dans cette étape : un curseur présent, bien formé ou lié à
    // une autre portée, désigne toujours une collecte devenue indisponible.
    if payload.opaque_cursor.is_some() {
        return Err(IpcError::from_reason(
            ReasonCode::CursorExpired,
            "Collecte indisponible ; recommencer la lecture sans curseur",
        ));
    }

    let repo_id = payload.repo_id.clone();
    let store = Arc::clone(&state.store);
    let snapshot = tokio::task::spawn_blocking(move || {
        store.project_worktree_registry(project, &repo_id)
    })
    .await
    .map_err(|_| internal_failure("Lecture du projet interrompue"))?
    .map_err(|error| match error {
        StoreError::NotFound(_) => hidden(),
        other => crate::error::store_error_to_ipc_error(&other, None),
    })?;
    let (project_record, managed_paths) = snapshot.value;
    let Some(repository) = project_record
        .repositories
        .iter()
        .find(|repository| repository.repo_id == payload.repo_id)
    else {
        return Err(hidden());
    };
    let managed: HashSet<String> = managed_paths.into_iter().collect();
    let root = PathBuf::from(&repository.root_path);
    let inventory = git_inspector::collect(&root, Some(&managed))
        .await
        .map_err(to_ipc_error)?;
    let page_size = payload.effective_page_size();
    // Plus petite étape honnête : sans collecte conservée, une page ne peut pas être suivie.
    if inventory.items.len() > usize::from(page_size) {
        return Err(IpcError::from_reason(
            ReasonCode::LocalResourceExhausted,
            "Inventaire plus grand que la page demandée ; aucune collecte n'est conservée pour la suite",
        ));
    }
    let observed_at_ms = state.store.config().clock.now_ms();
    let result = ListWorktreesResult {
        project_id: project,
        repo_id: payload.repo_id,
        object_format: inventory.object_format,
        git_profile_digest: inventory.git_profile_digest,
        observed_at: DateTime::<Utc>::from_timestamp_millis(observed_at_ms.as_i64())
            .unwrap_or(DateTime::UNIX_EPOCH),
        items: inventory.items,
        coverage: inventory.coverage,
        next_cursor: None,
    };
    result
        .check_contract(project, &result.repo_id, page_size)
        .map_err(|_| internal_failure("Inventaire des worktrees incohérent avec le contrat"))?;
    let result = serde_json::to_value(&result)
        .map_err(|_| internal_failure("Sérialisation de l'inventaire impossible"))?;
    Ok(QueryResponse {
        request_id: query.request_id,
        watermark: Some(snapshot.cursor),
        result,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use unleash_domain::Sha256Digest;
    use unleash_protocol::{
        GitObjectFormat, GitObjectId, InventoryCoverage, OperationBranch,
        OperationBranchProvenance, ReservedBranchOperation, WorktreeDirty, WorktreeOwnership,
    };

    use super::*;

    fn item(path: &str, branch: Option<&str>) -> WorktreeItem {
        WorktreeItem {
            path: path.to_owned(),
            head_oid: Some(
                GitObjectId::parse(GitObjectFormat::Sha1, &"2".repeat(40)).unwrap(),
            ),
            branch_ref: branch.map(ToOwned::to_owned),
            dirty: WorktreeDirty::Dirty,
            operations: Vec::new(),
            operations_complete: true,
            operation_branches: Vec::new(),
            operation_branches_complete: true,
            ownership: WorktreeOwnership::External,
        }
    }

    fn inventory(items: Vec<WorktreeItem>, complete: bool) -> ListWorktreesResult {
        ListWorktreesResult {
            project_id: ProjectId::from_u128(0x10),
            repo_id: String::from("repo-01"),
            object_format: GitObjectFormat::Sha1,
            git_profile_digest: Sha256Digest::parse_hex(&"3".repeat(64)).unwrap(),
            observed_at: DateTime::parse_from_rfc3339("2026-11-24T12:00:00Z")
                .unwrap()
                .to_utc(),
            items,
            coverage: InventoryCoverage {
                status: if complete {
                    InventoryCoverageStatus::Complete
                } else {
                    InventoryCoverageStatus::Incomplete
                },
            },
            next_cursor: None,
        }
    }

    // Q-10 / AC-F010-001 : une cible checkout dans un worktree humain est annoncée bloquée.
    #[test]
    fn a_checked_out_target_is_blocked_even_when_dirty() {
        let human = inventory(
            vec![item("/w/human", Some("refs/heads/integration")), item("/w/other", None)],
            true,
        );
        assert_eq!(
            integration_readiness(&human, "refs/heads/integration"),
            IntegrationReadiness::BlockedTargetCheckout {
                worktree: String::from("/w/human")
            }
        );
        assert_eq!(
            integration_readiness(&human, "refs/heads/main"),
            IntegrationReadiness::NotCheckedOut
        );
    }

    #[test]
    fn a_target_reserved_by_a_rebase_or_a_bisect_is_blocked() {
        let mut rebasing = item("/w/rebasing", None);
        rebasing.operations = vec![unleash_protocol::GitOperation::Rebase];
        rebasing.operation_branches = vec![OperationBranch {
            operation: ReservedBranchOperation::Rebase,
            branch_ref: String::from("refs/heads/integration"),
            provenance: OperationBranchProvenance::RebaseMetadata,
        }];
        let collection = inventory(vec![rebasing], true);
        assert_eq!(
            integration_readiness(&collection, "refs/heads/integration"),
            IntegrationReadiness::BlockedTargetCheckout {
                worktree: String::from("/w/rebasing")
            }
        );
        assert_eq!(
            integration_readiness(&collection, "refs/heads/main"),
            IntegrationReadiness::NotCheckedOut
        );
    }

    #[test]
    fn an_incomplete_collection_never_leaves_a_target_free() {
        let incomplete = inventory(vec![item("/w/human", Some("refs/heads/other"))], false);
        assert_eq!(
            integration_readiness(&incomplete, "refs/heads/integration"),
            IntegrationReadiness::Unknown
        );
        // La cible observée checkout reste un fait positif.
        let seen = inventory(vec![item("/w/human", Some("refs/heads/integration"))], false);
        assert_eq!(
            integration_readiness(&seen, "refs/heads/integration"),
            IntegrationReadiness::BlockedTargetCheckout {
                worktree: String::from("/w/human")
            }
        );
    }

    #[test]
    fn a_target_that_is_not_a_full_branch_ref_stays_unknown() {
        let collection = inventory(vec![item("/w/human", Some("refs/heads/integration"))], true);
        for target in ["", "integration", "refs/remotes/origin/integration"] {
            assert_eq!(
                integration_readiness(&collection, target),
                IntegrationReadiness::Unknown,
                "cible acceptée : {target:?}"
            );
        }
    }

    #[test]
    fn collection_failures_follow_the_f010_error_projection() {
        let unsupported = to_ipc_error(InventoryError::VersionUnsupported);
        assert_eq!(unsupported.code, ErrorCode::CapabilityDenied);
        assert_eq!(unsupported.reason_code, Some(ReasonCode::GitVersionUnsupported));
        assert_eq!(
            to_ipc_error(InventoryError::GitUnavailable).reason_code,
            Some(ReasonCode::GitVersionUnsupported)
        );

        let exhausted = to_ipc_error(InventoryError::DeadlineReached);
        assert_eq!(exhausted.code, ErrorCode::ResourceBusy);
        assert_eq!(exhausted.reason_code, Some(ReasonCode::LocalResourceExhausted));
        assert_eq!(exhausted.retry_class, RetryClass::AfterHumanDecision);
        assert_eq!(
            to_ipc_error(InventoryError::StreamLimitReached).reason_code,
            Some(ReasonCode::LocalResourceExhausted)
        );

        let unreadable = to_ipc_error(InventoryError::UnreadableRepository);
        assert_eq!(unreadable.code, ErrorCode::InvalidArgument);
        assert_eq!(unreadable.retry_class, RetryClass::Never);
        for failed in [InventoryError::GitFailed, InventoryError::UnexpectedOutput] {
            assert_eq!(
                to_ipc_error(failed).code,
                ErrorCode::InternalFailure
            );
        }
    }
}
