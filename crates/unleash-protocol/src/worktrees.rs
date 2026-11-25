// SPDX-License-Identifier: Apache-2.0
//! Inventaire en lecture des worktrees d'un dépôt rattaché : charge utile fermée de
//! `ListWorktrees`, résultat fermé et contrôle de concordance avec le contrat.
//!
//! Les valeurs décrivent des faits observés par le daemon (`REQ-GIT-001`, `AC-F010-001`). Aucune
//! ne vaut décision d'allocation, preuve de propriété ni autorisation de mutation : une
//! observation qui ne peut être établie reste `unknown` ou `incomplete` et ne devient jamais
//! « propre » ni « libre ».

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unleash_domain::{ProjectId, Sha256Digest};

/// Taille de page retenue quand la requête omet `page_size`.
pub const DEFAULT_WORKTREE_PAGE_SIZE: u16 = 50;
/// Plus grande page admise par `ipc-query.schema.json`.
pub const MAX_WORKTREE_PAGE_SIZE: u16 = 100;
/// Plus long curseur opaque admis par `ipc-query.schema.json`.
pub const MAX_WORKTREE_CURSOR_CHARS: usize = 512;

const MAX_OBSERVED_OPERATIONS: usize = 4;
const MAX_RESERVED_BRANCHES: usize = 2;
const BRANCH_REF_PREFIX: &str = "refs/heads/";
const SHA1_HEX_LEN: usize = 40;
const SHA256_HEX_LEN: usize = 64;

/// Rejet d'une requête `ListWorktrees` mal formée, avant toute observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WorktreeQueryError {
    /// `repo_id` ne contient aucun caractère non blanc.
    #[error("repo_id doit contenir au moins un caractère non blanc")]
    BlankRepoId,
    /// `page_size` sort de l'intervalle 1 à 100.
    #[error("page_size doit être compris entre 1 et 100")]
    PageSizeOutOfRange,
    /// Le curseur ne se limite pas aux caractères et à la longueur admis.
    #[error("curseur opaque mal formé : 1 à 512 caractères alphanumériques, tirets ou tirets bas")]
    MalformedCursor,
}

/// Écart d'un résultat `ListWorktrees` avec la requête ou le contrat fermé.
///
/// Un tel écart est un défaut du daemon : il n'est jamais restitué comme un inventaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WorktreeContractViolation {
    /// `project_id` du résultat diffère du projet demandé.
    #[error("project_id du résultat diffère du projet demandé")]
    ProjectMismatch,
    /// `repo_id` du résultat diffère du dépôt demandé.
    #[error("repo_id du résultat diffère du dépôt demandé")]
    RepoMismatch,
    /// La page porte plus d'éléments que `page_size`.
    #[error("la page porte plus d'éléments que page_size")]
    PageOverflow,
    /// Un chemin d'élément est vide.
    #[error("un chemin de worktree est vide")]
    EmptyPath,
    /// Les éléments ne sont pas triés par les octets de `path`, ou un chemin est répété.
    #[error("les éléments ne sont pas triés par chemin, sans doublon")]
    UnsortedPaths,
    /// Un worktree n'a ni `head_oid` ni `branch_ref`.
    #[error("un worktree n'a ni head_oid ni branch_ref")]
    MissingIdentity,
    /// Un `head_oid` n'utilise pas le `object_format` observé.
    #[error("head_oid n'utilise pas le format d'objet observé")]
    ObjectFormatMismatch,
    /// Un `branch_ref` n'est pas un nom complet de branche.
    #[error("branch_ref n'est pas un nom complet de branche")]
    InvalidBranchRef,
    /// Une opération est répétée ou hors des quatre valeurs du contrat.
    #[error("une opération est répétée ou hors du contrat")]
    InvalidOperations,
    /// Une association branche/opération est répétée ou contredit les opérations observées.
    #[error("une association branche/opération contredit les opérations observées")]
    InconsistentOperationBranch,
    /// La couverture est `complete` alors qu'une observation est incertaine.
    #[error("la couverture est complete alors qu'une observation est incertaine")]
    CoverageTooOptimistic,
    /// `next_cursor` n'est pas `null` alors que la page est vide.
    #[error("next_cursor doit être null quand la page est vide")]
    UnexpectedCursor,
}

