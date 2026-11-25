//! T-F012-015 / AC-F012-015 / REQ-MODEL-015 : une génération observée sur le socket du
//! démon, rejouable. Exige macOS (plan strict, écoute possédée) et l'hôte de processus
//! empaqueté depuis les mêmes sources ; aucun double de Seatbelt ne le remplace.

use super::{
    NominalTestHost, PROFILE_ID, TestResult, configure_profile, prepare_nominal_generation,
    prepare_service_fixture,
};
use crate::provider_service_evaluation::{
    claim_prepared_provider_service_launch, evaluate_provider_service_launch,
};
use crate::{DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};
use serde_json::{Map, Value, json};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use unleash_action_broker::AuthorizationOutcome;
use unleash_client::{ClientConfig, EventStreamReader, UnleashClient};
use unleash_domain::{ActionState, ActorId, RequestId, Sequence, Sha256Digest};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, EventCursor, ProtocolVersion, QueryEnvelope,
    SubscribeEventsFrame, SubscribeEventsRequest,
};
use unleash_store::{CallerAuthority, CallerRole};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const CHILD_TEST: &str =
    "provider_service_evaluation::tests::native::generation_ipc::t_f012_015_ollama_fixture_child";
const RESPONSE: &str = concat!(
    r#"{"model":"inert-fixture","created_at":"2026-11-20T00:00:00Z","message":{"role":"assistant","content":"Answer"},"done":false}"#,
    "\n",
    r#"{"model":"inert-fixture","created_at":"2026-11-20T00:00:01Z","message":{"role":"assistant","content":"!"},"done":true,"done_reason":"stop","eval_count":2}"#,
    "\n",
);

struct CompletedGeneration {
    receipt: CommandReceipt,
    action_id: Uuid,
    observed_at: EventCursor,
    profile_observation: Value,
}

#[tokio::test]
#[ignore = "requires the dedicated macOS runner and the same-source packaged process host"]
async fn t_f012_015_generation_active_mandate_persists_one_result_and_replays_after_restart()
-> TestResult {
    let host_binary = PathBuf::from(
        std::env::var_os("UNLEASH_TEST_PROCESS_HOST_BIN").ok_or("missing fixture host binary")?,
    )
    .canonicalize()?;
    let data = local_test_dir("f012-gen-data")?;
    let rendezvous = local_test_dir("f012-gen-ipc")?;
    let service = local_test_dir("f012-gen-host")?;
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let daemon = start(config.clone()).await?;
    let first = submit_generation(&daemon, service.path(), &host_binary).await;
    daemon
        .shutdown_graceful("generation-fixture-restart")
        .await?;
    let (command, completed, requests) = first?;
    assert_eq!(
        fs::read(&requests)?.as_slice(),
        b"1",
        "one native request before restart"
    );

    let restarted = start(config).await?;
    let replay = async {
        let mut client = connect(&restarted).await?;
        verify_replay(&mut client, &command, &completed).await
    }
    .await;
    restarted
        .shutdown_graceful("generation-fixture-complete")
        .await?;
    replay?;
    assert_eq!(
        fs::read(requests)?.as_slice(),
        b"1",
        "replay made no second request"
    );
    Ok(())
}

async fn start(config: DaemonConfig) -> TestResult<DaemonHandle> {
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config).await? else {
        return Err("expected a freshly started fixture daemon".into());
    };
    Ok(daemon)
}

async fn connect(daemon: &DaemonHandle) -> TestResult<UnleashClient> {
    Ok(UnleashClient::connect(ClientConfig::new(
        daemon.socket_path().to_path_buf(),
        ClientKind::Cli,
        "generation-fixture",
    ))
    .await?)
}

