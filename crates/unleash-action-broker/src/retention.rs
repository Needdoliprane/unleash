// SPDX-License-Identifier: Apache-2.0
//! Rédaction multi-lignes/chunkée des canaris secrets et planification/exécution de purge
//! selon la politique `unleash.retention.default.v1` (`REQ-SEC-011`, `Q-12`, `storage-model.md`).

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use unleash_domain::{BackupId, TimestampMs};

/// Marqueur de remplacement utilisé après rédaction d'un secret.
pub const REDACTED_MARKER: &str = "[REDACTED]";

/// Résultat d'une passe de rédaction sur un texte de log, contexte ou rapport (`REQ-SEC-011`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionOutcome {
    /// Texte résultant après suppression des canaris et fragments détectés.
    pub redacted_text: String,
    /// Indique si au moins une rédaction a été appliquée.
    pub redaction_applied: bool,
    /// Nombre de zones supprimées.
    pub redacted_zones_count: usize,
    /// Fidélité exacte des octets (`false` dès qu'une rédaction a eu lieu, `security-boundaries.md`).
    pub is_byte_exact: bool,
}

/// Rédige les canaris secrets dans `input`, y compris lorsqu'ils sont découpés sur plusieurs
/// lignes (`\n`, `\r\n`, `\r`) ou en chunks adjacents (`REQ-SEC-011`, `T-F017-011`).
#[must_use]
pub fn redact_canaries(input: &str, canaries: &[&SecretString]) -> RedactionOutcome {
    let mut current = input.to_owned();
    let mut zones = 0_usize;

    for secret in canaries {
        let raw_secret = secret.expose_secret();
        if raw_secret.is_empty() {
            continue;
        }

        // 1. Correspondances exactes.
        while let Some(pos) = current.find(raw_secret) {
            let end = pos + raw_secret.len();
            current.replace_range(pos..end, REDACTED_MARKER);
            zones = zones.saturating_add(1);
        }

        // 2. Correspondances découpées sur retours à la ligne (`\n`, `\r`).
        let normalized_secret: String = raw_secret
            .chars()
            .filter(|c| *c != '\n' && *c != '\r')
            .collect();
        if normalized_secret.len() >= 4 {
            while let Some((raw_start, raw_end)) = find_multiline_span(&current, &normalized_secret)
            {
                current.replace_range(raw_start..raw_end, REDACTED_MARKER);
                zones = zones.saturating_add(1);
            }
        }
    }

    RedactionOutcome {
        redacted_text: current,
        redaction_applied: zones > 0,
        redacted_zones_count: zones,
        is_byte_exact: zones == 0,
    }
}

fn find_multiline_span(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let mut stripped = String::with_capacity(haystack.len());
    let mut byte_map: Vec<(usize, usize)> = Vec::with_capacity(haystack.len());

    for (idx, ch) in haystack.char_indices() {
        if ch == '\n' || ch == '\r' {
            continue;
        }
        let char_end = idx + ch.len_utf8();
        let start_in_stripped = stripped.len();
        stripped.push(ch);
        let end_in_stripped = stripped.len();
        for _ in start_in_stripped..end_in_stripped {
            byte_map.push((idx, char_end));
        }
    }

    let match_start = stripped.find(needle)?;
    let match_last_byte = match_start.checked_add(needle.len())?.checked_sub(1)?;
    let raw_start = byte_map.get(match_start)?.0;
    let raw_end = byte_map.get(match_last_byte)?.1;
    Some((raw_start, raw_end))
}

/// Classes de rétention définies par `unleash.retention.default.v1` (`Q-12`, `storage-model.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionClass {
    /// Journal métier (sans échéance automatique).
    BusinessJournal,
    /// Réponses complètes de commandes (7 jours puis tombstone).
    CommandResponse,
    /// Sessions durables et manifestes de contexte.
    SessionAndManifest,
    /// Payloads exacts des appels modèle (90 jours après mission terminale).
    ModelPayload,
    /// Sorties d'outils et logs de processus/terminaux (30 jours).
    ToolAndProcessLogs,
    /// Artefacts navigateur (30 jours).
    BrowserArtifacts,
    /// Captures dirty de worktree (30 jours).
    DirtyWorktreeCaptures,
    /// Données importées d'automatisation (90 jours).
    AutomationData,
    /// Copies locales de documents connectés (30 jours après retrait).
    ConnectedDocumentCopies,
    /// Rapports scellés (sans échéance automatique).
    SealedReports,
    /// Artefacts d'export préparés (7 jours).
    PreparedExports,
    /// Logs de diagnostic (14 jours).
    DiagnosticLogs,
    /// Cache reconstructible (LRU <= 2 GiB, purgeable immédiatement).
    Cache,
    /// Index dérivés reconstructibles.
    DerivedIndexes,
    /// Worktrees gérés (7 jours).
    ManagedWorktrees,
}

