//! REQ-CORE-004 : preuve administrative typée, format fermé et Debug masqué.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use unleash_protocol::AdminProof;

#[test]
fn admin_proof_is_fixed_length_strict_and_debug_redacted() {
    let proof = AdminProof::from_bytes([42; 32]);
    let json = serde_json::to_string(&proof).unwrap();
    assert_eq!(json.len(), 66);
    assert!(!format!("{proof:?}").contains(&"2a".repeat(32)));
    let same: AdminProof = serde_json::from_str(&json).unwrap();
    assert!(proof.matches(&same));
    assert!(!proof.matches(&AdminProof::from_bytes([43; 32])));
    assert!(serde_json::from_str::<AdminProof>("\"abcd\"").is_err());
    assert!(serde_json::from_str::<AdminProof>(&format!("\"{}\"", "AA".repeat(32))).is_err());
}