/// Charge utile fermée de la requête `ListWorktrees`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListWorktreesPayload {
    /// Identifiant du dépôt parmi ceux rattachés au projet.
    pub repo_id: String,
    /// Taille de page demandée, de 1 à 100 ; absente pour la valeur par défaut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u16>,
    /// Curseur opaque de la collecte lue ; absent ou `null` ouvre la lecture.
    #[serde(default)]
    pub opaque_cursor: Option<String>,
}

impl ListWorktreesPayload {
    /// Taille de page effective : celle de la requête, sinon 50.
    #[must_use]
    pub const fn effective_page_size(&self) -> u16 {
        match self.page_size {
            Some(size) => size,
            None => DEFAULT_WORKTREE_PAGE_SIZE,
        }
    }

    /// Contrôle la requête selon F010/interfaces : `repo_id` non blanc, `page_size` de 1 à 100
    /// et curseur de 1 à 512 caractères ASCII alphanumériques, tirets ou tirets bas.
    ///
    /// # Errors
    /// Retourne [`WorktreeQueryError`] dès qu'une règle n'est pas satisfaite.
    pub fn validate(&self) -> Result<(), WorktreeQueryError> {
        if self.repo_id.trim().is_empty() {
            return Err(WorktreeQueryError::BlankRepoId);
        }
        if let Some(size) = self.page_size
            && !(1..=MAX_WORKTREE_PAGE_SIZE).contains(&size)
        {
            return Err(WorktreeQueryError::PageSizeOutOfRange);
        }
        if let Some(cursor) = self.opaque_cursor.as_deref()
            && !is_well_formed_cursor(cursor)
        {
            return Err(WorktreeQueryError::MalformedCursor);
        }
        Ok(())
    }
}

/// Indique si un curseur respecte le motif `run_page_cursor` du schéma des requêtes.
#[must_use]
pub fn is_well_formed_cursor(cursor: &str) -> bool {
    !cursor.is_empty()
        && cursor.len() <= MAX_WORKTREE_CURSOR_CHARS
        && cursor
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Format d'objet d'un dépôt Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitObjectFormat {
    /// Objets adressés par SHA-1.
    Sha1,
    /// Objets adressés par SHA-256.
    Sha256,
}

impl GitObjectFormat {
    /// Longueur, en caractères hexadécimaux, d'un identifiant dans ce format.
    #[must_use]
    pub const fn hex_len(self) -> usize {
        match self {
            Self::Sha1 => SHA1_HEX_LEN,
            Self::Sha256 => SHA256_HEX_LEN,
        }
    }
}

/// Identifiant d'objet Git observé : un couple format et hexadécimal minuscule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitObjectId {
    /// Format d'objet du dépôt.
    pub format: GitObjectFormat,
    /// Empreinte hexadécimale minuscule, de la longueur du format.
    pub hex: String,
}

impl GitObjectId {
    /// Construit un identifiant si `hex` a la longueur du format et n'utilise que `0-9a-f`.
    ///
    /// # Errors
    /// Retourne [`WorktreeContractViolation::ObjectFormatMismatch`] sinon.
    pub fn parse(format: GitObjectFormat, hex: &str) -> Result<Self, WorktreeContractViolation> {
        let well_formed = hex.len() == format.hex_len()
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if well_formed {
            Ok(Self {
                format,
                hex: hex.to_owned(),
            })
        } else {
            Err(WorktreeContractViolation::ObjectFormatMismatch)
        }
    }

    /// Indique si l'empreinte est l'identifiant nul, celui que Git rend pour une branche unborn.
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.hex.bytes().all(|byte| byte == b'0')
    }
}

/// État de propreté observé d'un worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeDirty {
    /// Aucun changement engagé, non engagé ou en conflit, et aucun fichier non suivi, n'a été observé.
    Clean,
    /// Au moins un changement ou un fichier non suivi a été observé.
    Dirty,
    /// L'état n'a pas pu être établi.
    Unknown,
}