/// Politique configurable `unleash.retention.default.v1` (`Q-12`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicy {
    /// Identifiant de politique versionné (`"unleash.retention.default.v1"`).
    pub policy_id: String,
    /// Rétention des réponses de commande (défaut 7 jours).
    pub command_response_days: u32,
    /// Rétention des payloads modèles (défaut 90 jours).
    pub model_payload_retention_days: u32,
    /// Rétention des sorties d'outils/logs/navigateur/dirty (défaut 30 jours).
    pub tool_output_retention_days: u32,
    /// Rétention des logs de diagnostic (défaut 14 jours).
    pub diagnostic_log_retention_days: u32,
    /// Rétention des artefacts d'export (défaut 7 jours).
    pub export_artifact_retention_days: u32,
    /// Durée maximale de rotation des sauvegardes gérées (défaut 35 jours).
    pub backup_rotation_max_days: u32,
}

impl RetentionPolicy {
    /// Retourne la politique par défaut `unleash.retention.default.v1` (`Q-12`).
    #[must_use]
    pub fn unleash_default_v1() -> Self {
        Self {
            policy_id: String::from("unleash.retention.default.v1"),
            command_response_days: 7,
            model_payload_retention_days: 90,
            tool_output_retention_days: 30,
            diagnostic_log_retention_days: 14,
            export_artifact_retention_days: 7,
            backup_rotation_max_days: 35,
        }
    }

    fn ttl_days_for_class(&self, class: RetentionClass) -> Option<u32> {
        match class {
            RetentionClass::BusinessJournal
            | RetentionClass::SessionAndManifest
            | RetentionClass::SealedReports => None,
            RetentionClass::CommandResponse
            | RetentionClass::PreparedExports
            | RetentionClass::ManagedWorktrees => Some(
                self.command_response_days
                    .min(self.export_artifact_retention_days),
            ),
            RetentionClass::ModelPayload | RetentionClass::AutomationData => {
                Some(self.model_payload_retention_days)
            }
            RetentionClass::ToolAndProcessLogs
            | RetentionClass::BrowserArtifacts
            | RetentionClass::DirtyWorktreeCaptures
            | RetentionClass::ConnectedDocumentCopies => Some(self.tool_output_retention_days),
            RetentionClass::DiagnosticLogs => Some(self.diagnostic_log_retention_days),
            RetentionClass::Cache | RetentionClass::DerivedIndexes => Some(0),
        }
    }
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self::unleash_default_v1()
    }
}

/// État d'éligibilité à la purge d'un objet ou enregistrement (`Q-12`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionItemState {
    /// Référencé par un run/mission/invocation non terminal (protégé).
    ActiveNonTerminal,
    /// Associé à une action en `outcome_unknown` (protégé).
    OutcomeUnknown,
    /// Associé à un incident non résolu (protégé).
    UnresolvedIncident,
    /// Preuve exigée par un contrat actif (protégée).
    RequiredByActiveContract,
    /// Explicitement épinglé par politique ou utilisateur (protégé).
    Pinned,
    /// Propriétaire terminal depuis `terminal_at_ms`.
    TerminalEligible {
        /// Horodatage UTC de passage à l'état terminal.
        terminal_at_ms: TimestampMs,
    },
}

/// Élément candidat à l'évaluation de rétention/purge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionItem {
    /// Identifiant unique de l'objet ou enregistrement.
    pub id: String,
    /// Classe de rétention associée.
    pub class: RetentionClass,
    /// État courant de protection ou d'expiration.
    pub state: RetentionItemState,
    /// Taille en octets.
    pub byte_size: u64,
    /// Indique la présence connue de canaris secrets.
    pub contains_sensitive_canaries: bool,
}

/// Référence vers une sauvegarde gérée contenant potentiellement les octets purgés.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedBackupRef {
    /// Identifiant de la sauvegarde.
    pub backup_id: BackupId,
    /// Horodatage de création.
    pub created_at_ms: TimestampMs,
    /// Vrai si cette sauvegarde contient les copies antérieures aux purges.
    pub contains_purged_bytes: bool,
}

/// Tombstone durable conservé après purge d'un enregistrement sensible (`REQ-SEC-011`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeTombstone {
    /// Identifiant de l'objet purgé.
    pub item_id: String,
    /// Classe d'origine.
    pub class: RetentionClass,
    /// Horodatage UTC de purge.
    pub purged_at_ms: TimestampMs,
    /// Motif de la purge.
    pub reason: String,
}

/// Limite explicite de suppression physique signalée à l'utilisateur (`security-boundaries.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeLimitDisclosure {
    /// Portée non garantie (`apfs_time_machine`, `delivered_exports`, `remote_provider_copies`, `manual_backups`).
    pub scope: String,
    /// Explication claire de la limite physique ou tiers.
    pub reason: String,
}

