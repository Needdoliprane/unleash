//! Tests de conformité RFC 8785 (JCS) et de calcul du digest canonique d'idempotence v2.

#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation
)]

use serde_json::json;
use unleash_domain::{
    AuthorityGeneration, CommandId, ProjectId, RequestId, Revision, Sha256Digest,
};
use unleash_protocol::{
    CommandEnvelope, ProtocolVersion, canonical_sha256, canonicalize_to_string,
    compute_command_digest_v2,
};

#[test]
fn test_rfc8785_canonicalization_vectors() {
    // 1. Tri lexicographique UTF-16 code units des clés
    let val = json!({
        "b": 1,
        "a": 2,
        "1": 3,
        "A": 4
    });
    // En ASCII/UTF-16 : "1" (0x31) < "A" (0x41) < "a" (0x61) < "b" (0x62)
    let s = canonicalize_to_string(&val).expect("JCS doit réussir");
    assert_eq!(s, "{\"1\":3,\"A\":4,\"a\":2,\"b\":1}");

    // 2. Échappements stricts (seuls \", \\, et caractères de contrôle < 0x20 sont échappés ; / n'est pas échappé)
    let val_escapes = json!({
        "slash": "https://example.com/api/test",
        "special": "line1\nline2\t\"quoted\"\\backslash"
    });
    let s_escapes = canonicalize_to_string(&val_escapes).expect("JCS échappements");
    assert_eq!(
        s_escapes,
        "{\"slash\":\"https://example.com/api/test\",\"special\":\"line1\\nline2\\t\\\"quoted\\\"\\\\backslash\"}"
    );

    // 3. Nombres entiers et flottants conformes ES6/JCS
    let val_numbers = json!({
        "int": 42,
        "zero": 0,
        "neg": -100
    });
    let s_numbers = canonicalize_to_string(&val_numbers).expect("JCS nombres");
    assert_eq!(s_numbers, "{\"int\":42,\"neg\":-100,\"zero\":0}");
}

#[test]
fn test_invocation_admitted_exact_payload_hash() {
    // Exemple normatif de F002/interfaces.md :
    // "payload": {
    //   "attempt_id": "00000000-0000-4000-8000-000000000021",
    //   "authority_generation": "00000000-0000-4000-8000-000000000018",
    //   "daemon_epoch": 12,
    //   "slot_reservation_id": "00000000-0000-4000-8000-000000000022"
    // }
    // payload_hash attendu : sha256:396829e2a6811b428d99e673f23edf4797fd06a2b78736afd2b92ba42fc77cc7

    // Clés insérées volontairement dans le désordre pour vérifier le tri JCS
    let payload = json!({
        "slot_reservation_id": "00000000-0000-4000-8000-000000000022",
        "daemon_epoch": 12,
        "authority_generation": "00000000-0000-4000-8000-000000000018",
        "attempt_id": "00000000-0000-4000-8000-000000000021"
    });

    let canonical_str = canonicalize_to_string(&payload).expect("JCS payload");
    assert_eq!(
        canonical_str,
        "{\"attempt_id\":\"00000000-0000-4000-8000-000000000021\",\"authority_generation\":\"00000000-0000-4000-8000-000000000018\",\"daemon_epoch\":12,\"slot_reservation_id\":\"00000000-0000-4000-8000-000000000022\"}"
    );

    let digest: Sha256Digest = canonical_sha256(&payload).expect("Calcul digest");
    assert_eq!(
        digest.as_prefixed_str(),
        "sha256:396829e2a6811b428d99e673f23edf4797fd06a2b78736afd2b92ba42fc77cc7",
        "Le digest SHA-256 canonique du payload InvocationAdmitted doit être exactement égal à la référence de F002/interfaces.md"
    );
}

