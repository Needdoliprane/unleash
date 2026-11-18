//! T-F012-011/013/014/018 : configuration wire, sans effet ni preuve d'autorité.
use serde_json::{Value, json};
use unleash_protocol::{SetProviderProfilePayload, from_json_str_strict};

fn remote() -> Value {
    json!({"provider_kind":"openai", "dialect":{"kind":"openai_responses","version":1},
        "endpoint":"https://fixture.invalid/v1/responses", "model":"fixture-model",
        "locality":"approved_remote", "server_management":"user_managed",
        "auth":{"kind":"secret_ref", "secret_ref":"fixture:opaque"},
        "limits":{"max_inflight":1,"connect_timeout_ms":2000,"queue_timeout_ms":10000,
                  "load_timeout_ms":180_000,"first_token_timeout_ms":60000,"generation_timeout_ms":300_000},
        "declared_capabilities":["text","stream"],
        "pricing":{"kind":"catalog", "catalog_id":"fixture-catalog", "catalog_version":"1",
                   "catalog_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                   "entry_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                   "stale_policy":"block"}})
}

fn entry() -> Value {
    json!({"currency":"USD","input_micros_per_million":123,"cached_input_micros_per_million":null,
        "cache_write_micros_per_million":null,"output_micros_per_million":456,
        "observed_at_ms":1,"valid_until_ms":2,"source_url":null})
}

fn set(value: &mut Value, key: &str, replacement: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.into(), replacement);
    } else {
        assert!(value.is_object(), "fixture object");
    }
}

fn nested(value: &mut Value, parent: &str, key: &str, replacement: Value) {
    if let Some(object) = value.get_mut(parent) {
        set(object, key, replacement);
    } else {
        assert!(value.get(parent).is_some(), "fixture parent");
    }
}

fn decode(value: &Value) -> Result<SetProviderProfilePayload, serde_json::Error> {
    serde_json::from_value(value.clone())
}

#[test]
fn exact_remote_local_and_pricing_sources_round_trip() {
    let r = remote();
    assert_eq!(serde_json::to_value(decode(&r).unwrap()).unwrap(), r);
    let mut local = remote();
    set(&mut local, "provider_kind", json!("ollama"));
    set(
        &mut local,
        "dialect",
        json!({"kind":"ollama_chat","version":1}),
    );
    set(
        &mut local,
        "endpoint",
        json!("http://127.0.0.1:11434/api/chat"),
    );
    set(&mut local, "locality", json!("local_only"));
    set(&mut local, "auth", json!({"kind":"none"}));
    set(&mut local, "pricing", json!({"kind":"none_local"}));
    assert_eq!(
        serde_json::to_value(decode(&local).unwrap()).unwrap(),
        local
    );
    set(&mut local, "endpoint", json!("http://[::1]:11434/api/chat"));
    assert!(decode(&local).is_ok());
    for kind in ["user_import", "user_declared"] {
        let mut value = remote();
        set(
            &mut value,
            "pricing",
            json!({"kind":kind,"entry":entry(),"stale_policy":"warn"}),
        );
        assert_eq!(
            serde_json::to_value(decode(&value).unwrap()).unwrap(),
            value
        );
    }
}

#[test]
fn rejects_extra_fields_origin_secrets_and_bad_unions() {
    for field in [
        "profile_id",
        "profile_version",
        "actor",
        "principal_id",
        "scope",
        "state",
        "phase",
        "receipt_id",
        "profile_digest",
        "placement_observed",
        "unknown",
    ] {
        let mut value = remote();
        set(&mut value, field, json!("forged"));
        assert!(decode(&value).is_err(), "{field}");
    }
    for (parent, field) in [
        ("auth", "token"),
        ("auth", "value"),
        ("dialect", "endpoint"),
        ("limits", "authority"),
        ("pricing", "trusted"),
    ] {
        let mut value = remote();
        nested(&mut value, parent, field, json!("forged"));
        assert!(decode(&value).is_err(), "{parent}.{field}");
    }
    for bad in [
        json!({"kind":"none","secret_ref":"fixture:opaque"}),
        json!({"kind":"secret_ref","secret_ref":""}),
        json!({"kind":"bearer","secret":"fixture"}),
        json!({"kind":"other"}),
    ] {
        let mut value = remote();
        set(&mut value, "auth", bad);
        assert!(decode(&value).is_err());
    }
    let duplicate = remote().to_string().replace(
        "\"provider_kind\":\"openai\"",
        "\"provider_kind\":\"openai\",\"provider_kind\":\"ollama\"",
    );
    assert!(from_json_str_strict::<SetProviderProfilePayload>(&duplicate).is_err());
}