/// Rapport complet produit par une purge contrôlée (`REQ-SEC-011`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeReport {
    /// Politique appliquée.
    pub policy_id: String,
    /// Identifiants des éléments purgés.
    pub purged_item_ids: Vec<String>,
    /// Identifiants préservés avec le motif de protection.
    pub preserved_item_ids: Vec<(String, String)>,
    /// Tombstones durables émis.
    pub tombstones: Vec<PurgeTombstone>,
    /// Nombre d'entrées de cache nettoyées.
    pub caches_cleared: usize,
    /// Indique l'obligation d'exécuter `PRAGMA wal_checkpoint(TRUNCATE)` sous `secure_delete=FAST`.
    pub wal_checkpoint_truncate_required: bool,
    /// Sauvegardes gérées contenant encore les octets purgés (jusqu'à rotation 35 jours).
    pub affected_managed_backups: Vec<BackupId>,
    /// Plafond de rotation des sauvegardes gérées en jours.
    pub backup_retention_max_days: u32,
    /// Limites explicites concernant snapshots système, exports remis et copies fournisseurs.
    pub explicit_limits: Vec<PurgeLimitDisclosure>,
}

/// Planifie et calcule une purge selon `policy` en protégeant tous les runs non terminaux,
/// `outcome_unknown`, incidents ouverts, preuves requises et pins (`Q-12`, `REQ-SEC-011`).
#[must_use]
pub fn plan_and_execute_purge(
    policy: &RetentionPolicy,
    items: &[RetentionItem],
    backups: &[ManagedBackupRef],
    now_ms: TimestampMs,
    reason: &str,
) -> PurgeReport {
    let mut purged_item_ids = Vec::new();
    let mut preserved_item_ids = Vec::new();
    let mut tombstones = Vec::new();
    let mut caches_cleared = 0_usize;

    for item in items {
        match item.state {
            RetentionItemState::ActiveNonTerminal => {
                preserved_item_ids.push((item.id.clone(), String::from("active_non_terminal")));
            }
            RetentionItemState::OutcomeUnknown => {
                preserved_item_ids.push((item.id.clone(), String::from("outcome_unknown")));
            }
            RetentionItemState::UnresolvedIncident => {
                preserved_item_ids.push((item.id.clone(), String::from("unresolved_incident")));
            }
            RetentionItemState::RequiredByActiveContract => {
                preserved_item_ids
                    .push((item.id.clone(), String::from("required_by_active_contract")));
            }
            RetentionItemState::Pinned => {
                preserved_item_ids.push((item.id.clone(), String::from("pinned")));
            }
            RetentionItemState::TerminalEligible { terminal_at_ms } => {
                let Some(ttl_days) = policy.ttl_days_for_class(item.class) else {
                    preserved_item_ids.push((item.id.clone(), String::from("no_automatic_expiry")));
                    continue;
                };
                let ttl_ms = i64::from(ttl_days).saturating_mul(86_400_000);
                let elapsed_ms = now_ms.as_i64().saturating_sub(terminal_at_ms.as_i64());
                if elapsed_ms >= ttl_ms {
                    purged_item_ids.push(item.id.clone());
                    if matches!(
                        item.class,
                        RetentionClass::Cache | RetentionClass::DerivedIndexes
                    ) {
                        caches_cleared = caches_cleared.saturating_add(1);
                    }
                    tombstones.push(PurgeTombstone {
                        item_id: item.id.clone(),
                        class: item.class,
                        purged_at_ms: now_ms,
                        reason: reason.to_owned(),
                    });
                } else {
                    preserved_item_ids
                        .push((item.id.clone(), String::from("within_retention_window")));
                }
            }
        }
    }

    let affected_managed_backups: Vec<BackupId> = backups
        .iter()
        .filter(|b| b.contains_purged_bytes)
        .map(|b| b.backup_id)
        .collect();

    let explicit_limits = vec![
        PurgeLimitDisclosure {
            scope: String::from("apfs_time_machine"),
            reason: String::from(
                "Unleash ne supprime ni les snapshots APFS/filesystem ni les sauvegardes Time Machine",
            ),
        },
        PurgeLimitDisclosure {
            scope: String::from("delivered_exports"),
            reason: String::from(
                "Les bundles ou rapports déjà exportés hors stockage géré restent sous responsabilité utilisateur",
            ),
        },
        PurgeLimitDisclosure {
            scope: String::from("remote_provider_copies"),
            reason: String::from(
                "Les copies conservées côté fournisseurs de modèles ou connecteurs externes échappent à la purge locale",
            ),
        },
        PurgeLimitDisclosure {
            scope: String::from("manual_backups"),
            reason: String::from(
                "Les sauvegardes manuelles ou externes non supprimées explicitement conservent les octets antérieurs",
            ),
        },
    ];

    let wal_checkpoint_truncate_required = !tombstones.is_empty();
    PurgeReport {
        policy_id: policy.policy_id.clone(),
        purged_item_ids,
        preserved_item_ids,
        tombstones,
        caches_cleared,
        wal_checkpoint_truncate_required,
        affected_managed_backups,
        backup_retention_max_days: policy.backup_rotation_max_days,
        explicit_limits,
    }
}