/// Opération Git observée en cours sur un worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperation {
    /// Fusion en cours.
    Merge,
    /// Rebase en cours.
    Rebase,
    /// Bissection en cours.
    Bisect,
    /// Cherry-pick en cours.
    CherryPick,
}

/// Opération qui réserve une branche et dont la provenance est exposée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservedBranchOperation {
    /// Rebase en cours.
    Rebase,
    /// Bissection en cours.
    Bisect,
}

impl ReservedBranchOperation {
    /// Opération correspondante dans `operations`.
    #[must_use]
    pub const fn observed(self) -> GitOperation {
        match self {
            Self::Rebase => GitOperation::Rebase,
            Self::Bisect => GitOperation::Bisect,
        }
    }
}

/// Métadonnées Git à l'origine d'une association branche/opération.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationBranchProvenance {
    /// Lues dans les métadonnées de rebase du worktree.
    RebaseMetadata,
    /// Lues dans les métadonnées de bissection du worktree.
    BisectMetadata,
}

/// Branche réservée par un rebase ou un bisect, avec sa provenance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationBranch {
    /// Opération qui réserve la branche.
    pub operation: ReservedBranchOperation,
    /// Nom complet de la branche réservée.
    pub branch_ref: String,
    /// Métadonnées Git où l'association a été observée.
    pub provenance: OperationBranchProvenance,
}

/// Propriété observée d'un worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeOwnership {
    /// Alloué par Unleash et inscrit à son registre des allocations.
    Managed,
    /// Hors du registre des allocations d'Unleash.
    External,
    /// Le registre des allocations n'a pas pu être lu.
    Unknown,
}

/// Worktree observé d'un dépôt rattaché.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeItem {
    /// Localisation observée ; ce n'est ni une identité durable ni une preuve de propriété.
    pub path: String,
    /// Objet pointé par `HEAD`, ou `None` pour une branche unborn.
    pub head_oid: Option<GitObjectId>,
    /// Branche attachée, ou `None` pour un `HEAD` détaché.
    pub branch_ref: Option<String>,
    /// État de propreté observé.
    pub dirty: WorktreeDirty,
    /// Opérations Git observées.
    pub operations: Vec<GitOperation>,
    /// `false` quand une opération en cours n'a pas pu être lue ou représentée.
    pub operations_complete: bool,
    /// Branches réservées par un rebase ou un bisect.
    pub operation_branches: Vec<OperationBranch>,
    /// `false` quand une association branche/opération n'a pas pu être établie.
    pub operation_branches_complete: bool,
    /// Propriété observée.
    pub ownership: WorktreeOwnership,
}

impl WorktreeItem {
    /// Indique si une observation de ce worktree reste incertaine.
    ///
    /// Le contrat exige alors une couverture `incomplete` de toute la collecte.
    #[must_use]
    pub fn is_uncertain(&self) -> bool {
        self.dirty == WorktreeDirty::Unknown
            || !self.operations_complete
            || !self.operation_branches_complete
    }

