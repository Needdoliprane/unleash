//! Native loopback fixtures, with targeted pauses in the real writer path.
//! No observation, receipt or reservation is injected into `SQLite` by this bank.

use super::{
    ConnectFixture, DispatchGuard, assert_committed_connect_proof,
    assert_completed_probe_without_model, assert_connect_observation,
    assert_no_model_or_secret_rows, capture_authorized_connect, connect, envelope, policy_command,
};
use rusqlite::OptionalExtension;
use serde_json::json;
use std::{
    future::Future,
    io::ErrorKind,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use unleash_client::{ClientConfig, ClientError, EventStreamReader, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::Revision;
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ErrorCode, ReceiptState, SubscribeEventsRequest,
};
use unleash_store::{CutpointObserver, StoreCutpoint, StoreError};
use unleash_testkit::{Cutpoint, CutpointBarrier, CutpointController};
use uuid::Uuid;

const FACT_FIELD: &str = "connect_observation";

struct ObservationPause {
    point: StoreCutpoint,
    category: Cutpoint,
    controller: CutpointController,
    barrier: Arc<CutpointBarrier>,
    entered: tokio::sync::Notify,
}

impl ObservationPause {
    fn new(point: StoreCutpoint, category: Cutpoint) -> Arc<Self> {
        let controller = CutpointController::new();
        let barrier = controller
            .arm_barrier_on_hit(category, 1, Duration::from_secs(10))
            .unwrap();
        Arc::new(Self {
            point,
            category,
            controller,
            barrier,
            entered: tokio::sync::Notify::new(),
        })
    }

    fn guard(&self) -> DispatchGuard {
        DispatchGuard {
            barrier: Arc::clone(&self.barrier),
            armed: true,
        }
    }
}

impl CutpointObserver for ObservationPause {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == self.point {
            self.entered.notify_one();
            self.controller
                .hit(self.category)
                .map_err(|error| StoreError::CutpointInjected {
                    cutpoint: format!("{:?}", self.point),
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }
}

struct ObservationPauses(Vec<Arc<ObservationPause>>);

impl CutpointObserver for ObservationPauses {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        for pause in &self.0 {
            pause.hit(point)?;
        }
        Ok(())
    }
}

struct Target {
    profile: Uuid,
    version: u64,
    receipt: CommandReceipt,
}

async fn configure_target(
    fixture: &ConnectFixture,
    client: &mut UnleashClient,
    profile: Uuid,
    id: u128,
    revision: u64,
) -> Target {
    let endpoint = format!("http://{}/v1", fixture.endpoint.local_addr().unwrap());
    // Longer bounded fixture deadlines leave room for the explicit test pauses;
    // they grant neither an effect nor a model call.
    let payload = json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":endpoint,"model":"synthetic-connect-profile","locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":20_000,"queue_timeout_ms":1_000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":60_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}});
    let mut command = envelope(client, profile, id, "SetProviderProfile", payload);
    command.expected_revision = Revision::new(revision);
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.new_revision, Revision::new(revision + 1));
    Target {
        profile,
        version: revision + 1,
        receipt,
    }
}

async fn issue_target(
    fixture: &ConnectFixture,
    client: &mut UnleashClient,
    target: &Target,
    id: u128,
    revision: u64,
) -> CommandReceipt {
    let principal:String=fixture.db().query_row(
        "SELECT p.principal_id FROM principals p JOIN command_receipts c ON c.actor_id=p.actor_id WHERE c.receipt_id=?1 AND p.state='active' AND p.principal_kind='user'",
        [target.receipt.receipt_id.to_string()],|row|row.get(0),
    ).unwrap();
    let expires = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
        + 60_000;
    let command = policy_command(
        client,
        id,
        revision,
        json!({"kind":"issue","principal_id":principal,"profile_id":target.profile,"profile_version":target.version,"phase":"connect","locality":"local_only","capability_set":[],"limits":{"timeout_ms":10_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0},"expires_at_ms":expires}),
    );
    client.send_command(&command).await.unwrap()
}