#[test]
fn rejects_invalid_pairs_endpoints_and_inexact_integers() {
    for (field, bad) in [
        ("provider_kind", json!("other")),
        ("endpoint", json!("https://user:pass@fixture.invalid/v1")),
        ("endpoint", json!("https://fixture.invalid/v1?token=secret")),
        ("endpoint", json!("file:///tmp/model")),
        ("model", json!("  ")),
    ] {
        let mut value = remote();
        set(&mut value, field, bad);
        assert!(decode(&value).is_err(), "{field}");
    }
    let mut value = remote();
    nested(&mut value, "dialect", "kind", json!("anthropic_messages"));
    assert!(decode(&value).is_err());
    for bad in [0_u64, 9_007_199_254_740_992] {
        let mut value = remote();
        nested(&mut value, "dialect", "version", json!(bad));
        assert!(decode(&value).is_err());
        let mut value = remote();
        nested(&mut value, "limits", "max_inflight", json!(bad));
        assert!(decode(&value).is_err());
    }
    let mut value = remote();
    set(
        &mut value,
        "pricing",
        json!({"kind":"user_declared","entry":{"currency":"USD","input_micros_per_million":9_007_199_254_740_992_u64,"cached_input_micros_per_million":null,"cache_write_micros_per_million":null,"output_micros_per_million":1,"observed_at_ms":1,"valid_until_ms":2,"source_url":null},"stale_policy":"block"}),
    );
    assert!(decode(&value).is_err());
}

#[test]
fn endpoint_accepts_valid_percent_encoding_and_rejects_malformed_escapes() {
    let mut value = remote();
    set(
        &mut value,
        "endpoint",
        json!("https://fixture.invalid/route%20part"),
    );
    assert!(decode(&value).is_ok());
    for path in ["/route%", "/route%2", "/route%zz"] {
        set(
            &mut value,
            "endpoint",
            json!(format!("https://fixture.invalid{path}")),
        );
        assert!(decode(&value).is_err(), "{path}");
    }
}

#[test]
fn all_supported_dialect_pairs_are_distinct_and_exact() {
    for (provider, dialect) in [
        ("openai", "openai_responses"),
        ("anthropic", "anthropic_messages"),
        ("lmstudio", "openai_chat_completions"),
        ("ollama", "ollama_chat"),
        ("llama_cpp", "openai_chat_completions"),
        ("openai_compatible", "openai_chat_completions"),
        ("openai_compatible", "openai_responses"),
    ] {
        let mut value = remote();
        set(&mut value, "provider_kind", json!(provider));
        nested(&mut value, "dialect", "kind", json!(dialect));
        assert!(decode(&value).is_ok(), "{provider}/{dialect}");
    }
}

#[test]
fn inline_tiers_are_rejected_and_unknown_rates_remain_null() {
    let mut value = remote();
    let mut price = entry();
    set(&mut price, "tiers", json!([{"threshold":1,"price":2}]));
    set(
        &mut value,
        "pricing",
        json!({"kind":"user_import","entry":price,"stale_policy":"block"}),
    );
    assert!(decode(&value).is_err());
    let mut unknown = entry();
    set(&mut unknown, "input_micros_per_million", Value::Null);
    set(
        &mut value,
        "pricing",
        json!({"kind":"user_declared","entry":unknown,"stale_policy":"block"}),
    );
    assert_eq!(
        serde_json::to_value(decode(&value).unwrap()).unwrap(),
        value
    );
}

#[test]
fn exact_integer_range_applies_to_each_wire_number() {
    const MAX: u64 = 9_007_199_254_740_991;
    for field in [
        "max_inflight",
        "connect_timeout_ms",
        "queue_timeout_ms",
        "load_timeout_ms",
        "first_token_timeout_ms",
        "generation_timeout_ms",
    ] {
        let mut value = remote();
        nested(&mut value, "limits", field, json!(MAX));
        assert!(decode(&value).is_ok(), "{field} maximum");
        nested(&mut value, "limits", field, json!(MAX + 1));
        assert!(decode(&value).is_err(), "{field} overflow");
    }
    for field in [
        "input_micros_per_million",
        "cached_input_micros_per_million",
        "cache_write_micros_per_million",
        "output_micros_per_million",
    ] {
        let mut price = entry();
        set(&mut price, field, json!(MAX));
        let mut value = remote();
        set(
            &mut value,
            "pricing",
            json!({"kind":"user_import","entry":price,"stale_policy":"block"}),
        );
        assert!(decode(&value).is_ok(), "{field} maximum");
        let mut price = entry();
        set(&mut price, field, json!(MAX + 1));
        set(
            &mut value,
            "pricing",
            json!({"kind":"user_import","entry":price,"stale_policy":"block"}),
        );
        assert!(decode(&value).is_err(), "{field} overflow");
    }
}

#[test]
fn unencrypted_remote_and_local_price_on_native_cloud_are_refused() {
    let mut value = remote();
    set(
        &mut value,
        "endpoint",
        json!("http://fixture.invalid/v1/responses"),
    );
    assert!(decode(&value).is_err());
    let mut value = remote();
    set(&mut value, "pricing", json!({"kind":"none_local"}));
    assert!(decode(&value).is_err());
}