    /// Contrôle un élément selon le contrat fermé des items de `ListWorktrees`.
    ///
    /// # Errors
    /// Retourne le premier [`WorktreeContractViolation`] observé.
    pub fn check_contract(
        &self,
        object_format: GitObjectFormat,
    ) -> Result<(), WorktreeContractViolation> {
        use WorktreeContractViolation::{
            InconsistentOperationBranch, InvalidBranchRef, InvalidOperations, MissingIdentity,
            ObjectFormatMismatch,
        };
        if self.head_oid.is_none() && self.branch_ref.is_none() {
            return Err(MissingIdentity);
        }
        if let Some(oid) = &self.head_oid
            && oid.format != object_format
        {
            return Err(ObjectFormatMismatch);
        }
        if let Some(branch) = self.branch_ref.as_deref()
            && !is_worktree_branch_ref(branch)
        {
            return Err(InvalidBranchRef);
        }
        if self.operations.len() > MAX_OBSERVED_OPERATIONS || has_repetition(&self.operations) {
            return Err(InvalidOperations);
        }
        if self.operation_branches.len() > MAX_RESERVED_BRANCHES
            || has_repetition(&self.operation_branches)
        {
            return Err(InconsistentOperationBranch);
        }
        let associations_known = self.operation_branches.iter().all(|association| {
            matches!(
                (association.operation, association.provenance),
                (
                    ReservedBranchOperation::Rebase,
                    OperationBranchProvenance::RebaseMetadata
                ) | (
                    ReservedBranchOperation::Bisect,
                    OperationBranchProvenance::BisectMetadata
                )
            ) && is_worktree_branch_ref(&association.branch_ref)
                && self.operations.contains(&association.operation.observed())
        });
        if !associations_known {
            return Err(InconsistentOperationBranch);
        }
        let reservations_known = [
            (GitOperation::Rebase, ReservedBranchOperation::Rebase),
            (GitOperation::Bisect, ReservedBranchOperation::Bisect),
        ]
        .into_iter()
        .filter(|(operation, _)| self.operations.contains(operation))
        .all(|(_, reserved)| {
            !self.operation_branches_complete
                || self
                    .operation_branches
                    .iter()
                    .any(|association| association.operation == reserved)
        });
        if !reservations_known {
            return Err(InconsistentOperationBranch);
        }
        Ok(())
    }
}

fn has_repetition<T: PartialEq>(values: &[T]) -> bool {
    values.iter().enumerate().any(|(index, value)| {
        values
            .iter()
            .skip(index + 1)
            .any(|later| *later == *value)
    })
}

/// Couverture d'une collecte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryCoverageStatus {
    /// Chaque worktree a été observé sans incertitude et aucun n'a été omis.
    Complete,
    /// Au moins une observation est incertaine ou un worktree a été omis.
    Incomplete,
}

/// Couverture de la collecte entière, pas seulement de la page restituée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryCoverage {
    /// `complete` ou `incomplete`.
    pub status: InventoryCoverageStatus,
}

/// Résultat fermé de `ListWorktrees`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListWorktreesResult {
    /// Projet demandé et autorisé.
    pub project_id: ProjectId,
    /// Dépôt demandé et autorisé.
    pub repo_id: String,
    /// Format d'objet observé du dépôt.
    pub object_format: GitObjectFormat,
    /// Empreinte du profil Git effectif de la collecte.
    pub git_profile_digest: Sha256Digest,
    /// Date de la collecte.
    pub observed_at: DateTime<Utc>,
    /// Worktrees de la page, triés par les octets de `path`.
    pub items: Vec<WorktreeItem>,
    /// Couverture de la collecte entière.
    pub coverage: InventoryCoverage,
    /// Curseur de la page suivante, ou `None` sur la dernière page.
    pub next_cursor: Option<String>,
}

impl ListWorktreesResult {
    /// Couverture d'une collecte dont aucun worktree n'a été omis.
    #[must_use]
    pub fn coverage_of(items: &[WorktreeItem]) -> InventoryCoverage {
        let status = if items.iter().any(WorktreeItem::is_uncertain) {
            InventoryCoverageStatus::Incomplete
        } else {
            InventoryCoverageStatus::Complete
        };
        InventoryCoverage { status }
    }