fn request_for(
    client: &UnleashClient,
    target: &Target,
    id: u128,
    mandate: &CommandReceipt,
) -> CommandEnvelope {
    let mut request = connect(client, target.profile, id, mandate);
    request
        .payload
        .insert("profile_version".into(), json!(target.version));
    request.payload.insert(
        "limits".into(),
        json!({"timeout_ms":10_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0}),
    );
    request
}

async fn subscribe(client: &UnleashClient, receipt: &CommandReceipt) -> EventStreamReader {
    client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: receipt.event_cursor.stream_id,
            after_sequence: receipt.event_cursor.sequence,
            project_id: None,
            aggregate_id: None,
            batch_limit: 64,
        })
        .await
        .unwrap()
}

async fn wait_for_observation_hook<F>(
    fixture: &ConnectFixture,
    pause: &ObservationPause,
    pending: &mut F,
) -> Option<CommandReceipt>
where
    F: Future<Output = Result<CommandReceipt, ClientError>> + Unpin,
{
    tokio::select! {
        result=pending => match result {
            Ok(receipt) => {
                tokio::time::timeout(Duration::from_secs(3),pause.entered.notified()).await.unwrap();
                Some(receipt)
            }
            Err(error) => {
                pause.barrier.abort();
                fixture.handle.shutdown_graceful("connect-observation-red").await.unwrap();
                fixture.assert_no_dispatch_or_active_unit();
                assert!(matches!(&error,ClientError::Remote(remote) if remote.code==ErrorCode::NotFoundOrHidden),"unexpected setup error: {error:?}");
                panic!("connect route is still closed before its native observation: {error:?}");
            }
        },
        reached=tokio::time::timeout(Duration::from_secs(3),pause.entered.notified()) => {
            reached.unwrap(); None
        }
    }
}

fn assert_retained_before_finish(fixture: &ConnectFixture, probe: Uuid) {
    let retained:i64=fixture.db().query_row(
        "SELECT COUNT(*) FROM provider_probe_transport_contexts c JOIN provider_probes p ON p.probe_id=c.probe_id JOIN actions a ON a.action_id=c.action_id JOIN action_dispatches d ON d.action_id=a.action_id JOIN execution_reservation_owners o ON o.provider_probe_id=p.probe_id JOIN capacity_reservations r ON r.reservation_id=o.reservation_id JOIN resource_leases l ON l.owner_id=p.probe_id AND l.resource_kind='execution' WHERE p.probe_id=?1 AND p.state IN ('admitted','running') AND a.state IN ('dispatching','in_flight') AND r.state='held' AND l.state='held' AND l.fence=p.fence AND l.authority_generation=p.authority_generation AND l.daemon_epoch=p.daemon_epoch",
        [probe.to_string()],|row|row.get(0),
    ).unwrap();
    assert_eq!(
        retained, 1,
        "native publication is not a completion or capacity release"
    );
    let completed:i64=fixture.db().query_row(
        "SELECT COUNT(*) FROM provider_probe_transport_contexts c JOIN broker_phase_records b ON b.action_id=c.action_id WHERE c.probe_id=?1 AND b.phase_key LIKE 'observation:%'",
        [probe.to_string()],|row|row.get(0),
    ).unwrap();
    assert_eq!(completed, 0);
    assert_no_model_or_secret_rows(fixture);
}

fn permanent_fact_count(fixture: &ConnectFixture, probe: Uuid) -> i64 {
    fixture.db().query_row(
        "SELECT COUNT(*) FROM object_references r JOIN object_records o ON o.object_hash=r.object_hash WHERE r.owner_kind='provider_probe' AND r.owner_id=?1 AND r.source_field=?2 AND r.state='live' AND o.state='live'",
        rusqlite::params![probe.to_string(),FACT_FIELD],|row|row.get(0),
    ).unwrap()
}

