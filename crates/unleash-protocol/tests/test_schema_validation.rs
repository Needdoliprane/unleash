//! Tests de validation des DTOs sérialisés contre les JSON Schemas 2020-12 de `.claude/schemas/`.
// REQ-CORE-003 / REQ-CORE-005 / REQ-CORE-009 / REQ-CORE-010 / REQ-PERSIST-004

#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation
)]

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::Path;
use unleash_domain::{
    AuthorityGeneration, CommandId, DaemonEpoch, DaemonInstanceId, OperationId, ProjectId,
    ReceiptId, RequestId, Revision, Sequence, Sha256Digest, StreamId,
};
use unleash_protocol::{
    ClientKind, ClientUploadSegment, CommandEnvelope, CommandReceipt, CommandReceiptInspection,
    ErrorCode, EventCausation, EventCursor, EventEnvelope, HelloRequest, HelloResponse, IpcError,
    LineageStatus, MAX_CLIENT_UPLOAD_BYTES, ProtocolError, ProtocolVersion, QueryEnvelope,
    QueryResponse, ReasonCode, ReceiptState, RetryClass, Sensitivity, SubscribeEventsFrame,
    SubscribeEventsRequest, from_json_str_strict,
};

fn json_obj(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("La fixture de test doit être un objet JSON, obtenu : {other:?}"),
    }
}

fn load_schema(rel_path: &str) -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let full_path = root.join(".claude/schemas").join(rel_path);
    let content = fs::read_to_string(&full_path)
        .unwrap_or_else(|e| panic!("Impossible de lire le schéma {}: {e}", full_path.display()));
    serde_json::from_str(&content).expect("Schéma JSON valide")
}

fn build_validator(rel_path: &str) -> jsonschema::Validator {
    let schema = load_schema(rel_path);
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap_or_else(|e| panic!("Validateur {rel_path}: {e}"))
}

fn ipc_command_fixture() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(".claude/examples/ipc-command.json"))
        .expect("Lecture fixture ipc-command.json")
}