#[test]
fn test_compute_command_digest_v2_stability_and_sensitivity() {
    let base_envelope = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000001")),
        command_id: CommandId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-000000000002")),
        authority_generation: AuthorityGeneration::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000100"
        )),
        actor_ref: Some("client:original-actor".to_string()),
        project_id: Some(ProjectId::from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-000000000003"
        ))),
        aggregate_id: uuid::uuid!("00000000-0000-4000-8000-000000000004"),
        expected_revision: Revision::new(8),
        command_type: "PauseMission".to_string(),
        payload: json!({
            "reason": "inspection utilisateur",
            "metadata": { "b": 2, "a": 1 }
        }),
    };

    let base_digest = compute_command_digest_v2(&base_envelope).expect("Digest original");

    // 1. Stabilité par rapport à request_id (changement lors d'un retry)
    let mut retry_envelope = base_envelope.clone();
    retry_envelope.request_id =
        RequestId::from_uuid(uuid::uuid!("99999999-9999-4999-8999-999999999999"));
    let retry_digest = compute_command_digest_v2(&retry_envelope).expect("Digest retry");
    assert_eq!(
        base_digest, retry_digest,
        "Le digest doit être STRICTEMENT invariant par rapport à request_id"
    );

    // 2. Stabilité par rapport à actor_ref (résolu par le serveur, non autoritaire dans le wire)
    let mut different_actor_envelope = base_envelope.clone();
    different_actor_envelope.actor_ref = Some("client:other-actor".to_string());
    assert_eq!(
        base_digest,
        compute_command_digest_v2(&different_actor_envelope).unwrap(),
        "Le digest doit être STRICTEMENT invariant par rapport à actor_ref"
    );
    different_actor_envelope.actor_ref = None;
    assert_eq!(
        base_digest,
        compute_command_digest_v2(&different_actor_envelope).unwrap(),
        "Le digest doit être STRICTEMENT invariant que actor_ref soit Some ou None"
    );

    // 3. Stabilité par rapport à l'ordre des clés dans le payload JSON (normalisation JCS)
    let mut reordered_payload_envelope = base_envelope.clone();
    reordered_payload_envelope.payload = json!({
        "metadata": { "a": 1, "b": 2 },
        "reason": "inspection utilisateur"
    });
    assert_eq!(
        base_digest,
        compute_command_digest_v2(&reordered_payload_envelope).unwrap(),
        "Le digest doit être identique quel que soit l'ordre des clés du payload"
    );

    // 4. Sensibilité aux champs structurants : toute variation doit modifier le digest
    // 4a. authority_generation
    let mut diff_gen = base_envelope.clone();
    diff_gen.authority_generation =
        AuthorityGeneration::from_uuid(uuid::uuid!("11111111-1111-4111-8111-111111111111"));
    assert_ne!(base_digest, compute_command_digest_v2(&diff_gen).unwrap());

    // 4b. aggregate_id
    let mut diff_agg = base_envelope.clone();
    diff_agg.aggregate_id = uuid::uuid!("22222222-2222-4222-8222-222222222222");
    assert_ne!(base_digest, compute_command_digest_v2(&diff_agg).unwrap());

    // 4c. expected_revision
    let mut diff_rev = base_envelope.clone();
    diff_rev.expected_revision = Revision::new(9);
    assert_ne!(base_digest, compute_command_digest_v2(&diff_rev).unwrap());

    // 4d. command_type
    let mut diff_type = base_envelope.clone();
    diff_type.command_type = "ResumeMission".to_string();
    assert_ne!(base_digest, compute_command_digest_v2(&diff_type).unwrap());

    // 4e. project_id
    let mut diff_proj = base_envelope.clone();
    diff_proj.project_id = None;
    assert_ne!(base_digest, compute_command_digest_v2(&diff_proj).unwrap());

    // 4f. payload
    let mut diff_payload = base_envelope.clone();
    diff_payload.payload = json!({ "reason": "autre raison" });
    assert_ne!(
        base_digest,
        compute_command_digest_v2(&diff_payload).unwrap()
    );
}
