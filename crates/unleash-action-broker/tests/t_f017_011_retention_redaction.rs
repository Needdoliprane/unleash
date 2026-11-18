// T-F017-011 / AC-F017-011 / REQ-SEC-011
//! Tests de rédaction multi-lignes/chunkée des canaris et de rétention/purge `unleash.retention.default.v1`
//! (tombstones, protection des runs non terminaux / `outcome_unknown` / pins, limites WAL/cache/backups/tiers).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use secrecy::SecretString;
use unleash_action_broker::{
    ManagedBackupRef, RetentionClass, RetentionItem, RetentionItemState, RetentionPolicy,
    plan_and_execute_purge, redact_canaries,
};
use unleash_domain::{BackupId, TimestampMs};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
fn t_f017_011_multiline_canary_redaction_and_retention_purge_limits() {
    let canary_a = SecretString::from("sk-canary-alpha-1234567890".to_owned());
    let canary_b = SecretString::from("oauth-refresh-beta-abcdef".to_owned());

    let raw_log = String::from(
        "header\nAuthorization: Bearer sk-canary-alpha-1234567890\nchunk1: oauth-refresh-\nbeta-abcdef\nfooter",
    );
    let outcome = redact_canaries(&raw_log, &[&canary_a, &canary_b]);
    assert!(outcome.redaction_applied);
    assert!(outcome.redacted_zones_count >= 2);
    assert!(!outcome.is_byte_exact);
    assert!(!outcome.redacted_text.contains("sk-canary-alpha-1234567890"));
    assert!(!outcome.redacted_text.contains("oauth-refresh-beta-abcdef"));
    assert!(!outcome.redacted_text.contains("oauth-refresh-"));
    assert!(outcome.redacted_text.contains("[REDACTED]"));

    let policy = RetentionPolicy::unleash_default_v1();
    assert_eq!(policy.policy_id, "unleash.retention.default.v1");
    assert_eq!(policy.model_payload_retention_days, 90);
    assert_eq!(policy.tool_output_retention_days, 30);
    assert_eq!(policy.diagnostic_log_retention_days, 14);
    assert_eq!(policy.backup_rotation_max_days, 35);

    let now = TimestampMs::new(1_767_225_600_000 + 40 * 86_400_000);
    let expired_ts = TimestampMs::new(1_767_225_600_000); // > 30 jours et > 14 jours.

    let items = vec![
        RetentionItem {
            id: String::from("log-expired-1"),
            class: RetentionClass::ToolAndProcessLogs,
            state: RetentionItemState::TerminalEligible {
                terminal_at_ms: expired_ts,
            },
            byte_size: 4096,
            contains_sensitive_canaries: true,
        },
        RetentionItem {
            id: String::from("cache-entry-1"),
            class: RetentionClass::Cache,
            state: RetentionItemState::TerminalEligible {
                terminal_at_ms: expired_ts,
            },
            byte_size: 2048,
            contains_sensitive_canaries: false,
        },
        RetentionItem {
            id: String::from("active-run-log"),
            class: RetentionClass::ToolAndProcessLogs,
            state: RetentionItemState::ActiveNonTerminal,
            byte_size: 1024,
            contains_sensitive_canaries: true,
        },
        RetentionItem {
            id: String::from("unknown-effect-payload"),
            class: RetentionClass::ModelPayload,
            state: RetentionItemState::OutcomeUnknown,
            byte_size: 1024,
            contains_sensitive_canaries: true,
        },
        RetentionItem {
            id: String::from("pinned-evidence-1"),
            class: RetentionClass::SealedReports,
            state: RetentionItemState::Pinned,
            byte_size: 1024,
            contains_sensitive_canaries: false,
        },
    ];

    let bkp_id = BackupId::from_u128(111);
    let backups = vec![ManagedBackupRef {
        backup_id: bkp_id,
        created_at_ms: expired_ts,
        contains_purged_bytes: true,
    }];

    let report = plan_and_execute_purge(&policy, &items, &backups, now, "purge canaris test");

    // Les copies expirées + cache sont purgées et remplacées par des tombstones.
    assert!(
        report
            .purged_item_ids
            .contains(&String::from("log-expired-1"))
    );
    assert!(
        report
            .purged_item_ids
            .contains(&String::from("cache-entry-1"))
    );
    assert_eq!(report.caches_cleared, 1);
    assert!(report.wal_checkpoint_truncate_required);
    assert!(
        report
            .tombstones
            .iter()
            .any(|t| t.item_id == "log-expired-1")
    );

    // Les objets actifs / outcome_unknown / épinglés NE SONT JAMAIS purgés.
    assert!(
        !report
            .purged_item_ids
            .contains(&String::from("active-run-log"))
    );
    assert!(
        !report
            .purged_item_ids
            .contains(&String::from("unknown-effect-payload"))
    );
    assert!(
        !report
            .purged_item_ids
            .contains(&String::from("pinned-evidence-1"))
    );
    assert_eq!(report.preserved_item_ids.len(), 3);

    // Sauvegardes gérées concernées listées avec borne 35j + limites explicites (APFS, exports, fournisseur).
    assert_eq!(report.affected_managed_backups, vec![bkp_id]);
    assert_eq!(report.backup_retention_max_days, 35);
    assert!(
        report
            .explicit_limits
            .iter()
            .any(|l| l.scope == "apfs_time_machine")
    );
    assert!(
        report
            .explicit_limits
            .iter()
            .any(|l| l.scope == "delivered_exports")
    );
    assert!(
        report
            .explicit_limits
            .iter()
            .any(|l| l.scope == "remote_provider_copies")
    );
}
