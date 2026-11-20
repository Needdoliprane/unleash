//! A real policy change after V008 must not dispatch an old scoped intention.

use super::*;
use serde_json::Value;
use unleash_domain::{ProjectId, RequestId, Sha256Digest};
use unleash_protocol::{ProtocolVersion, QueryEnvelope, canonical_sha256};

struct ScopedTarget {
    project: ProjectId,
    target: Target,
    mandate: CommandReceipt,
}

fn scoped_policy(
    client: &UnleashClient,
    project: Option<ProjectId>,
    id: u128,
    expected: u64,
    version: &str,
    mandate: Option<Value>,
) -> CommandEnvelope {
    let (scope, scope_id, aggregate) = project.map_or_else(
        || {
            (
                "global",
                "global".to_owned(),
                client.hello().authority_generation.as_uuid(),
            )
        },
        |project| ("project", project.to_string(), project.as_uuid()),
    );
    let mut request = envelope(
        client,
        aggregate,
        id,
        "SetPolicy",
        json!({
            "scope_kind":scope,"scope_id":scope_id,"isolation_mode":"standard",
            "policy_version":version,"reason":"local policy race fixture"
        }),
    );
    request.project_id = project;
    request.expected_revision = Revision::new(expected);
    if let Some(mandate) = mandate {
        request
            .payload
            .insert("provider_probe_mandate".into(), mandate);
    }
    request
}

async fn project_target(fixture: &ConnectFixture, client: &mut UnleashClient) -> ScopedTarget {
    let project = ProjectId::from_u128(0xf012_0500);
    client
        .send_command(&envelope(
            client,
            project.as_uuid(),
            1300,
            "CreateProject",
            json!({"name":"connect-policy-race"}),
        ))
        .await
        .unwrap();
    let profile = Uuid::from_u128(0xf012_0501);
    let endpoint = format!("http://{}/v1", fixture.endpoint.local_addr().unwrap());
    let mut configuration = envelope(
        client,
        profile,
        1301,
        "SetProviderProfile",
        json!({
            "provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},
            "endpoint":endpoint,"model":"synthetic-policy-race-profile","locality":"local_only",
            "server_management":"user_managed","auth":{"kind":"none"},
            "limits":{"max_inflight":1,"connect_timeout_ms":20_000,"queue_timeout_ms":1_000,
                "load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":60_000},
            "declared_capabilities":["text"],"pricing":{"kind":"none_local"}
        }),
    );
    configuration.project_id = Some(project);
    let receipt = client.send_command(&configuration).await.unwrap();
    let principal: String = fixture.db().query_row(
        "SELECT p.principal_id FROM principals p JOIN command_receipts c ON c.actor_id=p.actor_id WHERE c.receipt_id=?1 AND p.state='active' AND p.principal_kind='user'",
        [receipt.receipt_id.to_string()], |row| row.get(0),
    ).unwrap();
    let expires = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
        + 60_000;
    let issued = scoped_policy(
        client,
        Some(project),
        1302,
        0,
        "project-v1",
        Some(json!({
            "kind":"issue","principal_id":principal,"profile_id":profile,"profile_version":1,
            "phase":"connect","locality":"local_only","capability_set":[],
            "limits":{"timeout_ms":10_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0},
            "expires_at_ms":expires
        })),
    );
    let mandate = client.send_command(&issued).await.unwrap();
    for (id, expected, version) in [(1303, 1, "project-v2"), (1304, 2, "project-v3")] {
        let updated = client
            .send_command(&scoped_policy(
                client,
                Some(project),
                id,
                expected,
                version,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(updated.new_revision, Revision::new(expected + 1));
    }
    client
        .send_command(&scoped_policy(client, None, 1305, 0, "global-v1", None))
        .await
        .unwrap();
    ScopedTarget {
        project,
        target: Target {
            profile,
            version: 1,
            receipt,
        },
        mandate,
    }
}

async fn policy_state(
    client: &mut UnleashClient,
    project: ProjectId,
    id: u128,
) -> (u64, Sha256Digest) {
    let view = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(id),
            query_type: "ReadPolicySummary".into(),
            project_id: Some(project),
            payload: serde_json::Map::default(),
        })
        .await
        .unwrap()
        .result;
    let revision = view
        .pointer("/global_policy/revision")
        .and_then(Value::as_u64)
        .unwrap()
        .max(
            view.pointer("/project_policy/revision")
                .and_then(Value::as_u64)
                .unwrap(),
        );
    let digest = canonical_sha256(&json!({"schema_version":1,"global_policy":view.get("global_policy"),"project_policy":view.get("project_policy"),"guard_config":view.get("guard_config")})).unwrap();
    (revision, digest)
}

