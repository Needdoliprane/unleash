//! T-F012-013/014/018: forme wire, sans admission ni appel fournisseur.
use serde_json::{Value, json};
use unleash_domain::{DataLocality, MicrosAmount};
use unleash_protocol::{
    CommandEnvelope, ProviderTestLimits, ProviderTestPhase, TestProviderProfilePayload,
    from_json_str_strict,
};

fn fixture() -> Value {
    json!({
        "profile_id": "fixture-profile", "profile_version": 7,
        "phase": "generation_probe", "capability_set": ["text", "stream"],
        "limits": {"timeout_ms": 60000, "max_calls": 3, "max_output_tokens": 512,
                   "max_cost_usd_micros": 100_000},
        "locality": "approved_remote", "mandate_ref": "fixture-mandate"
    })
}

fn decode(value: &Value) -> Result<TestProviderProfilePayload, serde_json::Error> {
    serde_json::from_value(value.clone())
}

fn set(value: &mut Value, field: &str, replacement: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(field.to_owned(), replacement);
    } else {
        assert!(value.is_object(), "fixture object");
    }
}

fn set_limit(value: &mut Value, field: &str, replacement: Value) {
    if let Some(limits) = value.get_mut("limits") {
        set(limits, field, replacement);
    } else {
        assert!(value.get("limits").is_some(), "fixture limits");
    }
}

#[test]
fn phases_and_exact_bounds_round_trip() {
    for phase in [
        "validate_config",
        "connect",
        "inventory",
        "generation_probe",
    ] {
        let mut value = fixture();
        set(&mut value, "phase", json!(phase));
        let payload = decode(&value).expect("valid payload");
        assert_eq!(serde_json::to_value(payload).unwrap(), value);
    }
    let mut local = fixture();
    set(&mut local, "locality", json!("local_only"));
    set_limit(&mut local, "max_cost_usd_micros", json!(0));
    assert!(decode(&local).is_ok());
    let mut reduced = fixture();
    set(
        &mut reduced,
        "limits",
        json!({"timeout_ms":1,"max_calls":1,"max_output_tokens":1,"max_cost_usd_micros":1}),
    );
    assert!(decode(&reduced).is_ok());
    // Les plafonds de fixture s'appliquent à la génération, pas aux phases passives.
    let mut passive = fixture();
    set(&mut passive, "phase", json!("connect"));
    set(
        &mut passive,
        "limits",
        json!({"timeout_ms":70_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0}),
    );
    assert!(decode(&passive).is_ok());
}

#[test]
fn unknown_fields_and_authority_injection_are_rejected() {
    for field in [
        "actor",
        "actor_ref",
        "principal_id",
        "origin_kind",
        "source_run_id",
        "authorization",
        "effective_scope",
        "unexpected",
    ] {
        let mut value = fixture();
        set(&mut value, field, json!("injected"));
        assert!(decode(&value).is_err(), "field {field}");
    }
    let mut duplicate_capability = fixture();
    set(
        &mut duplicate_capability,
        "capability_set",
        json!(["text", "text"]),
    );
    assert!(decode(&duplicate_capability).is_err());
    let mut value = fixture();
    set_limit(&mut value, "authority", json!(true));
    assert!(decode(&value).is_err());
    let duplicate = fixture().to_string().replace(
        "\"phase\":\"generation_probe\"",
        "\"phase\":\"generation_probe\",\"phase\":\"connect\"",
    );
    assert!(from_json_str_strict::<TestProviderProfilePayload>(&duplicate).is_err());
}

#[test]
fn invalid_enums_identifiers_and_generation_limits_are_rejected() {
    for (field, invalid) in [
        ("phase", json!("other")),
        ("locality", json!("remote")),
        ("profile_id", json!("  ")),
        ("profile_version", json!(0)),
        ("mandate_ref", json!("")),
    ] {
        let mut value = fixture();
        set(&mut value, field, invalid);
        assert!(decode(&value).is_err(), "field {field}");
    }
    for (field, invalid) in [
        ("timeout_ms", json!(0)),
        ("timeout_ms", json!(60001)),
        ("max_calls", json!(0)),
        ("max_calls", json!(4)),
        ("max_output_tokens", json!(0)),
        ("max_output_tokens", json!(513)),
        ("max_cost_usd_micros", json!(100_001)),
        ("max_calls", json!(-1)),
        ("max_cost_usd_micros", json!(0.1)),
    ] {
        let mut value = fixture();
        set_limit(&mut value, field, invalid);
        assert!(decode(&value).is_err(), "limits.{field}");
    }
}

#[test]
fn existing_command_envelope_stays_unchanged() {
    let envelope = json!({"protocol_version":"unleash.ipc.v2", "request_id":"00000000-0000-0000-0000-000000000001", "command_id":"00000000-0000-0000-0000-000000000002", "authority_generation":"00000000-0000-0000-0000-000000000003", "actor_ref":null, "project_id":null, "aggregate_id":"00000000-0000-0000-0000-000000000004", "expected_revision":0, "command_type":"TestProviderProfile", "payload":fixture()});
    let decoded: CommandEnvelope = from_json_str_strict(&envelope.to_string()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), envelope);
    let mut previous = envelope;
    set(&mut previous, "command_type", json!("StopDaemon"));
    set(&mut previous, "payload", json!({"reason":"fixture"}));
    let decoded: CommandEnvelope = from_json_str_strict(&previous.to_string()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), previous);
}

#[test]
fn typed_constructor_enforces_the_same_contract_as_decoding() {
    let limits = ProviderTestLimits::new(60_000, 3, 512, MicrosAmount::new(100_000));
    let request = TestProviderProfilePayload::try_new(
        "fixture-profile".to_owned(),
        7,
        ProviderTestPhase::GenerationProbe,
        vec!["text".to_owned(), "stream".to_owned()],
        limits.clone(),
        DataLocality::ApprovedRemote,
        "fixture-mandate".to_owned(),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), fixture());
    assert!(
        TestProviderProfilePayload::try_new(
            "fixture-profile".to_owned(),
            0,
            ProviderTestPhase::GenerationProbe,
            vec![],
            limits,
            DataLocality::LocalOnly,
            "fixture-mandate".to_owned(),
        )
        .is_err()
    );
}