#[test]
fn test_validate_ipc_command_schema() {
    let validator = build_validator("ipc-command.schema.json");

    // 1. Valider la fixture `.claude/examples/ipc-command.json`
    let fixture_content = ipc_command_fixture();
    let fixture_val: Value = serde_json::from_str(&fixture_content).expect("JSON fixture");

    let eval = validator.validate(&fixture_val);
    assert!(
        eval.is_ok(),
        "ipc-command.json doit être valide contre son schéma"
    );

    // Désérialiser dans CommandEnvelope et ré-sérialiser
    let envelope: CommandEnvelope =
        from_json_str_strict(&fixture_content).expect("Désérialisation stricte");
    let reserialized = serde_json::to_value(&envelope).expect("Sérialisation CommandEnvelope");
    assert!(
        validator.is_valid(&reserialized),
        "CommandEnvelope sérialisée doit valider le schéma"
    );

    // 2. Cas négatif : unleash.ipc.v1 refusé
    let mut v1_val = fixture_val.clone();
    v1_val["protocol_version"] = json!("unleash.ipc.v1");
    assert!(
        !validator.is_valid(&v1_val),
        "v1 doit être refusé par le schéma"
    );
    let v1_str = serde_json::to_string(&v1_val).unwrap();
    let des_res: Result<CommandEnvelope, _> = from_json_str_strict(&v1_str);
    assert!(
        matches!(
            des_res,
            Err(ProtocolError::ProtocolIncompatible { ref version }) if version == "unleash.ipc.v1"
        ),
        "CommandEnvelope v1 doit renvoyer ProtocolIncompatible, obtenu : {des_res:?}"
    );

    // 2b. Version inconnue v99 refusée avec ProtocolIncompatible
    let mut v99_val = fixture_val.clone();
    v99_val["protocol_version"] = json!("unleash.ipc.v99");
    let v99_res: Result<CommandEnvelope, _> =
        from_json_str_strict(&serde_json::to_string(&v99_val).unwrap());
    assert!(
        matches!(
            v99_res,
            Err(ProtocolError::ProtocolIncompatible { ref version }) if version == "unleash.ipc.v99"
        ),
        "CommandEnvelope v99 doit renvoyer ProtocolIncompatible, obtenu : {v99_res:?}"
    );

    // 2c. Enveloppe v1 sans authority_generation refusée avec ProtocolIncompatible
    let mut v1_no_auth = v1_val.clone();
    if let Some(obj) = v1_no_auth.as_object_mut() {
        obj.remove("authority_generation");
    }
    let v1_no_auth_res: Result<CommandEnvelope, _> =
        from_json_str_strict(&serde_json::to_string(&v1_no_auth).unwrap());
    assert!(
        matches!(
            v1_no_auth_res,
            Err(ProtocolError::ProtocolIncompatible { ref version }) if version == "unleash.ipc.v1"
        ),
        "CommandEnvelope v1 sans authority_generation doit renvoyer ProtocolIncompatible, obtenu : {v1_no_auth_res:?}"
    );

    // 3. Cas négatif : expected_revision manquant
    let mut missing_rev = fixture_val.clone();
    missing_rev
        .as_object_mut()
        .unwrap()
        .remove("expected_revision");
    assert!(!validator.is_valid(&missing_rev));
    let missing_rev_res: Result<CommandEnvelope, _> =
        from_json_str_strict(&serde_json::to_string(&missing_rev).unwrap());
    assert!(matches!(missing_rev_res, Err(ProtocolError::Json(_))));

    // 4. Cas négatif : authority_generation manquant
    let mut missing_gen = fixture_val.clone();
    missing_gen
        .as_object_mut()
        .unwrap()
        .remove("authority_generation");
    assert!(!validator.is_valid(&missing_gen));
    let missing_gen_res: Result<CommandEnvelope, _> =
        from_json_str_strict(&serde_json::to_string(&missing_gen).unwrap());
    assert!(matches!(missing_gen_res, Err(ProtocolError::Json(_))));

    // 5. Cas négatif : UUID invalide pour aggregate_id (vérifié via formats activés)
    let mut bad_uuid = fixture_val.clone();
    bad_uuid["aggregate_id"] = json!("not-a-uuid");
    assert!(
        !validator.is_valid(&bad_uuid),
        "Un UUID invalide doit être rejeté par le validateur de formats"
    );
    let bad_uuid_res: Result<CommandEnvelope, _> =
        from_json_str_strict(&serde_json::to_string(&bad_uuid).unwrap());
    assert!(matches!(bad_uuid_res, Err(ProtocolError::Json(_))));

    // 6. Cas négatif : champ inconnu rejeté
    let mut unknown_field = fixture_val.clone();
    unknown_field["extra_privileged_flag"] = json!(true);
    assert!(!validator.is_valid(&unknown_field));
    let unknown_res: Result<CommandEnvelope, _> =
        from_json_str_strict(&serde_json::to_string(&unknown_field).unwrap());
    assert!(matches!(unknown_res, Err(ProtocolError::Json(_))));

    // 7. Cas négatif : payload non-objet rejeté par le schéma ET par from_json_str_strict
    for invalid_payload in [
        json!([]),
        json!([1, 2]),
        json!("texte"),
        json!(42),
        json!(true),
        Value::Null,
    ] {
        let mut bad_payload = fixture_val.clone();
        bad_payload["payload"] = invalid_payload;
        assert!(
            !validator.is_valid(&bad_payload),
            "Un payload non-objet doit être rejeté par ipc-command.schema.json"
        );
        let res: Result<CommandEnvelope, _> =
            from_json_str_strict(&serde_json::to_string(&bad_payload).unwrap());
        assert!(
            matches!(res, Err(ProtocolError::Json(_))),
            "CommandEnvelope doit rejeter tout payload non-objet, obtenu : {res:?}"
        );
    }

    let mut probe = fixture_val.clone();
    probe["command_type"] = json!("TestProviderProfile");
    probe["payload"] = json!({
        "profile_id": "fixture-profile",
        "profile_version": 9_007_199_254_740_991_u64,
        "phase": "generation_probe",
        "capability_set": ["text"],
        "limits": {
            "timeout_ms": 60_000,
            "max_calls": 3,
            "max_output_tokens": 512,
            "max_cost_usd_micros": 100_000
        },
        "locality": "approved_remote",
        "mandate_ref": "fixture-mandate"
    });
    assert!(validator.is_valid(&probe));

    let mut unicode_reference = probe.clone();
    unicode_reference["payload"]["mandate_ref"] = json!("\u{feff}");
    assert!(validator.is_valid(&unicode_reference));
    unicode_reference["payload"]["mandate_ref"] = json!("\u{00a0}\u{2003}");
    assert!(!validator.is_valid(&unicode_reference));

    let mut out_of_range = probe.clone();
    out_of_range["payload"]["profile_version"] = json!(9_007_199_254_740_992_u64);
    assert!(!validator.is_valid(&out_of_range));

    let mut unknown_origin = probe.clone();
    unknown_origin["payload"]["origin_kind"] = json!("control_plane");
    assert!(!validator.is_valid(&unknown_origin));

    probe["payload"]["limits"]["max_calls"] = json!(4);
    assert!(!validator.is_valid(&probe));
}