fn assert_authorized_and_held(
    fixture: &ConnectFixture,
    request: &CommandEnvelope,
    digest: Sha256Digest,
) {
    let held: i64 = fixture.db().query_row(
        "SELECT COUNT(*) FROM provider_probe_transport_contexts c JOIN actions a ON a.action_id=c.action_id JOIN execution_reservation_owners o ON o.provider_probe_id=c.probe_id JOIN capacity_reservations r ON r.reservation_id=o.reservation_id JOIN resource_leases l ON l.owner_id=c.probe_id AND l.resource_kind='execution' WHERE c.probe_id=?1 AND json_extract(c.context_json,'$.policy_revision')=3 AND json_extract(c.context_json,'$.policy_digest')=?2 AND a.state='authorized' AND r.state='held' AND l.state='held' AND l.fence>0 AND NOT EXISTS(SELECT 1 FROM action_dispatches d WHERE d.action_id=a.action_id)",
        rusqlite::params![request.aggregate_id.to_string(),digest.to_string()], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        held, 1,
        "the policy change is strictly after V008 and durable Authorization"
    );
    assert!(matches!(fixture.endpoint.accept(),Err(error) if error.kind()==ErrorKind::WouldBlock));
    assert_no_model_or_secret_rows(fixture);
}

fn assert_no_dispatch_and_released(fixture: &ConnectFixture, probe: Uuid) {
    let dispatches: i64 = fixture
        .db()
        .query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        dispatches, 0,
        "changed full policy digest must refuse before any marker"
    );
    assert!(matches!(fixture.endpoint.accept(),Err(error) if error.kind()==ErrorKind::WouldBlock));
    let released: i64 = fixture.db().query_row(
        "SELECT COUNT(*) FROM provider_probes p JOIN execution_reservation_owners o ON o.provider_probe_id=p.probe_id JOIN capacity_reservations r ON r.reservation_id=o.reservation_id JOIN resource_leases l ON l.owner_id=p.probe_id AND l.resource_kind='execution' WHERE p.probe_id=?1 AND p.state='ended' AND r.state='released' AND l.state='released' AND l.fence=p.fence",
        [probe.to_string()],|row|row.get(0),
    ).unwrap();
    assert_eq!(
        released, 1,
        "a proven pre-dispatch refusal must dispose of its exact held reservation"
    );
    assert_no_model_or_secret_rows(fixture);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t_f012_013_connect_policy_digest_change_after_admission_disposes_without_dispatch() {
    let pause = ObservationPause::new(
        StoreCutpoint::C3AfterProviderProbeConnectAuthorizationCommit,
        Cutpoint::C3,
    );
    let (fixture, mut client) = ConnectFixture::start_with_cutpoints(Some(pause.clone())).await;
    let mut guard = pause.guard();
    let target = project_target(&fixture, &mut client).await;
    let before = policy_state(&mut client, target.project, 1307).await;
    assert_eq!(before.0, 3);
    let mut request = request_for(&client, &target.target, 1306, &target.mandate);
    request.project_id = Some(target.project);
    let receipt = {
        let mut pending = Box::pin(client.send_command(&request));
        let early = wait_for_observation_hook(&fixture, &pause, &mut pending).await;
        assert_authorized_and_held(&fixture, &request, before.1);
        let mut other = peer_client(&fixture).await;
        let changed = other
            .send_command(&scoped_policy(&other, None, 1308, 1, "global-v2", None))
            .await
            .unwrap();
        assert_eq!(changed.new_revision, Revision::new(2));
        let after = policy_state(&mut other, target.project, 1309).await;
        assert_eq!(
            after.0, before.0,
            "the max revision is deliberately unchanged"
        );
        assert_ne!(
            after.1, before.1,
            "the full global/project/Guard digest changed"
        );
        guard.release();
        match early {
            Some(receipt) => receipt,
            None => pending.await.unwrap(),
        }
    };
    super::super::connect_trace::wait_for_dispatch_refusal(request.aggregate_id).await;
    fixture
        .handle
        .shutdown_graceful("connect-policy-refused")
        .await
        .unwrap();
    assert_no_dispatch_and_released(&fixture, request.aggregate_id);
    assert_eq!(receipt.state, ReceiptState::Accepted);
}