fn assert_released_after_finish(fixture: &ConnectFixture, probe: Uuid) {
    let finished:i64=fixture.db().query_row(
        "SELECT COUNT(*) FROM provider_probe_transport_contexts c JOIN provider_probes p ON p.probe_id=c.probe_id JOIN actions a ON a.action_id=c.action_id JOIN execution_reservation_owners o ON o.provider_probe_id=p.probe_id JOIN capacity_reservations r ON r.reservation_id=o.reservation_id JOIN resource_leases l ON l.owner_id=p.probe_id AND l.resource_kind='execution' WHERE p.probe_id=?1 AND p.state='ended' AND a.state='observed_success' AND r.state='released' AND l.state='released' AND l.fence=p.fence",
        [probe.to_string()],|row|row.get(0),
    ).unwrap();
    assert_eq!(
        finished, 1,
        "the final writer transaction completes the exact execution"
    );
    let causal:(i64,i64)=fixture.db().query_row(
        "SELECT COUNT(DISTINCT e.event_id),COUNT(DISTINCT disposition.evidence_id) FROM provider_probe_transport_contexts c JOIN broker_phase_records b ON b.action_id=c.action_id AND b.phase_key='observation:1' JOIN events e ON e.event_id=b.event_id AND e.event_type='ActionObserved' JOIN evidence disposition ON disposition.action_id=c.action_id AND disposition.snapshot_ref LIKE 'provider-connect-disposition:v1:%:'||e.event_id AND disposition.object_hash=json_extract(e.payload_json,'$.output_digest') AND disposition.availability='available' WHERE c.probe_id=?1 AND json_extract(e.payload_json,'$.state')='observed_success'",
        [probe.to_string()],|row|Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!(
        causal,
        (1, 1),
        "one exact ActionObserved and one attached execution disposition after replay"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t_f012_013_connect_closes_before_adoption_and_finishes_atomically() {
    let before_adoption = ObservationPause::new(
        StoreCutpoint::C1BeforeProviderProbeConnectFactAdoption,
        Cutpoint::C1,
    );
    let before_finish = ObservationPause::new(
        StoreCutpoint::C1BeforeProviderProbeConnectObservationCommit,
        Cutpoint::C1,
    );
    let pauses = Arc::new(ObservationPauses(vec![
        before_adoption.clone(),
        before_finish.clone(),
    ]));
    let (fixture, mut client) = ConnectFixture::start_with_cutpoints(Some(pauses)).await;
    // Guards are declared after the daemon so they abort barriers before its drop.
    let mut adoption_guard = before_adoption.guard();
    let mut finish_guard = before_finish.guard();
    let target =
        configure_target(&fixture, &mut client, Uuid::from_u128(0xf012_c201), 210, 0).await;
    let mandate = issue_target(&fixture, &mut client, &target, 211, 0).await;
    let mut events = subscribe(&client, &mandate).await;
    let request = request_for(&client, &target, 212, &mandate);
    let capture = capture_authorized_connect(&fixture);
    let receipt = {
        let mut pending = Box::pin(client.send_command(&request));
        let early = wait_for_observation_hook(&fixture, &before_adoption, &mut pending).await;
        assert!(
            capture.await.unwrap().is_empty(),
            "the real peer observes EOF with zero HTTP bytes"
        );
        assert_retained_before_finish(&fixture, request.aggregate_id);
        assert_eq!(permanent_fact_count(&fixture, request.aggregate_id), 0);
        adoption_guard.release();
        tokio::time::timeout(Duration::from_secs(3), before_finish.entered.notified())
            .await
            .unwrap();
        assert_eq!(permanent_fact_count(&fixture, request.aggregate_id), 1);
        assert_retained_before_finish(&fixture, request.aggregate_id);
        finish_guard.release();
        match early {
            Some(receipt) => receipt,
            None => pending.await.unwrap(),
        }
    };
    assert_connect_observation(
        &mut client,
        target.profile,
        request.aggregate_id,
        &receipt,
        &mut events,
    )
    .await;
    assert_eq!(client.send_command(&request).await.unwrap(), receipt);
    drop(events);
    fixture
        .handle
        .shutdown_graceful("connect-observation-complete")
        .await
        .unwrap();
    assert_completed_probe_without_model(&fixture, &request, &receipt);
    assert_committed_connect_proof(&fixture, &receipt);
    assert_released_after_finish(&fixture, request.aggregate_id);
}

async fn alias_targets(
    fixture: &ConnectFixture,
    client: &mut UnleashClient,
) -> (Target, CommandReceipt, Target, CommandReceipt) {
    let first = configure_target(fixture, client, Uuid::from_u128(0xf012_c202), 220, 0).await;
    let second = configure_target(fixture, client, Uuid::from_u128(0xf012_c203), 221, 0).await;
    let second = configure_target(fixture, client, second.profile, 222, 1).await;
    let first_mandate = issue_target(fixture, client, &first, 223, 0).await;
    let second_mandate = issue_target(
        fixture,
        client,
        &second,
        224,
        first_mandate.new_revision.as_u64(),
    )
    .await;
    (first, first_mandate, second, second_mandate)
}

async fn peer_client(fixture: &ConnectFixture) -> UnleashClient {
    UnleashClient::connect(ClientConfig::new(
        fixture.handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "connect-alias-peer",
    ))
    .await
    .unwrap()
}

fn action_state(fixture: &ConnectFixture, probe: Uuid) -> Option<String> {
    fixture.db().query_row(
        "SELECT a.state FROM provider_probe_transport_contexts c JOIN actions a ON a.action_id=c.action_id WHERE c.probe_id=?1",
        [probe.to_string()],|row|row.get(0),
    ).optional().unwrap()
}

async fn wait_state(
    fixture: &ConnectFixture,
    probe: Uuid,
    state: &str,
    events: &mut EventStreamReader,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while action_state(fixture, probe).as_deref() != Some(state) {
            assert!(
                events.next_frame().await.unwrap().is_some(),
                "event stream closed before native phase"
            );
        }
    })
    .await
    .unwrap();
}

fn assert_alias_retained(fixture: &ConnectFixture, first: Uuid, second: Uuid) {
    let db = fixture.db();
    let held:i64=db.query_row("SELECT COUNT(*) FROM capacity_reservations r JOIN execution_reservation_owners o ON o.reservation_id=r.reservation_id JOIN provider_probe_transport_contexts c ON c.probe_id=o.provider_probe_id WHERE r.state IN ('held','reconciling') AND c.probe_id IN (?1,?2)",
        rusqlite::params![first.to_string(),second.to_string()],|row|row.get(0)).unwrap();
    assert_eq!(held, 1);
    let second_slots:i64=db.query_row("SELECT COUNT(*) FROM capacity_reservations r JOIN execution_reservation_owners o ON o.reservation_id=r.reservation_id WHERE o.provider_probe_id=?1",
        [second.to_string()],|row|row.get(0)).unwrap();
    assert_eq!(
        second_slots, 0,
        "the other profile has no hidden reservation"
    );
    let markers:i64=db.query_row("SELECT COUNT(*) FROM action_dispatches d JOIN provider_probe_transport_contexts c ON c.action_id=d.action_id WHERE c.probe_id IN (?1,?2)",
        rusqlite::params![first.to_string(),second.to_string()],|row|row.get(0)).unwrap();
    assert_eq!(
        markers, 1,
        "the second profile cannot dispatch while the first is held/unknown"
    );
    assert!(matches!(fixture.endpoint.accept(),Err(error) if error.kind()==ErrorKind::WouldBlock));
    assert_no_model_or_secret_rows(fixture);
}

async fn accepted_while_paused<F>(pending: &mut F) -> CommandReceipt
where
    F: Future<Output = Result<CommandReceipt, ClientError>> + Unpin,
{
    let receipt = tokio::time::timeout(Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    receipt
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t_f012_013_connect_alias_profiles_serialize_native_effects() {
    let after_adoption = ObservationPause::new(
        StoreCutpoint::C2AfterProviderProbeConnectFactAdoption,
        Cutpoint::C2,
    );
    let (fixture, mut first_client) =
        ConnectFixture::start_with_cutpoints(Some(after_adoption.clone())).await;
    let mut adoption_guard = after_adoption.guard();
    let (first, first_mandate, second, second_mandate) =
        alias_targets(&fixture, &mut first_client).await;
    let mut second_client = peer_client(&fixture).await;
    let mut events = subscribe(&first_client, &second_mandate).await;
    let first_request = request_for(&first_client, &first, 225, &first_mandate);
    let second_request = request_for(&second_client, &second, 226, &second_mandate);
    let first_capture = capture_authorized_connect(&fixture);
    let (first_receipt, second_receipt) = {
        let mut first_pending = Box::pin(first_client.send_command(&first_request));
        let early = wait_for_observation_hook(&fixture, &after_adoption, &mut first_pending).await;
        assert!(first_capture.await.unwrap().is_empty());
        assert_eq!(
            permanent_fact_count(&fixture, first_request.aggregate_id),
            1
        );
        assert_retained_before_finish(&fixture, first_request.aggregate_id);
        let mut second_pending = Box::pin(second_client.send_command(&second_request));
        let second_receipt = accepted_while_paused(&mut second_pending).await;
        assert_alias_retained(
            &fixture,
            first_request.aggregate_id,
            second_request.aggregate_id,
        );
        let second_capture = capture_authorized_connect(&fixture);
        // This point is one-shot: the same observer must not pause the second
        // legitimate publication after the first reservation is actually released.
        adoption_guard.release();
        let first_receipt = match early {
            Some(receipt) => receipt,
            None => first_pending.await.unwrap(),
        };
        assert!(second_capture.await.unwrap().is_empty());
        (first_receipt, second_receipt)
    };
    wait_state(
        &fixture,
        first_request.aggregate_id,
        "observed_success",
        &mut events,
    )
    .await;
    wait_state(
        &fixture,
        second_request.aggregate_id,
        "observed_success",
        &mut events,
    )
    .await;
    assert_eq!(
        first_client.send_command(&first_request).await.unwrap(),
        first_receipt
    );
    assert_eq!(
        second_client.send_command(&second_request).await.unwrap(),
        second_receipt
    );
    let ordered:i64=fixture.db().query_row(
        "SELECT COUNT(*) FROM events ended JOIN provider_probe_transport_contexts a ON a.action_id=ended.aggregate_id JOIN events admitted ON admitted.event_type='ProviderProbeAdmitted' AND admitted.aggregate_id=?2 WHERE a.probe_id=?1 AND ended.event_type='ActionObserved' AND ended.sequence<admitted.sequence",
        rusqlite::params![first_request.aggregate_id.to_string(),second_request.aggregate_id.to_string()],|row|row.get(0),
    ).unwrap();
    assert_eq!(
        ordered, 1,
        "the second real admission follows the first atomic completion"
    );
    drop(events);
    fixture
        .handle
        .shutdown_graceful("connect-alias-complete")
        .await
        .unwrap();
    for (request, receipt) in [
        (&first_request, &first_receipt),
        (&second_request, &second_receipt),
    ] {
        assert_completed_probe_without_model(&fixture, request, receipt);
        assert_committed_connect_proof(&fixture, receipt);
        assert_released_after_finish(&fixture, request.aggregate_id);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t_f012_013_connect_adoption_ack_loss_keeps_alias_capacity_unknown() {
    let after_adoption = ObservationPause::new(
        StoreCutpoint::C2AfterProviderProbeConnectFactAdoption,
        Cutpoint::C2,
    );
    let (mut fixture, mut first_client) =
        ConnectFixture::start_with_cutpoints(Some(after_adoption.clone())).await;
    let mut adoption_guard = after_adoption.guard();
    let (first, first_mandate, second, second_mandate) =
        alias_targets(&fixture, &mut first_client).await;
    let mut second_client = peer_client(&fixture).await;
    let mut events = subscribe(&first_client, &second_mandate).await;
    let first_request = request_for(&first_client, &first, 227, &first_mandate);
    let second_request = request_for(&second_client, &second, 228, &second_mandate);
    let capture = capture_authorized_connect(&fixture);
    let (first_receipt, second_receipt) = {
        let mut first_pending = Box::pin(first_client.send_command(&first_request));
        let early = wait_for_observation_hook(&fixture, &after_adoption, &mut first_pending).await;
        assert!(
            capture.await.unwrap().is_empty(),
            "the native TCP effect already ended"
        );
        assert_eq!(
            permanent_fact_count(&fixture, first_request.aggregate_id),
            1
        );
        assert_retained_before_finish(&fixture, first_request.aggregate_id);
        let mut second_pending = Box::pin(second_client.send_command(&second_request));
        let second_receipt = accepted_while_paused(&mut second_pending).await;
        assert_alias_retained(
            &fixture,
            first_request.aggregate_id,
            second_request.aggregate_id,
        );
        after_adoption.barrier.abort();
        adoption_guard.armed = false;
        let first_receipt = match early {
            Some(receipt) => receipt,
            None => first_pending.await.unwrap(),
        };
        (first_receipt, second_receipt)
    };
    wait_state(
        &fixture,
        first_request.aggregate_id,
        "outcome_unknown",
        &mut events,
    )
    .await;
    assert_alias_retained(
        &fixture,
        first_request.aggregate_id,
        second_request.aggregate_id,
    );
    assert_eq!(
        permanent_fact_count(&fixture, first_request.aggregate_id),
        1,
        "the adopted fact survives a lost acknowledgement"
    );
    assert_eq!(
        first_client.send_command(&first_request).await.unwrap(),
        first_receipt
    );
    assert_eq!(
        second_client.send_command(&second_request).await.unwrap(),
        second_receipt
    );
    assert_alias_retained(
        &fixture,
        first_request.aggregate_id,
        second_request.aggregate_id,
    );
    drop(events);
    fixture
        .handle
        .shutdown_graceful("connect-alias-unknown")
        .await
        .unwrap();
    assert_eq!(
        action_state(&fixture, first_request.aggregate_id).as_deref(),
        Some("outcome_unknown")
    );
    assert_alias_retained(
        &fixture,
        first_request.aggregate_id,
        second_request.aggregate_id,
    );
    drop(first_client);
    drop(second_client);
    assert_unknown_after_restart(
        &mut fixture,
        [
            (&first_request, &first_receipt),
            (&second_request, &second_receipt),
        ],
        &second,
        &second_mandate,
    )
    .await;
}

async fn assert_unknown_after_restart(
    fixture: &mut ConnectFixture,
    requests: [(&CommandEnvelope, &CommandReceipt); 2],
    target: &Target,
    mandate: &CommandReceipt,
) {
    let [
        (first_request, first_receipt),
        (second_request, second_receipt),
    ] = requests;
    let epoch = fixture.handle.daemon_epoch();
    let mut restarted = restart_after_unknown(fixture).await;
    assert!(fixture.handle.daemon_epoch().as_u64() > epoch.as_u64());
    assert_eq!(
        restarted.send_command(&first_request).await.unwrap(),
        *first_receipt
    );
    assert_eq!(
        restarted.send_command(&second_request).await.unwrap(),
        *second_receipt
    );
    assert_eq!(
        action_state(&fixture, first_request.aggregate_id).as_deref(),
        Some("outcome_unknown")
    );
    assert_eq!(
        permanent_fact_count(&fixture, first_request.aggregate_id),
        1
    );
    assert_alias_retained(
        &fixture,
        first_request.aggregate_id,
        second_request.aggregate_id,
    );
    let mut recovery_events = subscribe(&restarted, &second_receipt).await;
    let fresh_request = request_for(&restarted, target, 229, mandate);
    let fresh_receipt = restarted.send_command(&fresh_request).await.unwrap();
    assert_eq!(fresh_receipt.state, ReceiptState::Accepted);
    wait_state(
        &fixture,
        fresh_request.aggregate_id,
        "authorizing",
        &mut recovery_events,
    )
    .await;
    assert_alias_retained(
        &fixture,
        first_request.aggregate_id,
        fresh_request.aggregate_id,
    );
    drop(recovery_events);
    fixture
        .handle
        .shutdown_graceful("connect-alias-restarted")
        .await
        .unwrap();
    assert_alias_retained(
        &fixture,
        first_request.aggregate_id,
        second_request.aggregate_id,
    );
    assert_alias_retained(
        &fixture,
        first_request.aggregate_id,
        fresh_request.aggregate_id,
    );
}

async fn restart_after_unknown(fixture: &mut ConnectFixture) -> UnleashClient {
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(fixture.data.path().to_path_buf()),
        rendezvous_dir: Some(fixture._rendezvous.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("real restarted daemon required");
    };
    fixture.handle = handle;
    fixture.handle.wait_ready().await.unwrap();
    UnleashClient::connect(ClientConfig::new(
        fixture.handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "connect-alias-restarted",
    ))
    .await
    .unwrap()
}
