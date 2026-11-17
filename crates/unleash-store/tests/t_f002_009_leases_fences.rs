// T-F002-009 / AC-F002-009 / REQ-PERSIST-009
//! Clôtures monotones de ressources et rejet des anciens propriétaires (`StaleFence`).

use std::sync::Arc;

use serde_json::Map;
use unleash_domain::{
    ActorId, AuthorityGeneration, CommandId, DaemonEpoch, Fence, IdGeneratorPort, RequestId,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{
    AcquireLeaseRequest, AggregateKind, CommandMutation, CommitCommandRequest, FenceAssertion,
    LeaseState, StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, local_test_dir};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Suite T-F002-009 : fences monotones, réattribution, rejet lignée étrangère et classement suspect"
)]
fn test_t_f002_009_monotone_fences_reassignment_and_stale_worker_rejection() {
    let dir = local_test_dir("f002-009").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0009));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.id_generator = ids.clone();

    let store = UnleashStore::open(cfg).unwrap();
    let authority = store.active_authority_generation().unwrap();

    // 1. Worker A acquiert lease -> fence 1
    let lease_a = store
        .acquire_lease(AcquireLeaseRequest {
            resource_kind: "worktree".to_owned(),
            resource_key: "wt-1".to_owned(),
            owner_type: "worker".to_owned(),
            owner_id: "worker-A".to_owned(),
            ttl_ms: 30_000,
        })
        .unwrap();
    assert_eq!(lease_a.fence, Fence::new(1));
    assert_eq!(lease_a.state, LeaseState::Held);

    // Conflit si un autre worker tente d'acquérir pendant qu'elle est held
    assert!(
        store
            .acquire_lease(AcquireLeaseRequest {
                resource_kind: "worktree".to_owned(),
                resource_key: "wt-1".to_owned(),
                owner_type: "worker".to_owned(),
                owner_id: "worker-B".to_owned(),
                ttl_ms: 30_000,
            })
            .is_err()
    );

    // 2. Révoquer et réattribuer à Worker B -> fence 2 (> 1)
    let lease_b = store
        .revoke_and_reassign_lease(
            lease_a.lease_id,
            AcquireLeaseRequest {
                resource_kind: "worktree".to_owned(),
                resource_key: "wt-1".to_owned(),
                owner_type: "worker".to_owned(),
                owner_id: "worker-B".to_owned(),
                ttl_ms: 30_000,
            },
        )
        .unwrap();
    assert_eq!(lease_b.fence, Fence::new(2));
    assert_eq!(lease_b.owner_id, "worker-B");

    // 3. Libérer B puis réacquérir -> fence NE REPART PAS à 0 (obtient fence 3)
    store.release_lease(lease_b.lease_id).unwrap();
    let lease_c = store
        .acquire_lease(AcquireLeaseRequest {
            resource_kind: "worktree".to_owned(),
            resource_key: "wt-1".to_owned(),
            owner_type: "worker".to_owned(),
            owner_id: "worker-C".to_owned(),
            ttl_ms: 30_000,
        })
        .unwrap();
    assert_eq!(lease_c.fence, Fence::new(3));

    // 4. Résultat ou mutation tardive de Worker A (fence=1) refusé par StaleFence, état de C inchangé
    let stale_err = store
        .validate_lease_fence(&FenceAssertion {
            lease_id: lease_a.lease_id,
            authority_generation: authority,
            daemon_epoch: lease_a.daemon_epoch,
            fence: lease_a.fence,
            owner_id: "worker-A".to_owned(),
        })
        .unwrap_err();
    assert!(matches!(stale_err, StoreError::StaleFence { .. }));

    let rev_before = store.daemon_state().unwrap().revision;
    let late_cmd_err = store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("worker-A").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: CommandId::from_uuid(ids.new_uuid()),
                authority_generation: authority,
                actor_ref: Some("worker-A".to_owned()),
                project_id: None,
                aggregate_id: authority.as_uuid(),
                expected_revision: rev_before,
                command_type: "ReportLateResult".to_owned(),
                payload: Map::new(),
            },
            target_aggregate: AggregateKind::Daemon,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: Some(FenceAssertion {
                lease_id: lease_a.lease_id,
                authority_generation: authority,
                daemon_epoch: lease_a.daemon_epoch,
                fence: lease_a.fence,
                owner_id: "worker-A".to_owned(),
            }),
            mutation: CommandMutation::NoopDaemonBump,
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert!(matches!(late_cmd_err, StoreError::StaleFence { .. }));
    assert_eq!(store.daemon_state().unwrap().revision, rev_before);

    // 5. Autre lignée refusée même avec fence/epoch numériquement plus élevés
    let other_lineage_err = store
        .validate_lease_fence(&FenceAssertion {
            lease_id: lease_c.lease_id,
            authority_generation: AuthorityGeneration::from_u128(0xCAFE_BABE),
            daemon_epoch: DaemonEpoch::new(999),
            fence: Fence::new(999),
            owner_id: "worker-C".to_owned(),
        })
        .unwrap_err();
    assert!(matches!(
        other_lineage_err,
        StoreError::StaleAuthorityGeneration { .. }
    ));

    // 6. Redémarrage ordinaire dans la même lignée : epoch+1 classe les anciennes leases held en suspect
    let store = store.restart_same_lineage().unwrap();
    let state_after_restart: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT state FROM resource_leases WHERE lease_id = ?1",
                [lease_c.lease_id.to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(state_after_restart, "suspect");
}
