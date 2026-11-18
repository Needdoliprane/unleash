//! Identité durable d'une évaluation d'autorisation, distincte d'un `RequestId` de transport.
use unleash_domain::AuthorizationEvaluationId;

#[test]
fn authorization_evaluation_identity_roundtrips_and_rejects_invalid_uuid() {
    let evaluation = AuthorizationEvaluationId::from_u128(0xA117_0001);
    let encoded = serde_json::to_string(&evaluation).unwrap();
    let recovered: AuthorizationEvaluationId = serde_json::from_str(&encoded).unwrap();
    assert_eq!(evaluation, recovered);
    assert_eq!(evaluation.as_uuid(), recovered.as_uuid());
    assert!(serde_json::from_str::<AuthorizationEvaluationId>("\"not-a-uuid\"").is_err());
}