    /// Contrôle la concordance du résultat avec la requête et le contrat fermé : références
    /// demandées, taille et tri de la page, identité, format et opérations de chaque élément,
    /// et couverture jamais optimiste.
    ///
    /// # Errors
    /// Retourne le premier [`WorktreeContractViolation`] observé.
    pub fn check_contract(
        &self,
        project: ProjectId,
        repo_id: &str,
        page_size: u16,
    ) -> Result<(), WorktreeContractViolation> {
        if self.project_id != project {
            return Err(WorktreeContractViolation::ProjectMismatch);
        }
        if self.repo_id != repo_id {
            return Err(WorktreeContractViolation::RepoMismatch);
        }
        let limit = usize::from(page_size.min(MAX_WORKTREE_PAGE_SIZE));
        if self.items.len() > limit {
            return Err(WorktreeContractViolation::PageOverflow);
        }
        if self.items.iter().any(|item| item.path.is_empty()) {
            return Err(WorktreeContractViolation::EmptyPath);
        }
        let unsorted = self
            .items
            .iter()
            .zip(self.items.iter().skip(1))
            .any(|(earlier, later)| earlier.path.as_bytes() >= later.path.as_bytes());
        if unsorted {
            return Err(WorktreeContractViolation::UnsortedPaths);
        }
        for item in &self.items {
            item.check_contract(self.object_format)?;
        }
        let optimistic = self.coverage.status == InventoryCoverageStatus::Complete
            && self.items.iter().any(WorktreeItem::is_uncertain);
        if optimistic {
            return Err(WorktreeContractViolation::CoverageTooOptimistic);
        }
        if self.items.is_empty() && self.next_cursor.is_some() {
            return Err(WorktreeContractViolation::UnexpectedCursor);
        }
        Ok(())
    }
}

