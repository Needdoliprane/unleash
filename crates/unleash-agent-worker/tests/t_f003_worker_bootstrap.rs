//! Sous-cas T-F003-001/006 : canal privé d'un véritable enfant Rust.
#![expect(clippy::unwrap_used, reason = "Isolated process transcript assertions")]

use std::io::Cursor;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use unleash_domain::Sha256Digest;
use unleash_protocol::{MAX_FRAME_BYTES, read_frame, write_frame};
use uuid::Uuid;

const WIRE_VERSION: &str = "unleash.agent-worker.v1";
const MANIFEST: &str = r#"{"schema_version":1,"entries":[],"fixture":"bootstrap-only"}"#;

fn binding() -> Value {
    json!({
        "invocation_id": Uuid::from_u128(1),
        "attempt_id": Uuid::from_u128(2),
        "agent_id": Uuid::from_u128(3),
        "session_id": Uuid::from_u128(4),
        "reservation_id": Uuid::from_u128(5),
        "lease_id": Uuid::from_u128(6),
        "authority_generation": Uuid::from_u128(7),
        "daemon_epoch": 3,
        "fence": 4,
        "context_manifest_id": Uuid::from_u128(8),
        "context_manifest_digest": Sha256Digest::from_bytes(MANIFEST.as_bytes()),
        "scope_id": "fixture-scope",
        "scope_revision": 1
    })
}

fn bootstrap(binding: &Value, manifest: &str) -> Value {
    json!({
        "wire_version": WIRE_VERSION, "binding": binding, "manifest_json": manifest
    })
}

fn permit(binding: &Value) -> Value {
    json!({"wire_version": WIRE_VERSION, "binding": binding})
}

fn frames(messages: &[Value]) -> Vec<u8> {
    let mut encoded = Vec::new();
    for message in messages {
        write_frame(&mut encoded, &serde_json::to_vec(message).unwrap()).unwrap();
    }
    encoded
}

async fn transcript(input: Vec<u8>) -> (u32, bool, Vec<Value>) {
    tokio::time::timeout(Duration::from_secs(5), async move {
        let mut child = Command::new(env!("CARGO_BIN_EXE_unleash-agent-worker"))
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(&input).await.unwrap();
        drop(stdin);
        let output = child.wait_with_output().await.unwrap();
        assert!(output.stderr.is_empty());
        let mut stdout = Cursor::new(output.stdout);
        let mut messages = Vec::new();
        while let Some(frame) = read_frame(&mut stdout).unwrap() {
            messages.push(serde_json::from_slice(&frame).unwrap());
        }
        (pid, output.status.success(), messages)
    })
    .await
    .unwrap()
}

fn worker_reply(kind: &str, pid: u32, binding: &Value) -> Value {
    json!({"wire_version": WIRE_VERSION, "kind": kind, "pid": pid, "binding": binding})
}

#[tokio::test]
async fn t_f003_001_worker_echoes_exact_manifest_and_activates_only_on_matching_permit() {
    let binding = binding();
    let (pid, success, output) =
        transcript(frames(&[bootstrap(&binding, MANIFEST), permit(&binding)])).await;
    assert!(success);
    assert_eq!(
        output,
        [
            worker_reply("ready", pid, &binding),
            worker_reply("activated", pid, &binding)
        ]
    );
}

#[tokio::test]
async fn t_f003_001_worker_exits_on_controller_eof_before_activation() {
    let binding = binding();
    let (pid, success, output) = transcript(frames(&[bootstrap(&binding, MANIFEST)])).await;
    assert!(success);
    assert_eq!(output, [worker_reply("ready", pid, &binding)]);
}

#[tokio::test]
async fn t_f003_006_worker_refuses_a_permit_with_a_different_fence() {
    let binding = binding();
    let mut stale = binding.clone();
    *stale.get_mut("fence").unwrap() = json!(2);
    let (pid, success, output) =
        transcript(frames(&[bootstrap(&binding, MANIFEST), permit(&stale)])).await;
    assert!(!success);
    assert_eq!(output, [worker_reply("ready", pid, &binding)]);
}

#[tokio::test]
async fn t_f003_006_worker_refuses_unverified_or_unexpected_bootstrap_data() {
    let binding = binding();
    let mut with_extra = bootstrap(&binding, MANIFEST);
    with_extra
        .as_object_mut()
        .unwrap()
        .insert("claimed_capability".to_owned(), json!("fixture"));
    for message in [bootstrap(&binding, "{}"), permit(&binding), with_extra] {
        let (_, success, output) = transcript(frames(&[message])).await;
        assert!(!success);
        assert!(output.is_empty());
    }
}

#[tokio::test]
async fn t_f003_006_worker_rejects_oversize_before_decoding() {
    let announced = u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_be_bytes();
    let (_, success, output) = transcript(announced.to_vec()).await;
    assert!(!success);
    assert!(output.is_empty());
}