#[test]
fn test_validate_set_provider_profile_payload_schema() {
    let validator = build_validator("ipc-command.schema.json");
    let mut command: Value = serde_json::from_str(&ipc_command_fixture()).unwrap();
    command["command_type"] = json!("SetProviderProfile");
    command["payload"] = json!({
        "provider_kind": "openai",
        "dialect": {"kind": "openai_responses", "version": 1},
        "endpoint": "https://fixture.invalid/a%20b",
        "model": "fixture-model",
        "locality": "approved_remote",
        "server_management": "user_managed",
        "auth": {"kind": "secret_ref", "secret_ref": "fixture:opaque"},
        "limits": {"max_inflight": 1, "connect_timeout_ms": 2000, "queue_timeout_ms": 10000,
            "load_timeout_ms": 180_000, "first_token_timeout_ms": 60_000, "generation_timeout_ms": 300_000},
        "declared_capabilities": ["text", "stream"],
        "pricing": {"kind": "catalog", "catalog_id": "fixture-catalog", "catalog_version": "1",
            "catalog_digest": format!("sha256:{}", "a".repeat(64)),
            "entry_digest": {"algorithm": "sha256", "hex": "b".repeat(64)}, "stale_policy": "block"}
    });
    assert!(validator.is_valid(&command));

    let mut bare_digest = command.clone();
    let hex = "b".repeat(64);
    bare_digest["payload"]["pricing"]["entry_digest"] = json!(hex);
    assert!(validator.is_valid(&bare_digest));
    bare_digest["payload"]["pricing"]["entry_digest"] = json!(format!("{hex}\n"));
    assert!(
        !validator.is_valid(&bare_digest),
        "empreinte avec fin de ligne"
    );

    let mut forged = command.clone();
    forged["payload"]["principal_id"] = json!("fixture:principal");
    assert!(
        !validator.is_valid(&forged),
        "SetProviderProfile refuse une autorité déclarée"
    );

    for (field, invalid) in [
        (
            "dialect",
            json!({"kind": "anthropic_messages", "version": 1}),
        ),
        ("endpoint", json!("https://fixture-user@fixture.invalid/")),
        ("auth", json!({"kind": "none"})),
        ("pricing", json!({"kind": "none_local"})),
        ("declared_capabilities", json!(["text", "text"])),
    ] {
        let mut invalid_command = command.clone();
        invalid_command["payload"][field] = invalid;
        assert!(!validator.is_valid(&invalid_command), "payload.{field}");
    }

    let mut local = command.clone();
    local["payload"]["provider_kind"] = json!("ollama");
    local["payload"]["dialect"] = json!({"kind": "ollama_chat", "version": 1});
    local["payload"]["endpoint"] = json!("http://[::1]:11434/api/chat");
    local["payload"]["locality"] = json!("local_only");
    local["payload"]["auth"] = json!({"kind": "none"});
    local["payload"]["pricing"] = json!({"kind": "none_local"});
    assert!(validator.is_valid(&local));

    for kind in ["user_import", "user_declared"] {
        command["payload"]["pricing"] = json!({
            "kind": kind, "stale_policy": "warn", "entry": {
                "currency": "USD", "input_micros_per_million": 123,
                "cached_input_micros_per_million": null, "cache_write_micros_per_million": null,
                "output_micros_per_million": 456, "observed_at_ms": 1, "valid_until_ms": 2,
                "source_url": null
            }
        });
        assert!(validator.is_valid(&command));
        let mut tiered_command = command.clone();
        tiered_command["payload"]["pricing"]["entry"]["tiers"] = json!([]);
        assert!(
            !validator.is_valid(&tiered_command),
            "tarif inline avec paliers"
        );
        command["payload"]["pricing"]["entry"]["output_micros_per_million"] =
            json!(9_007_199_254_740_992_u64);
        assert!(!validator.is_valid(&command), "entier hors domaine exact");
    }
}

