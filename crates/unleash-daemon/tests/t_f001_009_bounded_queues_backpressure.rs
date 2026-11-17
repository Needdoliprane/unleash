// T-F001-009 / AC-F001-009 / REQ-CORE-009
//! Files IPC/writer et d'abonnement bornées : saturation retourne `ResourceBusy/BACKPRESSURE`
//! sans bloquer le diagnostic (`GetDaemonStatus`, `GetHealth`, `GetCommandReceipt`), et un abonné
//! lent est déconnecté sans bloquer le writer ni perdre les événements durables.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, StoreOverrides, UnleashDaemon};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision, Sequence};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReasonCode, RetryClass,
    SubscribeEventsFrame, SubscribeEventsRequest,
};
use unleash_store::{CutpointObserver, StoreCutpoint, StoreError};
use unleash_testkit::{Cutpoint, CutpointController, local_test_dir};

struct DaemonCutpointAdapter(CutpointController);

impl CutpointObserver for DaemonCutpointAdapter {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        let cp = match cutpoint {
            StoreCutpoint::C0BeforeCommandTx => Cutpoint::C0,
            StoreCutpoint::C1BeforeCommandCommit => Cutpoint::C1,
            StoreCutpoint::C2AfterCommandCommit => Cutpoint::C2,
            _ => return Ok(()),
        };
        self.0.hit(cp).map_err(|e| StoreError::CutpointInjected {
            cutpoint: cp.as_str().to_owned(),
            message: e.to_string(),
        })
    }
}

#[tokio::test]
async fn t_f001_009_bounded_writer_queue_backpressure_and_slow_subscriber_isolation() {
    let rdv = local_test_dir("f001-09-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-09-dat").unwrap();
    let ctrl = CutpointController::new();

    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        max_in_flight_mutations: 32,
        subscriber_queue_capacity: 2,
        subscriber_write_timeout: Duration::from_millis(200),
        store_overrides: StoreOverrides {
            writer_queue_capacity: Some(2),
            cutpoints: Some(Arc::new(DaemonCutpointAdapter(ctrl.clone()))),
            ..StoreOverrides::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };
    let sock = handle.socket_path().to_path_buf();

    // 1. Committer une première commande de référence (récupérable même pendant saturation)
    let mut client =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
            .await
            .unwrap();
    let auth = client.hello().authority_generation;
    let first_proj = ProjectId::from_u128(0x2900);
    let first_cmd = CommandId::from_u128(0xC900);
    let first_rcpt = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x1),
            command_id: first_cmd,
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(first_proj),
            aggregate_id: first_proj.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "Baseline", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();

    // 2. Ouvrir un abonné lent qui ne lit JAMAIS son flux SubscribeEvents
    let mut slow_reader = client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: handle.stream_id(),
            after_sequence: Sequence::new(0),
            project_id: None,
            aggregate_id: None,
            batch_limit: 1,
        })
        .await
        .unwrap();

    // Envoyer plusieurs commits volumineux sans lire `slow_reader` -> sature le socket/file bornée
    // et coupe l'abonné lent sans bloquer le writer ni les requêtes suivantes
    let pad = "x".repeat(4096);
    for idx in 1..=45_u128 {
        let p = ProjectId::from_u128(0x2900 + idx);
        client
            .send_command(&CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0x100 + idx),
                command_id: CommandId::from_u128(0xCA00 + idx),
                authority_generation: auth,
                actor_ref: None,
                project_id: Some(p),
                aggregate_id: p.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: String::from("CreateProject"),
                payload: match json!({ "name": format!("Burst-{idx}-{pad}"), "weight": 1 }) {
                    Value::Object(m) => m,
                    _ => Map::new(),
                },
            })
            .await
            .unwrap();
    }
    // Vérifier que l'abonné lent se termine proprement (EOF/coupure) sans bloquer indéfiniment
    let mut drained = 0_usize;
    while let Ok(Some(_)) = slow_reader.next_frame().await {
        drained += 1;
        if drained > 50 {
            break;
        }
    }
    drop(slow_reader);

    // Un lecteur sain retrouve tous les événements durables sans perte
    let mut healthy_sub = client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: handle.stream_id(),
            after_sequence: Sequence::new(0),
            project_id: None,
            aggregate_id: None,
            batch_limit: 64,
        })
        .await
        .unwrap();
    let frame = healthy_sub.next_frame().await.unwrap().unwrap();
    match frame {
        SubscribeEventsFrame::Events { events, .. } => {
            assert!(events.len() >= 46);
        }
        other => panic!("Attendu lot d'événements durables, reçu {other:?}"),
    }

    // 3. Saturation déterministe de la file du writer (capacité = 2) via barrière sur C1
    let barrier = ctrl.arm_barrier(Cutpoint::C1, Duration::from_secs(10));
    let mut in_flight = tokio::task::JoinSet::new();
    for idx in 0..8_u128 {
        let s = sock.clone();
        in_flight.spawn(async move {
            let mut c = UnleashClient::connect(ClientConfig::new(s, ClientKind::Cli, "0.1.0"))
                .await
                .unwrap();
            let p = ProjectId::from_u128(0x5000 + idx);
            c.send_command(&CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0x500 + idx),
                command_id: CommandId::from_u128(0xCB00 + idx),
                authority_generation: auth,
                actor_ref: None,
                project_id: Some(p),
                aggregate_id: p.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: String::from("CreateProject"),
                payload: match json!({ "name": format!("Sat-{idx}"), "weight": 1 }) {
                    Value::Object(m) => m,
                    _ => Map::new(),
                },
            })
            .await
        });
    }

    let barrier_wait = Arc::clone(&barrier);
    tokio::task::spawn_blocking(move || barrier_wait.wait_arrived(1, Duration::from_secs(10)))
        .await
        .unwrap()
        .unwrap();

    // Pendant que le writer est bloqué sur C1, les requêtes supplémentaires reçoivent BACKPRESSURE
    // tandis que GetDaemonStatus, GetHealth et GetCommandReceipt restent disponibles immédiatement !
    let diag_status = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x601),
            query_type: String::from("GetDaemonStatus"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        diag_status.result.get("mode").and_then(Value::as_str),
        Some("ready")
    );

    let diag_health = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x602),
            query_type: String::from("GetHealth"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert!(diag_health.result.get("browser_runtimes").is_some());

    let recovered_receipt = client
        .get_command_receipt(RequestId::from_u128(0x603), first_cmd)
        .await
        .unwrap();
    assert_eq!(recovered_receipt.receipt.receipt_id, first_rcpt.receipt_id);

    barrier.release();

    let mut backpressure_seen = 0_usize;
    let mut ok_seen = 0_usize;
    while let Some(res) = in_flight.join_next().await {
        match res.unwrap() {
            Ok(_) => ok_seen += 1,
            Err(ClientError::Remote(ref e))
                if e.code == ErrorCode::ResourceBusy
                    && e.reason_code == Some(ReasonCode::Backpressure)
                    && e.retry_class == RetryClass::AfterBackoff =>
            {
                backpressure_seen += 1;
            }
            Err(other) => panic!("Erreur inattendue sous saturation : {other:?}"),
        }
    }
    assert!(ok_seen >= 1);
    assert!(
        backpressure_seen >= 1,
        "Au moins une requête excédentaire doit recevoir ResourceBusy/BACKPRESSURE"
    );

    handle.shutdown_graceful("done").await.unwrap();
}