/// Indique si un nom est un `worktree_branch_ref` du schéma : `refs/heads/<nom>`, sans caractère
/// de contrôle ni séparateur de ligne.
#[must_use]
pub fn is_worktree_branch_ref(name: &str) -> bool {
    name.strip_prefix(BRANCH_REF_PREFIX).is_some_and(|rest| {
        !rest.is_empty()
            && !rest.chars().any(|character| {
                character.is_control() || matches!(character, '\u{2028}' | '\u{2029}')
            })
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
    use serde_json::{Value, json};
    use unleash_protocol::from_json_str_strict;

    use super::*;

    const PROJECT: u128 = 0x0000_0000_0000_0010;
    const HEAD: &str = "2222222222222222222222222222222222222222";

    fn digest() -> Sha256Digest {
        Sha256Digest::parse_hex(&"1".repeat(64)).unwrap()
    }

    fn item(path: &str) -> WorktreeItem {
        WorktreeItem {
            path: path.to_owned(),
            head_oid: Some(GitObjectId::parse(GitObjectFormat::Sha1, HEAD).unwrap()),
            branch_ref: Some(String::from("refs/heads/main")),
            dirty: WorktreeDirty::Dirty,
            operations: Vec::new(),
            operations_complete: true,
            operation_branches: Vec::new(),
            operation_branches_complete: true,
            ownership: WorktreeOwnership::External,
        }
    }

    fn result(items: Vec<WorktreeItem>) -> ListWorktreesResult {
        let coverage = ListWorktreesResult::coverage_of(&items);
        ListWorktreesResult {
            project_id: ProjectId::from_u128(PROJECT),
            repo_id: String::from("repo-01"),
            object_format: GitObjectFormat::Sha1,
            git_profile_digest: digest(),
            observed_at: DateTime::parse_from_rfc3339("2026-11-24T12:00:00Z")
                .unwrap()
                .to_utc(),
            items,
            coverage,
            next_cursor: None,
        }
    }

    fn reserved(operation: ReservedBranchOperation, branch: &str) -> OperationBranch {
        OperationBranch {
            operation,
            branch_ref: branch.to_owned(),
            provenance: match operation {
                ReservedBranchOperation::Rebase => OperationBranchProvenance::RebaseMetadata,
                ReservedBranchOperation::Bisect => OperationBranchProvenance::BisectMetadata,
            },
        }
    }

    // REQ-CORE-005 : la requête et la réponse de référence restent identiques après un aller-retour strict.
    #[test]
    fn query_and_response_fixtures_round_trip_through_the_closed_dtos() {
        let query: Value = serde_json::from_str(include_str!(
            "../../../.claude/examples/ipc-list-worktrees-query.json"
        ))
        .unwrap();
        let payload: ListWorktreesPayload =
            from_json_str_strict(query["payload"].to_string()).unwrap();
        assert_eq!(
            payload,
            ListWorktreesPayload {
                repo_id: String::from("repo-01"),
                page_size: Some(50),
                opaque_cursor: None,
            }
        );
        let serialized = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            serialized,
            json!({"repo_id": "repo-01", "page_size": 50, "opaque_cursor": null})
        );
        payload.validate().unwrap();
        assert_eq!(payload.effective_page_size(), 50);

        let response: Value = serde_json::from_str(include_str!(
            "../../../.claude/examples/ipc-list-worktrees-response.json"
        ))
        .unwrap();
        let expected_result = response.get("result").cloned().unwrap();
        let decoded: ListWorktreesResult =
            from_json_str_strict(expected_result.to_string()).unwrap();
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items.first().unwrap().dirty, WorktreeDirty::Dirty);
        assert_eq!(decoded.coverage.status, InventoryCoverageStatus::Complete);
        assert_eq!(decoded.git_profile_digest, digest());
        decoded
            .check_contract(decoded.project_id, "repo-01", 50)
            .unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), expected_result);

        let without_size: ListWorktreesPayload =
            from_json_str_strict(r#"{"repo_id":"r"}"#).unwrap();
        assert_eq!(
            without_size.effective_page_size(),
            DEFAULT_WORKTREE_PAGE_SIZE
        );
        assert_eq!(
            serde_json::to_value(&without_size).unwrap(),
            json!({"repo_id": "r", "opaque_cursor": null})
        );
        for rejected in [
            json!({"repo_id": "r", "page_size": null}),
            json!({"repo_id": "r", "root_path": "/tmp/r"}),
            json!({"page_size": 5}),
        ] {
            let candidate = serde_json::from_value::<ListWorktreesPayload>(rejected);
            assert!(candidate.is_err(), "charge utile acceptée : {rejected}");
        }
    }

    #[test]
    fn query_validation_rejects_blank_repository_bad_sizes_and_foreign_cursors() {
        let with = |repo_id: &str, page_size: Option<u16>, opaque_cursor: Option<&str>| {
            ListWorktreesPayload {
                repo_id: repo_id.to_owned(),
                page_size,
                opaque_cursor: opaque_cursor.map(ToOwned::to_owned),
            }
        };
        assert_eq!(
            with("  ", None, None).validate(),
            Err(WorktreeQueryError::BlankRepoId)
        );
        assert_eq!(
            with("r", Some(0), None).validate(),
            Err(WorktreeQueryError::PageSizeOutOfRange)
        );
        assert_eq!(
            with("r", Some(101), None).validate(),
            Err(WorktreeQueryError::PageSizeOutOfRange)
        );
        assert_eq!(
            with("r", None, Some("")).validate(),
            Err(WorktreeQueryError::MalformedCursor)
        );
        assert_eq!(
            with("r", None, Some("a+b")).validate(),
            Err(WorktreeQueryError::MalformedCursor)
        );
        let too_long = "a".repeat(513);
        assert_eq!(
            with("r", None, Some(too_long.as_str())).validate(),
            Err(WorktreeQueryError::MalformedCursor)
        );
        let longest = "Az_09-".repeat(85);
        let longest = longest.get(..512).unwrap();
        with("r", Some(100), Some(longest)).validate().unwrap();
        with("r", Some(1), None).validate().unwrap();
        assert!(is_well_formed_cursor(&"a".repeat(512)));
    }

    #[test]
    fn object_identifiers_follow_the_observed_format_and_expose_the_null_id() {
        let oid = GitObjectId::parse(GitObjectFormat::Sha1, HEAD).unwrap();
        assert!(!oid.is_null());
        assert_eq!(GitObjectFormat::Sha256.hex_len(), 64);
        assert_eq!(
            GitObjectId::parse(GitObjectFormat::Sha1, HEAD.get(..39).unwrap()),
            Err(WorktreeContractViolation::ObjectFormatMismatch)
        );
        assert_eq!(
            GitObjectId::parse(GitObjectFormat::Sha1, &HEAD.to_uppercase()),
            Err(WorktreeContractViolation::ObjectFormatMismatch)
        );
        assert_eq!(
            GitObjectId::parse(GitObjectFormat::Sha256, HEAD),
            Err(WorktreeContractViolation::ObjectFormatMismatch)
        );
        let unborn = GitObjectId::parse(GitObjectFormat::Sha1, &"0".repeat(40)).unwrap();
        assert!(unborn.is_null());
    }

    #[test]
    fn branch_refs_follow_the_closed_schema_pattern() {
        assert!(is_worktree_branch_ref("refs/heads/main"));
        assert!(is_worktree_branch_ref("refs/heads/feature/ové"));
        for refused in [
            "refs/heads/",
            "main",
            "refs/remotes/origin/main",
            "refs/heads/a\u{7f}b",
            "refs/heads/a\u{2028}b",
        ] {
            assert!(!is_worktree_branch_ref(refused), "accepté : {refused:?}");
        }
    }

    #[test]
    fn result_contract_refuses_every_drift_from_the_request() {
        let project = ProjectId::from_u128(PROJECT);
        let reversed = result(vec![item("/b"), item("/a")]);
        assert_eq!(
            reversed.check_contract(project, "repo-01", 50),
            Err(WorktreeContractViolation::UnsortedPaths)
        );

        let mut paths = reversed.items.clone();
        paths.swap(0, 1);
        let sorted = result(paths);
        sorted.check_contract(project, "repo-01", 50).unwrap();

        let mut other_repository = sorted.clone();
        other_repository.repo_id = String::from("repo-02");
        assert_eq!(
            other_repository.check_contract(project, "repo-02", 50),
            Err(WorktreeContractViolation::ProjectMismatch)
        );
        let other_project = ProjectId::from_u128(PROJECT + 1);
        assert_eq!(
            sorted.check_contract(other_project, "repo-01", 50),
            Err(WorktreeContractViolation::ProjectMismatch)
        );
        assert_eq!(
            sorted.check_contract(project, "repo-02", 50),
            Err(WorktreeContractViolation::RepoMismatch)
        );
        assert_eq!(
            sorted.check_contract(project, "repo-01", 1),
            Err(WorktreeContractViolation::PageOverflow)
        );
        sorted.check_contract(project, "repo-01", 100).unwrap();

        let mut repeated_path = result(vec![item("/a"), item("/a")]);
        repeated_path.coverage.status = InventoryCoverageStatus::Complete;
        assert_eq!(
            repeated_path.check_contract(project, "repo-01", 50),
            Err(WorktreeContractViolation::UnsortedPaths)
        );

        let mut blank_path = result(vec![item("")]);
        blank_path.coverage.status = InventoryCoverageStatus::Complete;
        assert_eq!(
            blank_path.check_contract(project, "repo-01", 50),
            Err(WorktreeContractViolation::EmptyPath)
        );
    }

    #[test]
    fn item_contract_refuses_identity_format_and_operation_drifts() {
        let sha1 = GitObjectFormat::Sha1;
        item("/a").check_contract(sha1).unwrap();

        let mut detached = item("/a");
        detached.branch_ref = None;
        detached.check_contract(sha1).unwrap();

        let mut unborn = item("/a");
        unborn.head_oid = None;
        unborn.check_contract(sha1).unwrap();
        assert_eq!(
            unborn.check_contract(GitObjectFormat::Sha256),
            Err(WorktreeContractViolation::ObjectFormatMismatch)
        );

        let mut without_identity = item("/a");
        without_identity.head_oid = None;
        without_identity.branch_ref = None;
        assert_eq!(
            without_identity.check_contract(sha1),
            Err(WorktreeContractViolation::MissingIdentity)
        );

        let mut short_branch = item("/a");
        short_branch.branch_ref = Some(String::from("main"));
        assert_eq!(
            short_branch.check_contract(sha1),
            Err(WorktreeContractViolation::InvalidBranchRef)
        );

        let mut repeated_operation = item("/a");
        repeated_operation.operations = vec![GitOperation::Merge, GitOperation::Merge];
        assert_eq!(
            repeated_operation.check_contract(sha1),
            Err(WorktreeContractViolation::InvalidOperations)
        );

        let mut wrong_provenance = item("/a");
        wrong_provenance.operations = vec![GitOperation::Rebase];
        wrong_provenance.operation_branches = vec![reserved(
            ReservedBranchOperation::Rebase,
            "refs/heads/topic",
        )];
        wrong_provenance
            .operation_branches
            .first_mut()
            .unwrap()
            .provenance = OperationBranchProvenance::BisectMetadata;
        assert_eq!(
            wrong_provenance.check_contract(sha1),
            Err(WorktreeContractViolation::InconsistentOperationBranch)
        );

        let mut unobserved_reservation = item("/a");
        unobserved_reservation.operation_branches = vec![reserved(
            ReservedBranchOperation::Bisect,
            "refs/heads/topic",
        )];
        assert_eq!(
            unobserved_reservation.check_contract(sha1),
            Err(WorktreeContractViolation::InconsistentOperationBranch)
        );

        let mut repeated_reservation = item("/a");
        repeated_reservation.operations = vec![GitOperation::Rebase];
        repeated_reservation.operation_branches = vec![
            reserved(ReservedBranchOperation::Rebase, "refs/heads/topic"),
            reserved(ReservedBranchOperation::Rebase, "refs/heads/topic"),
        ];
        assert_eq!(
            repeated_reservation.check_contract(sha1),
            Err(WorktreeContractViolation::InconsistentOperationBranch)
        );

        let mut reservation_without_branch = item("/a");
        reservation_without_branch.operations = vec![GitOperation::Rebase];
        assert_eq!(
            reservation_without_branch.check_contract(sha1),
            Err(WorktreeContractViolation::InconsistentOperationBranch)
        );
        reservation_without_branch.operation_branches_complete = false;
        reservation_without_branch.check_contract(sha1).unwrap();

        let mut rebasing = item("/a");
        rebasing.operations = vec![GitOperation::Rebase, GitOperation::Bisect];
        rebasing.operation_branches = vec![
            reserved(ReservedBranchOperation::Rebase, "refs/heads/topic"),
            reserved(ReservedBranchOperation::Bisect, "refs/heads/main"),
        ];
        rebasing.check_contract(sha1).unwrap();
    }

    #[test]
    fn coverage_never_claims_completeness_over_an_uncertain_observation() {
        let project = ProjectId::from_u128(PROJECT);
        let mut unknown_dirty = item("/a");
        unknown_dirty.dirty = WorktreeDirty::Unknown;
        let collection = result(vec![unknown_dirty]);
        assert_eq!(
            collection.coverage.status,
            InventoryCoverageStatus::Incomplete
        );
        collection.check_contract(project, "repo-01", 50).unwrap();

        let mut optimistic = collection.clone();
        optimistic.coverage.status = InventoryCoverageStatus::Complete;
        assert_eq!(
            optimistic.check_contract(project, "repo-01", 50),
            Err(WorktreeContractViolation::CoverageTooOptimistic)
        );

        let mut unreadable_operations = item("/a");
        unreadable_operations.operations_complete = false;
        assert_eq!(
            ListWorktreesResult::coverage_of(std::slice::from_ref(&unreadable_operations)).status,
            InventoryCoverageStatus::Incomplete
        );
        let mut unreadable_associations = item("/a");
        unreadable_associations.operation_branches_complete = false;
        assert_eq!(
            ListWorktreesResult::coverage_of(std::slice::from_ref(&unreadable_associations)).status,
            InventoryCoverageStatus::Incomplete
        );

        let empty = result(Vec::new());
        assert_eq!(empty.coverage.status, InventoryCoverageStatus::Complete);
        assert!(empty.next_cursor.is_none());

        let mut cursor_on_empty = result(Vec::new());
        cursor_on_empty.next_cursor = Some(String::from("c"));
        assert_eq!(
            cursor_on_empty.check_contract(project, "repo-01", 50),
            Err(WorktreeContractViolation::UnexpectedCursor)
        );

        let mut cautious = result(vec![item("/a")]);
        cautious.coverage.status = InventoryCoverageStatus::Incomplete;
        cautious.check_contract(project, "repo-01", 50).unwrap();
    }
}