#[test]
fn test_validate_ipc_receipt_schema() {
    let validator = build_validator("ipc-receipt.schema.json");

    // Fixture F001/interfaces.md
    let stream_id = StreamId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000006"));
    let receipt = CommandReceipt {
        receipt_id: ReceiptId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000007")),
        command_id: CommandId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000002")),
        accepted_at: "2026-11-16T12:00:00Z"
            .parse::<chrono::DateTime<Utc>>()
            .unwrap(),
        aggregate_id: uuid::uuid!("00000000-0000-4000-8000-000000000004"),
        new_revision: Revision::new(9),
        operation_id: Some(OperationId::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000005"
        ))),
        state: ReceiptState::Accepted,
        event_cursor: EventCursor::new(stream_id, Sequence::new(125)),
        result_ref: None,
    };

    let receipt_val = serde_json::to_value(&receipt).unwrap();
    assert_eq!(
        receipt_val["event_cursor"]["stream_id"],
        "00000000-0000-4000-8000-000000000006"
    );
    assert!(
        validator.is_valid(&receipt_val),
        "Receipt F001 doit valider le schéma"
    );

    // Valider avec operation_id: null
    let mut null_op = receipt.clone();
    null_op.operation_id = None;
    let null_op_val = serde_json::to_value(&null_op).unwrap();
    assert_eq!(null_op_val["operation_id"], Value::Null);
    assert!(
        validator.is_valid(&null_op_val),
        "operation_id null doit valider le schéma"
    );

    // Inspection historique de reçu
    let inspection = CommandReceiptInspection {
        receipt,
        authority_generation: AuthorityGeneration::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000100"
        )),
        lineage_status: LineageStatus::Historical,
        provenance_ref: Some("backup:snap-40".to_string()),
    };
    let inspection_json = serde_json::to_string(&inspection).unwrap();
    let inspection_val: Value = serde_json::from_str(&inspection_json).unwrap();
    assert_eq!(inspection_val["lineage_status"], "historical");
    let decoded_inspection: CommandReceiptInspection =
        from_json_str_strict(&inspection_json).unwrap();
    assert_eq!(decoded_inspection, inspection);
}

#[test]
fn test_validate_ipc_error_schema() {
    let validator = build_validator("ipc-error.schema.json");

    let err = IpcError::from_reason(ReasonCode::StaleAuthorityGeneration, "Lignée invalide")
        .with_command_id(CommandId::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000002"
        )));

    let err_val = serde_json::to_value(&err).unwrap();
    assert!(
        err_val.get("current_revision").is_none(),
        "current_revision doit être omis quand None"
    );
    assert_eq!(
        err_val["details_ref"],
        Value::Null,
        "details_ref doit être présent comme null"
    );
    assert!(
        validator.is_valid(&err_val),
        "IpcError doit valider le schéma"
    );

    // Erreur avec conflit de révision
    let err_conflict = IpcError::from_reason(ReasonCode::PublicationConflict, "Conflit")
        .with_current_revision(Revision::new(42))
        .with_details_ref("ref:audit-trail-123");

    let err_conflict_val = serde_json::to_value(&err_conflict).unwrap();
    assert_eq!(err_conflict_val["current_revision"], 42);
    assert!(validator.is_valid(&err_conflict_val));

    // Cas négatif : current_revision: null rejeté par le schéma ET par from_json_str_strict
    let mut null_rev_val = err_val;
    null_rev_val["current_revision"] = Value::Null;
    assert!(
        !validator.is_valid(&null_rev_val),
        "current_revision: null doit être rejeté par ipc-error.schema.json"
    );
    let null_rev_res: Result<IpcError, _> =
        from_json_str_strict(&serde_json::to_string(&null_rev_val).unwrap());
    assert!(
        matches!(null_rev_res, Err(ProtocolError::Json(_))),
        "IpcError doit rejeter current_revision: null, obtenu : {null_rev_res:?}"
    );

    // Vérifier les 17 ErrorCode et 5 RetryClass
    for code in [
        ErrorCode::InvalidArgument,
        ErrorCode::AuthenticationFailed,
        ErrorCode::CapabilityDenied,
        ErrorCode::ScopeDenied,
        ErrorCode::RevisionConflict,
        ErrorCode::IdempotencyConflict,
        ErrorCode::NotFoundOrHidden,
        ErrorCode::ResourceBusy,
        ErrorCode::BudgetExceeded,
        ErrorCode::RateLimited,
        ErrorCode::ProtocolIncompatible,
        ErrorCode::StaleFence,
        ErrorCode::IsolationUnavailable,
        ErrorCode::ContractViolation,
        ErrorCode::OutcomeUnknown,
        ErrorCode::InternalFailure,
        ErrorCode::ResyncRequired,
    ] {
        let e = IpcError::new(code, None, "test", RetryClass::Never);
        let val = serde_json::to_value(&e).unwrap();
        assert!(
            validator.is_valid(&val),
            "ErrorCode::{code} doit être valide"
        );
    }
}