async fn submit_generation(
    daemon: &DaemonHandle,
    service_dir: &Path,
    host_binary: &Path,
) -> TestResult<(CommandEnvelope, CompletedGeneration, PathBuf)> {
    let reserved = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let endpoint = reserved.local_addr()?;
    let program = std::env::current_exe()?
        .canonicalize()?
        .into_os_string()
        .into_string()
        .map_err(|_| "fixture executable must be UTF-8")?;
    let runtime = daemon.test_runtime_state();
    let store = &runtime.store;
    let caller = CallerAuthority {
        actor_id: ActorId::new(format!("uid:{}", unleash_platform::effective_uid()))?,
        role: CallerRole::HumanOperator,
    };
    configure_profile(store, &caller, &format!("http://{endpoint}"))?;
    let command = prepare_nominal_generation(store, &caller, caller.actor_id.clone(), 8)?;
    let (launch, bootstrap) = prepare_service_fixture(store, &caller, service_dir, |spec| {
        spec.program = program;
        spec.arguments = vec![
            CHILD_TEST.into(),
            "--exact".into(),
            "--include-ignored".into(),
        ];
    })?;
    let child_dir = bootstrap.cwd.clone();
    fs::write(child_dir.join("listener-port"), endpoint.port().to_string())?;
    let AuthorizationOutcome::Authorized(decision) = evaluate_provider_service_launch(
        Arc::clone(store),
        launch.clone(),
        caller.clone(),
        bootstrap.clone(),
    )
    .await?
    else {
        return Err("the real isolation probe did not authorize the fixture service".into());
    };
    let permit = claim_prepared_provider_service_launch(
        Arc::clone(store),
        launch.clone(),
        caller.clone(),
        bootstrap.clone(),
        decision,
    )
    .await?;
    let controller = &runtime.provider_service_hosts;
    let owner = NominalTestHost {
        controller,
        dispatch: permit.dispatch_id(),
    };
    drop(reserved);
    let first: TestResult<_> = async {
        let launched = controller.launch_with_binary(
            store,
            &launch,
            &caller,
            bootstrap,
            permit,
            host_binary,
        )?;
        if launched.state != ActionState::ObservedSuccess {
            return Err("the fixture service launch was not attested".into());
        }
        let ready = child_dir.join("listener-ready");
        tokio::time::timeout(Duration::from_secs(3), async {
            while !ready.try_exists()? {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<_, std::io::Error>(())
        })
        .await??;
        observe_and_replay(&mut connect(daemon).await?, &command).await
    }
    .await;
    let stop = fs::write(child_dir.join("listener-stop"), []);
    let reaped = owner.finish();
    if let Err(cleanup) = stop.map_err(Box::<dyn std::error::Error>::from).and(reaped) {
        return Err(format!(
            "fixture cleanup failed: {cleanup}; probe error: {:?}",
            first.as_ref().err()
        )
        .into());
    }
    match fs::read_to_string(child_dir.join("listener-error")) {
        Ok(error) => return Err(format!("Ollama fixture request failed: {error}").into()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok((command, first?, child_dir.join("requests")))
}

async fn observe_and_replay(
    client: &mut UnleashClient,
    command: &CommandEnvelope,
) -> TestResult<CompletedGeneration> {
    let receipt = client.send_command(command).await?;
    let (action_id, observed_at) =
        tokio::time::timeout(Duration::from_secs(8), wait_for_success(client, &receipt))
            .await
            .map_err(|_| "no successful generation was published before the deadline")??;
    let (profile_observation, _) = profile_observation(client, observed_at).await?;
    if profile_observation.get("probe_id") != Some(&json!(command.aggregate_id))
        || profile_observation
            .get("result_ref")
            .and_then(Value::as_str)
            != receipt.result_ref.as_deref()
        || receipt.result_ref.is_none()
    {
        return Err("the observation does not belong to the accepted probe".into());
    }
    let completed = CompletedGeneration {
        receipt,
        action_id,
        observed_at,
        profile_observation,
    };
    verify_replay(client, command, &completed).await?;
    Ok(completed)
}

async fn subscribe_after(
    client: &UnleashClient,
    cursor: EventCursor,
) -> TestResult<EventStreamReader> {
    Ok(client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: cursor.stream_id,
            after_sequence: cursor.sequence,
            project_id: None,
            aggregate_id: None,
            batch_limit: 64,
        })
        .await?)
}

async fn wait_for_success(
    client: &UnleashClient,
    receipt: &CommandReceipt,
) -> TestResult<(Uuid, EventCursor)> {
    let from = EventCursor::new(
        receipt.event_cursor.stream_id,
        Sequence::new(receipt.event_cursor.sequence.as_u64().saturating_sub(1)),
    );
    let mut events = subscribe_after(client, from).await?;
    let mut action_id = None;
    let mut dispatch_id = None;
    while let Some(frame) = events.next_frame().await? {
        let SubscribeEventsFrame::Events { events, .. } = frame else {
            return Err("lost the fixture generation event cursor".into());
        };
        for event in events {
            if event.event_type == "ProviderProbePrepared"
                && event.aggregate_id == receipt.aggregate_id
            {
                action_id = Some(
                    event
                        .payload
                        .get("action_id")
                        .and_then(Value::as_str)
                        .ok_or("the prepared probe has no action identity")?
                        .parse::<Uuid>()?,
                );
            }
            if action_id != Some(event.aggregate_id) {
                continue;
            }
            if event.event_type == "ActionDispatchCommitted" {
                let dispatched = event
                    .payload
                    .get("dispatch_id")
                    .and_then(Value::as_str)
                    .ok_or("the committed dispatch has no identity")?
                    .parse::<Uuid>()?;
                if dispatch_id.replace(dispatched).is_some() {
                    return Err("the probe already has a committed dispatch".into());
                }
            }
            if event.event_type == "ActionObserved" {
                let observed = event
                    .payload
                    .get("dispatch_id")
                    .and_then(Value::as_str)
                    .ok_or("the observed dispatch has no identity")?
                    .parse::<Uuid>()?;
                if dispatch_id != Some(observed)
                    || event.payload.get("state") != Some(&json!("observed_success"))
                {
                    return Err("the committed dispatch was not observed as successful".into());
                }
                let _: Sha256Digest = event
                    .payload
                    .get("output_digest")
                    .and_then(Value::as_str)
                    .ok_or("the generation observation has no output digest")?
                    .parse()?;
                return Ok((
                    event.aggregate_id,
                    EventCursor::new(from.stream_id, event.sequence),
                ));
            }
        }
    }
    Err("the fixture generation event stream closed early".into())
}

async fn verify_replay(
    client: &mut UnleashClient,
    command: &CommandEnvelope,
    expected: &CompletedGeneration,
) -> TestResult {
    if client.send_command(command).await? != expected.receipt {
        return Err("replay changed the accepted receipt".into());
    }
    let (observation, watermark) = profile_observation(client, expected.observed_at).await?;
    if observation != expected.profile_observation {
        return Err("replay changed the retained generation observation".into());
    }
    tokio::time::timeout(
        Duration::from_secs(3),
        verify_no_new_effect(client, expected, watermark),
    )
    .await
    .map_err(|_| "could not read the replay through the profile watermark")?
}

async fn verify_no_new_effect(
    client: &UnleashClient,
    expected: &CompletedGeneration,
    watermark: EventCursor,
) -> TestResult {
    if watermark == expected.observed_at {
        return Ok(());
    }
    let mut stream = subscribe_after(client, expected.observed_at).await?;
    while let Some(frame) = stream.next_frame().await? {
        let SubscribeEventsFrame::Events { events, cursor } = frame else {
            return Err("lost the replay event cursor".into());
        };
        for event in events {
            if event.sequence <= watermark.sequence
                && ((event.aggregate_id == expected.receipt.aggregate_id
                    && event.event_type == "ProviderProbePrepared")
                    || (event.aggregate_id == expected.action_id
                        && event.event_type == "ActionDispatchCommitted"))
            {
                return Err("the replay prepared or dispatched the probe again".into());
            }
        }
        if cursor.sequence >= watermark.sequence {
            return Ok(());
        }
    }
    Err("the replay event stream closed early".into())
}

async fn profile_observation(
    client: &mut UnleashClient,
    expected: EventCursor,
) -> TestResult<(Value, EventCursor)> {
    let view = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(24),
            query_type: "GetProviderProfile".into(),
            project_id: None,
            payload: Map::from_iter([
                ("profile_id".into(), json!(PROFILE_ID)),
                ("profile_version".into(), json!(1)),
            ]),
        })
        .await?;
    let watermark = view.watermark.ok_or("provider profile has no watermark")?;
    if watermark.stream_id != expected.stream_id || watermark.sequence < expected.sequence {
        return Err("the profile watermark does not cover the successful generation".into());
    }
    let observations = view
        .result
        .get("observations")
        .and_then(Value::as_array)
        .ok_or("provider profile has no observations field")?;
    let mut generation = observations
        .iter()
        .filter(|entry| entry.get("phase").and_then(Value::as_str) == Some("generation_probe"));
    let observation = generation
        .next()
        .ok_or("the completed generation is missing from GetProviderProfile")?;
    if generation.next().is_some() {
        return Err("more than one generation observation for the fixture profile".into());
    }
    Ok((observation.clone(), watermark))
}

