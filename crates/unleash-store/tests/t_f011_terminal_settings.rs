//! T-F011-001: actual Settings CAS, objects and auxiliary pools; no OS process fixture.
#![expect(clippy::unwrap_used, reason = "Local Store fixtures and assertions")]

use serde_json::{Map, json};
use unleash_domain::{ActorId, CommandId, RequestId, Revision};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::terminal_allocations::TerminalRuntimeSettings;
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    SettingsPatch, StoreConfig, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;

fn configuration(locale: &str) -> TerminalRuntimeSettings {
    serde_json::from_value(json!({"schema_version":1,"profiles":[{"profile_id":"settings-test","version":1,"variables":{"LANG":locale}}],"max_hosts":2,"max_processes":2,"max_ptys":2,"lease_ttl_ms":60000})).unwrap()
}
fn request(
    store: &UnleashStore,
    id: u128,
    expected: u64,
    configuration: Option<TerminalRuntimeSettings>,
) -> CommitCommandRequest {
    let command = CommandId::from_u128(id);
    let mut patch = SettingsPatch::default();
    let wire = if let Some(cfg) = configuration {
        let prepared = cfg
            .profiles
            .iter()
            .map(|p| store.prepare_terminal_environment(p, command).unwrap())
            .collect::<Vec<_>>();
        patch.extra_typed = Some(Map::from_iter([
            (
                "terminal_runtime".into(),
                serde_json::to_value(&cfg).unwrap(),
            ),
            (
                "__terminal_prepared".into(),
                serde_json::to_value(prepared).unwrap(),
            ),
        ]));
        json!({"terminal_runtime":cfg})
    } else {
        json!({"confirm_retention_shortening":false})
    };
    let daemon = store.daemon_state().unwrap();
    let actor = ActorId::new("uid:terminal-settings-test").unwrap();
    CommitCommandRequest {
        actor_id: actor.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(id),
            command_id: command,
            authority_generation: daemon.authority_generation,
            actor_ref: None,
            project_id: None,
            aggregate_id: daemon.authority_generation.as_uuid(),
            expected_revision: Revision::new(expected),
            command_type: "SetSettings".into(),
            payload: json!({"scope_kind":"user","scope_id":"default","patch":wire})
                .as_object()
                .unwrap()
                .clone(),
        },
        target_aggregate: AggregateKind::Settings,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::SetSettings {
            scope_kind: unleash_store::SettingsScopeKind::User,
            scope_id: "default".into(),
            caller: CallerAuthority {
                actor_id: actor,
                role: CallerRole::PolicyAdmin,
            },
            patch,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}
fn pools(store: &UnleashStore) -> Vec<(String, i64, i64)> {
    store.read_snapshot(|conn,_|{
        let mut s=conn.prepare("SELECT pool_id,revision,admissions_frozen FROM capacity_pools WHERE pool_kind IN ('process_host','process','pty') ORDER BY pool_id")?;
        let rows=s.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        rows.collect::<Result<Vec<_>,_>>().map_err(unleash_store::StoreError::from_rusqlite)
    }).unwrap()
}

#[test]
fn t_f011_001_omission_preserves_pool_revisions_and_explicit_update_preserves_freeze() {
    let dir = local_test_dir("terminal-settings-pools").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    store
        .commit_command(request(&store, 1, 0, Some(configuration("C"))))
        .unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_|{
        tx.execute("UPDATE capacity_pools SET admissions_frozen=1 WHERE pool_kind IN ('process_host','process','pty')",[])?;
        Ok(())
    }).unwrap();
    let frozen = pools(&store);
    assert_eq!(frozen.len(), 3);
    assert!(frozen.iter().all(|p| p.2 == 1));
    store.commit_command(request(&store, 2, 1, None)).unwrap();
    assert_eq!(
        pools(&store),
        frozen,
        "legacy patch must not touch auxiliary pools"
    );
    let mut cfg = configuration("C");
    cfg.max_hosts = 3;
    store
        .commit_command(request(&store, 3, 2, Some(cfg)))
        .unwrap();
    assert!(
        pools(&store).iter().all(|p| p.2 == 1),
        "configuration cannot clear an existing freeze"
    );
}

#[test]
fn t_f011_001_stale_cas_cannot_reserve_profile_version_and_history_survives_omission() {
    let dir = local_test_dir("terminal-settings-stale").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    store.commit_command(request(&store, 10, 0, None)).unwrap();
    assert!(
        store
            .commit_command(request(&store, 11, 0, Some(configuration("C"))))
            .is_err()
    );
    let bindings: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM object_references WHERE owner_kind='terminal_environment'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(bindings, 0, "failed CAS cannot bind a profile version");
    store
        .commit_command(request(&store, 12, 1, Some(configuration("C.UTF-8"))))
        .unwrap();
    assert!(
        store
            .commit_command(request(&store, 13, 2, Some(configuration("C"))))
            .is_err()
    );
    let mut newer = configuration("C.UTF-8");
    newer.profiles.first_mut().unwrap().version = 2;
    store
        .commit_command(request(&store, 14, 2, Some(newer)))
        .unwrap();
    let mut other = configuration("C");
    other.profiles.first_mut().unwrap().profile_id = "another".into();
    store
        .commit_command(request(&store, 15, 3, Some(other)))
        .unwrap();
    assert!(
        store
            .commit_command(request(&store, 16, 4, Some(configuration("C.UTF-8"))))
            .is_err(),
        "omitting a profile cannot reset its published version history"
    );
}