fn sample_event_envelope() -> EventEnvelope {
    EventEnvelope {
        event_id: uuid::uuid!("00000000-0000-4000-8000-000000000017"),
        stream_id: StreamId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000006")),
        sequence: Sequence::new(204),
        aggregate_id: uuid::uuid!("00000000-0000-4000-8000-000000000020"),
        aggregate_revision: Revision::new(3),
        event_type: "InvocationAdmitted".to_string(),
        schema_version: 1,
        occurred_at: "2026-11-16T12:00:00Z".parse::<DateTime<Utc>>().unwrap(),
        recorded_at: "2026-11-16T12:00:00Z".parse::<DateTime<Utc>>().unwrap(),
        project_id: Some(ProjectId::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000003"
        ))),
        actor_ref: "system:scheduler".to_string(),
        correlation_id: uuid::uuid!("00000000-0000-4000-8000-000000000004"),
        causation: EventCausation::from_command_receipt(ReceiptId::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000007"
        ))),
        command_id: Some(CommandId::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000002"
        ))),
        payload_hash: "sha256:396829e2a6811b428d99e673f23edf4797fd06a2b78736afd2b92ba42fc77cc7"
            .parse::<Sha256Digest>()
            .unwrap(),
        payload: json_obj(json!({
            "attempt_id": "00000000-0000-4000-8000-000000000021",
            "authority_generation": "00000000-0000-4000-8000-000000000018",
            "daemon_epoch": 12,
            "slot_reservation_id": "00000000-0000-4000-8000-000000000022"
        })),
        sensitivity: Sensitivity::Project,
    }
}

#[test]
fn test_validate_event_schema() {
    let validator = build_validator("event.schema.json");

    // Exemple InvocationAdmitted de F002/interfaces.md
    let event = sample_event_envelope();
    let event_val = serde_json::to_value(&event).unwrap();
    assert_eq!(
        event_val["stream_id"],
        "00000000-0000-4000-8000-000000000006"
    );
    assert!(
        validator.is_valid(&event_val),
        "InvocationAdmitted doit valider event.schema.json"
    );

    // Cas négatif : payload non-objet rejeté par le schéma ET par from_json_str_strict
    for invalid_payload in [
        json!([]),
        json!("texte"),
        json!(99),
        json!(false),
        Value::Null,
    ] {
        let mut bad_event = event_val.clone();
        bad_event["payload"] = invalid_payload;
        assert!(
            !validator.is_valid(&bad_event),
            "EventEnvelope doit refuser payload non-objet dans event.schema.json"
        );
        let res: Result<EventEnvelope, _> =
            from_json_str_strict(&serde_json::to_string(&bad_event).unwrap());
        assert!(
            matches!(res, Err(ProtocolError::Json(_))),
            "EventEnvelope doit rejeter tout payload non-objet, obtenu : {res:?}"
        );
    }
}