#[test]
#[ignore = "run only by the exact parent through its owned fixture service"]
fn t_f012_015_ollama_fixture_child() -> TestResult {
    let port: u16 = fs::read_to_string("listener-port")?.parse()?;
    if port == 0 {
        return Err("the parent must reserve a nonzero fixture port".into());
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    listener.set_nonblocking(true)?;
    fs::write("listener-ready", [])?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !Path::new("listener-stop").try_exists()? && Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("requests")?
                    .write_all(b"1")?;
                if let Err(error) = respond_to_probe(stream) {
                    fs::write("listener-error", error.to_string())?;
                    return Err(error);
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn respond_to_probe(mut stream: TcpStream) -> TestResult {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut reader = BufReader::new(&stream).take(16 * 1024);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line != "POST /api/chat HTTP/1.1\r\n" {
        return Err("expected the native Ollama chat POST".into());
    }
    let mut content_length = None;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err("incomplete fixture request headers".into());
        }
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = content_length
        .filter(|length| *length <= 8 * 1024)
        .ok_or("missing or unbounded fixture request body")?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let request: Value = serde_json::from_slice(&body)?;
    if request.get("model").and_then(Value::as_str) != Some("inert-fixture")
        || request.get("stream").and_then(Value::as_bool) != Some(true)
    {
        return Err("expected a streaming request for the fixture model".into());
    }
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{RESPONSE}",
        RESPONSE.len()
    )?;
    Ok(())
}
