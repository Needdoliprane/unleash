//! REQ-MODEL-013 : dérivation syntaxique uniquement, sans DNS ni réseau.
#![expect(
    clippy::unwrap_used,
    reason = "Payloads synthétiques et assertions de contrat"
)]

use serde_json::{Value, json};
use unleash_protocol::{SetProviderProfilePayload, canonical_sha256};

fn wire(endpoint: &str) -> Value {
    json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":endpoint,"model":"synthetic-endpoint","locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2000,"queue_timeout_ms":1000,"load_timeout_ms":180000,"first_token_timeout_ms":60000,"generation_timeout_ms":60000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}})
}

fn payload(endpoint: &str) -> SetProviderProfilePayload {
    serde_json::from_value(wire(endpoint)).unwrap()
}

#[test]
fn endpoint_destination_default_and_explicit_ports() {
    for (endpoint, scheme, host, port) in [
        ("http://127.0.0.1/v1", "http", "127.0.0.1", 80),
        ("https://localhost/v1", "https", "localhost", 443),
        ("http://127.0.0.1:8080/v1", "http", "127.0.0.1", 8080),
        (
            "https://example.invalid:9443/v1",
            "https",
            "example.invalid",
            9443,
        ),
    ] {
        let derived = payload(endpoint).endpoint_destination().unwrap();
        assert_eq!(
            (derived.scheme(), derived.host(), derived.port()),
            (scheme, host, port)
        );
    }
}

#[test]
fn endpoint_destination_ipv4_and_ipv6_are_literals_not_dns_proof() {
    for (endpoint, host) in [
        ("http://127.0.0.1:8080/v1", "127.0.0.1"),
        ("https://[::1]:9443/v1", "::1"),
    ] {
        let derived = payload(endpoint).endpoint_destination().unwrap();
        assert_eq!(derived.host(), host);
        assert!(derived.is_loopback_literal());
    }
    let local_name = payload("https://localhost/v1")
        .endpoint_destination()
        .unwrap();
    assert!(
        !local_name.is_loopback_literal(),
        "a name is not an observed address"
    );
}

#[test]
fn endpoint_destination_keeps_original_wire_digest_and_path() {
    let original = payload("http://LOCALHOST:80/v1/a%20b");
    let before = canonical_sha256(&original).unwrap();
    let derived = original.endpoint_destination().unwrap();
    assert_eq!(derived.normalized_endpoint(), "http://localhost/v1/a%20b");
    assert_eq!(derived.host(), "localhost");
    assert_eq!(derived.port(), 80);
    assert_eq!(original.endpoint(), "http://LOCALHOST:80/v1/a%20b");
    assert_eq!(canonical_sha256(&original).unwrap(), before);
}

#[test]
fn endpoint_destination_does_not_relax_endpoint_rejections() {
    for endpoint in [
        "http://user:password@localhost/v1",
        "https://localhost/v1?key=value",
        "https://localhost/v1#fragment",
        "ftp://localhost/v1",
        "https://localhost:0/v1",
    ] {
        assert!(serde_json::from_value::<SetProviderProfilePayload>(wire(endpoint)).is_err());
    }
}