#[test]
fn test_validate_ipc_query_schemas() {
    let query_validator = build_validator("ipc-query.schema.json");

    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000001")),
        query_type: "GetAgent".to_string(),
        project_id: Some(ProjectId::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000003"
        ))),
        payload: json_obj(json!({ "agent_id": "00000000-0000-4000-8000-000000000050" })),
    };

    let query_val = serde_json::to_value(&query).unwrap();
    assert!(
        query_validator.is_valid(&query_val),
        "QueryEnvelope doit valider ipc-query.schema.json"
    );

    // Cas négatif : unleash.ipc.v1 refusé avec ProtocolIncompatible
    let mut query_v1 = query_val.clone();
    query_v1["protocol_version"] = json!("unleash.ipc.v1");
    assert!(
        !query_validator.is_valid(&query_v1),
        "QueryEnvelope v1 doit être refusé par ipc-query.schema.json"
    );
    let query_v1_res: Result<QueryEnvelope, _> =
        from_json_str_strict(&serde_json::to_string(&query_v1).unwrap());
    assert!(
        matches!(
            query_v1_res,
            Err(ProtocolError::ProtocolIncompatible { ref version }) if version == "unleash.ipc.v1"
        ),
        "QueryEnvelope v1 doit renvoyer ProtocolIncompatible, obtenu : {query_v1_res:?}"
    );

    // Cas négatif : payload non-objet rejeté par ipc-query.schema.json ET par from_json_str_strict
    for invalid_payload in [
        json!([]),
        json!("texte"),
        json!(1),
        json!(true),
        Value::Null,
    ] {
        let mut bad_query = query_val.clone();
        bad_query["payload"] = invalid_payload;
        assert!(
            !query_validator.is_valid(&bad_query),
            "QueryEnvelope doit refuser payload non-objet dans ipc-query.schema.json"
        );
        let res: Result<QueryEnvelope, _> =
            from_json_str_strict(&serde_json::to_string(&bad_query).unwrap());
        assert!(
            matches!(res, Err(ProtocolError::Json(_))),
            "QueryEnvelope doit rejeter tout payload non-objet, obtenu : {res:?}"
        );
    }

    let resp_validator = build_validator("ipc-query-response.schema.json");
    let stream_id = StreamId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000006"));

    let resp = QueryResponse {
        request_id: RequestId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000001")),
        watermark: Some(EventCursor::new(stream_id, Sequence::new(42))),
        result: json!({ "status": "active" }),
    };

    let resp_val = serde_json::to_value(&resp).unwrap();
    assert!(
        resp_validator.is_valid(&resp_val),
        "QueryResponse doit valider ipc-query-response.schema.json"
    );
}

#[test]
fn test_validate_ipc_hello_schema() {
    let validator = build_validator("ipc-hello.schema.json");

    let req = HelloRequest {
        protocol_version: ProtocolVersion,
        client_kind: ClientKind::Cli,
        client_version: "0.1.0".to_string(),
        capabilities: vec!["terminal".to_string(), "events".to_string()],
        admin_proof: None,
    };
    let req_val = serde_json::to_value(&req).unwrap();
    assert!(
        validator.is_valid(&req_val),
        "HelloRequest doit valider ipc-hello.schema.json"
    );

    let mut with_proof = req.clone();
    with_proof.admin_proof = Some(unleash_protocol::AdminProof::from_bytes([42; 32]));
    let valid_proof = serde_json::to_value(&with_proof).unwrap();
    assert!(validator.is_valid(&valid_proof));
    let decoded: HelloRequest = from_json_str_strict(&valid_proof.to_string()).unwrap();
    assert!(decoded.admin_proof.is_some());
    let mut null_proof = req_val.clone();
    null_proof["admin_proof"] = Value::Null;
    assert!(validator.is_valid(&null_proof));
    let decoded_null: HelloRequest = from_json_str_strict(&null_proof.to_string()).unwrap();
    assert!(decoded_null.admin_proof.is_none());
    for invalid in [String::from("abc"), "AA".repeat(32), "gg".repeat(32)] {
        let mut invalid_proof = req_val.clone();
        invalid_proof["admin_proof"] = json!(invalid);
        assert!(!validator.is_valid(&invalid_proof));
        assert!(from_json_str_strict::<HelloRequest>(&invalid_proof.to_string()).is_err());
    }

    let mut req_v1 = req_val.clone();
    req_v1["protocol_version"] = json!("unleash.ipc.v1");
    assert!(
        !validator.is_valid(&req_v1),
        "HelloRequest v1 doit être refusé par ipc-hello.schema.json"
    );
    let req_v1_res: Result<HelloRequest, _> =
        from_json_str_strict(&serde_json::to_string(&req_v1).unwrap());
    assert!(
        matches!(
            req_v1_res,
            Err(ProtocolError::ProtocolIncompatible { ref version }) if version == "unleash.ipc.v1"
        ),
        "HelloRequest v1 doit renvoyer ProtocolIncompatible, obtenu : {req_v1_res:?}"
    );

    let resp = HelloResponse {
        protocol_version: ProtocolVersion,
        authority_generation: AuthorityGeneration::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000100"
        )),
        daemon_epoch: DaemonEpoch::new(1),
        daemon_instance_id: DaemonInstanceId::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000200"
        )),
        stream_id: StreamId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000006")),
        max_frame_bytes: 8_388_608,
        suggested_segment_bytes: 1_048_576,
        idempotency_retention_seconds: 604_800,
        capabilities: vec!["core_v2".to_string()],
    };
    let resp_val = serde_json::to_value(&resp).unwrap();
    assert_eq!(
        resp_val["stream_id"],
        "00000000-0000-4000-8000-000000000006"
    );
    assert!(
        validator.is_valid(&resp_val),
        "HelloResponse doit valider ipc-hello.schema.json"
    );

    let mut resp_v1 = resp_val.clone();
    resp_v1["protocol_version"] = json!("unleash.ipc.v1");
    assert!(
        !validator.is_valid(&resp_v1),
        "HelloResponse v1 doit être refusé par ipc-hello.schema.json"
    );
    let resp_v1_res: Result<HelloResponse, _> =
        from_json_str_strict(&serde_json::to_string(&resp_v1).unwrap());
    assert!(
        matches!(
            resp_v1_res,
            Err(ProtocolError::ProtocolIncompatible { ref version }) if version == "unleash.ipc.v1"
        ),
        "HelloResponse v1 doit renvoyer ProtocolIncompatible, obtenu : {resp_v1_res:?}"
    );
}

