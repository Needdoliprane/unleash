// T-F001-005 / AC-F001-005 / REQ-CORE-005
//! Déduplication par `(acteur authentifié, command_id)` après coupure socket post-commit ;
//! rejet `IdempotencyConflict`, `RevisionConflict` (avec `current_revision`) et
//! `StaleFence/STALE_AUTHORITY_GENERATION` avant lookup idempotence.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value, json};
use unleash_client::{
    ClientConfig, ClientError, CommandCorrelationQueue, CorrelationState, UnleashClient,
};
use unleash_daemon::{
    DaemonConfig, DaemonStartOutcome, PostCommitAction, PostCommitHook, UnleashDaemon,
};
use unleash_domain::{AuthorityGeneration, CommandId, MissionId, ProjectId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, LineageStatus, ProtocolVersion, ReasonCode, RetryClass,
};
use unleash_testkit::local_test_dir;

struct DropOnceHook {
    arm_drop: AtomicBool,
}

impl PostCommitHook for DropOnceHook {
    fn after_commit(&self, _envelope: &CommandEnvelope) -> PostCommitAction {
        if self.arm_drop.swap(false, Ordering::AcqRel) {
            PostCommitAction::DropConnection
        } else {
            PostCommitAction::Continue
        }
    }
}

#[tokio::test]
async fn t_f001_005_drop_after_commit_retry_conflict_revision_and_stale_generation() {
    let rdv = local_test_dir("f001-05-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-05-dat").unwrap();
    let queue_dir = local_test_dir("f001-05-q").unwrap();

    let hook = Arc::new(DropOnceHook {
        arm_drop: AtomicBool::new(false),
    });

    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        post_commit_hook: Some(hook.clone()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };
    let sock = handle.socket_path().to_path_buf();

    let mut client =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
            .await
            .unwrap();
    let auth = client.hello().authority_generation;
    let project_id = ProjectId::from_u128(0x2301);
    let mission_id = MissionId::from_u128(0x3301);

    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x1),
            command_id: CommandId::from_u128(0xC1),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: project_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "Idempotence", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();

    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x2),
            command_id: CommandId::from_u128(0xC2),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("AssignMission"),
            payload: match json!({ "completion_policy": "mechanical", "state": "active" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();

    // 1. Armer la coupure post-commit sur PauseMission
    let mut queue =
        CommandCorrelationQueue::open_or_create(queue_dir.path().join("corr.json")).unwrap();
    let pause_cmd = CommandId::from_u128(0xC3);
    queue
        .enqueue(
            CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0x10),
                command_id: pause_cmd,
                authority_generation: auth,
                actor_ref: None,
                project_id: Some(project_id),
                aggregate_id: mission_id.as_uuid(),
                expected_revision: Revision::new(1),
                command_type: String::from("PauseMission"),
                payload: match json!({ "reason": "coupure réseau" }) {
                    Value::Object(m) => m,
                    _ => Map::new(),
                },
            },
            1000,
        )
        .unwrap();

    hook.arm_drop.store(true, Ordering::Release);
    let lost_err = client
        .submit_correlated(&mut queue, pause_cmd, RequestId::from_u128(0x11), 1001)
        .await
        .unwrap_err();
    assert!(matches!(lost_err, ClientError::Disconnected { .. }));
    assert!(matches!(
        queue.get(pause_cmd).unwrap().state,
        CorrelationState::Uncertain { .. }
    ));

    // 2. Reconnexion et retry avec nouveau request_id -> même receipt_id, aucune double mutation
    client.reconnect().await.unwrap();
    let inspected_via_query = client
        .get_command_receipt(RequestId::from_u128(0x12), pause_cmd)
        .await
        .unwrap();
    assert_eq!(inspected_via_query.lineage_status, LineageStatus::Current);
    assert_eq!(inspected_via_query.authority_generation, auth);

    let replayed_receipt = client
        .submit_correlated(&mut queue, pause_cmd, RequestId::from_u128(0x13), 1002)
        .await
        .unwrap();
    assert_eq!(
        replayed_receipt.receipt_id,
        inspected_via_query.receipt.receipt_id
    );
    assert_eq!(replayed_receipt.new_revision, Revision::new(2));

    // 3. Même command_id avec payload différent -> IdempotencyConflict
    let conflict_err = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x14),
            command_id: pause_cmd,
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::new(1),
            command_type: String::from("PauseMission"),
            payload: match json!({ "reason": "payload différent" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(
        conflict_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::IdempotencyConflict
    ));

    // 4. Révision périmée -> RevisionConflict avec current_revision = 2
    let rev_err = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x15),
            command_id: CommandId::from_u128(0xC4),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::new(1),
            command_type: String::from("ResumeMission"),
            payload: Map::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        rev_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::RevisionConflict && e.current_revision == Some(Revision::new(2))
    ));

    // 5. Lignée différente -> StaleFence/STALE_AUTHORITY_GENERATION même pour un command_id déjà committé
    let stale_gen = AuthorityGeneration::from_u128(0xDEAD_BEEF);
    let stale_err = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x16),
            command_id: pause_cmd,
            authority_generation: stale_gen,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::new(1),
            command_type: String::from("PauseMission"),
            payload: match json!({ "reason": "coupure réseau" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(
        stale_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::StaleFence
            && e.reason_code == Some(ReasonCode::StaleAuthorityGeneration)
            && e.retry_class == RetryClass::Never
    ));

    handle.shutdown_graceful("done").await.unwrap();
}
