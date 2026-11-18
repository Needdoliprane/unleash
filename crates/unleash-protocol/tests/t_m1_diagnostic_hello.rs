//! REQ-REC-001 : le diagnostic ne fabrique aucune identité durable.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use unleash_protocol::{
    DiagnosticHelloResponse, DiagnosticMode, DiagnosticReason, ProtocolVersion,
};

#[test]
fn diagnostic_hello_is_closed_and_has_no_durable_identifiers() {
    let hello = DiagnosticHelloResponse {
        protocol_version: ProtocolVersion,
        mode: DiagnosticMode::ReadOnlyRecovery,
        reason_code: DiagnosticReason::StorageCorrupt,
        max_frame_bytes: 1024,
        capabilities: vec!["query_daemon_diagnostic".into()],
    };
    let mut value = serde_json::to_value(&hello).unwrap();
    assert_eq!(value["mode"], "read_only_recovery");
    assert_eq!(value["reason_code"], "STORAGE_CORRUPT");
    for key in [
        "authority_generation",
        "daemon_epoch",
        "daemon_instance_id",
        "stream_id",
    ] {
        assert!(value.get(key).is_none());
    }
    value
        .as_object_mut()
        .unwrap()
        .insert("stream_id".into(), serde_json::json!("invented"));
    assert!(serde_json::from_value::<DiagnosticHelloResponse>(value).is_err());
}