#[test]
fn test_validate_ipc_subscribe_events_schema() {
    let schema = load_schema("ipc-subscribe-events.schema.json");
    let event_schema = load_schema("event.schema.json");

    // Construire le validateur en enregistrant event.schema.json sous son URN dans le registry local
    let registry = referencing::Registry::new()
        .add("urn:ovpt:unleash:event:1", event_schema)
        .expect("Ajout event.schema.json au registry")
        .prepare()
        .expect("Préparation du registry");

    let validator = jsonschema::options()
        .should_validate_formats(true)
        .with_registry(&registry)
        .build(&schema)
        .expect("Validateur ipc-subscribe-events avec schéma event lié");

    let stream_id = StreamId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000006"));
    let sub_req = SubscribeEventsRequest {
        stream_id,
        after_sequence: Sequence::new(10),
        project_id: None,
        aggregate_id: None,
        batch_limit: 100,
    };
    let sub_req_val = serde_json::to_value(&sub_req).unwrap();
    assert_eq!(
        sub_req_val["stream_id"],
        "00000000-0000-4000-8000-000000000006"
    );
    assert!(
        validator.is_valid(&sub_req_val),
        "SubscribeEventsRequest doit valider"
    );

    let events_frame = SubscribeEventsFrame::Events {
        events: vec![sample_event_envelope()],
        cursor: EventCursor::new(stream_id, Sequence::new(204)),
    };
    let events_val = serde_json::to_value(&events_frame).unwrap();
    assert_eq!(events_val["kind"], "events");
    assert!(
        validator.is_valid(&events_val),
        "Events frame doit valider ipc-subscribe-events.schema.json"
    );

    let resync_frame = SubscribeEventsFrame::ResyncRequired {
        stream_id,
        current_watermark: EventCursor::new(stream_id, Sequence::new(150)),
        reason: "Cursor expired".to_string(),
    };
    let resync_val = serde_json::to_value(&resync_frame).unwrap();
    assert_eq!(resync_val["kind"], "resync_required");
    assert_eq!(
        resync_val["stream_id"],
        "00000000-0000-4000-8000-000000000006"
    );
    assert!(
        validator.is_valid(&resync_val),
        "ResyncRequired frame doit valider"
    );
}

#[test]
fn test_validate_ipc_client_upload_segment_schema() {
    let validator = build_validator("ipc-client-upload-segment.schema.json");
    let command_id = CommandId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000099"));
    let raw_payload = b"row_id,item_key,quantity\nr01,SYNTHETIC-APPLE-1KG,2\n";

    // 1. Cas positif nominal : construction, validation schéma, round-trip JSON strict, décodage
    let segment = ClientUploadSegment::from_bytes(command_id, 0, 2, 0, raw_payload)
        .expect("Segment client_upload valide");
    let segment_json = serde_json::to_string(&segment).expect("Sérialisation segment");
    let segment_val: Value = serde_json::from_str(&segment_json).unwrap();
    assert!(
        validator.is_valid(&segment_val),
        "ClientUploadSegment valide doit passer ipc-client-upload-segment.schema.json"
    );
    let roundtrip: ClientUploadSegment =
        from_json_str_strict(&segment_json).expect("Désérialisation stricte segment");
    assert_eq!(roundtrip, segment);
    assert_eq!(
        roundtrip
            .decode_and_validate()
            .expect("Validation sémantique et SHA-256"),
        raw_payload
    );

    // 2. Cas positif limite : segment vide 0 octet (1 seul segment)
    let empty_seg = ClientUploadSegment::from_bytes(command_id, 0, 1, 0, b"")
        .expect("Segment vide 0 octet admis");
    let empty_val = serde_json::to_value(&empty_seg).unwrap();
    assert!(
        validator.is_valid(&empty_val),
        "Segment vide doit valider ipc-client-upload-segment.schema.json"
    );
    assert_eq!(empty_seg.decode_and_validate().unwrap(), b"");

    // 3. Rejet : total_segments == 0 (rejeté par schéma ET par validation Rust)
    let mut zero_total = segment.clone();
    zero_total.total_segments = 0;
    let zero_total_val = serde_json::to_value(&zero_total).unwrap();
    assert!(
        !validator.is_valid(&zero_total_val),
        "total_segments == 0 doit être rejeté par le schéma"
    );
    assert!(matches!(
        zero_total.validate(),
        Err(ProtocolError::Validation(_))
    ));

    // 4. Rejet : segment_index >= total_segments
    let mut bad_index = segment.clone();
    bad_index.segment_index = 2;
    bad_index.total_segments = 2;
    assert!(matches!(
        bad_index.validate(),
        Err(ProtocolError::Validation(_))
    ));

    // 5. Rejet : digest SHA-256 mismatch
    let mut bad_digest = segment.clone();
    bad_digest.sha256 = Sha256Digest::from_bytes(b"different content");
    assert!(
        validator.is_valid(&serde_json::to_value(&bad_digest).unwrap()),
        "Le schéma seul valide le format sha256 mais pas l'empreinte métier"
    );
    assert!(matches!(
        bad_digest.decode_and_validate(),
        Err(ProtocolError::Validation(_))
    ));

    // 6. Rejet : byte_length mismatch vs contenu décodé
    let mut bad_len = segment.clone();
    bad_len.byte_length += 1;
    assert!(matches!(
        bad_len.validate(),
        Err(ProtocolError::Validation(_))
    ));

    // 7. Rejet : base64 invalide (rejeté par schéma ET par validation Rust)
    let mut bad_b64 = segment.clone();
    bad_b64.data_base64 = "not!valid!base64".to_string();
    assert!(
        !validator.is_valid(&serde_json::to_value(&bad_b64).unwrap()),
        "data_base64 invalide doit être rejeté par le schéma"
    );
    assert!(matches!(
        bad_b64.validate(),
        Err(ProtocolError::Validation(_))
    ));

    // 8. Rejet : dépassement du plafond d'import de 10 MiB
    let mut exceeds_import_limit = segment;
    exceeds_import_limit.byte_offset = MAX_CLIENT_UPLOAD_BYTES;
    assert!(matches!(
        exceeds_import_limit.validate(),
        Err(ProtocolError::Validation(_))
    ));

    // 9. Rejet : format sha256 invalide (rejeté par schéma ET par from_json_str_strict)
    let mut bad_sha_fmt = segment_val.clone();
    bad_sha_fmt["sha256"] = json!("not-a-sha256");
    assert!(!validator.is_valid(&bad_sha_fmt));
    let bad_sha_res: Result<ClientUploadSegment, _> =
        from_json_str_strict(&serde_json::to_string(&bad_sha_fmt).unwrap());
    assert!(matches!(bad_sha_res, Err(ProtocolError::Json(_))));

    // 10. Rejet : command_id non-UUID et champ inconnu
    let mut bad_cmd_id = segment_val.clone();
    bad_cmd_id["command_id"] = json!("invalid-uuid");
    assert!(!validator.is_valid(&bad_cmd_id));

    let mut unknown_prop = segment_val;
    unknown_prop["unexpected"] = json!(123);
    assert!(!validator.is_valid(&unknown_prop));
    let unknown_res: Result<ClientUploadSegment, _> =
        from_json_str_strict(&serde_json::to_string(&unknown_prop).unwrap());
    assert!(matches!(unknown_res, Err(ProtocolError::Json(_))));
}
