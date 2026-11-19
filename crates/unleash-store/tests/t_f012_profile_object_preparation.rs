//! F012 : deux producteurs gardent chacun leur pin exact sur les mêmes octets.
use unleash_domain::Sha256Digest;
use unleash_store::{
    PrepareObjectRequest, PublishObjectReferenceRequest, StoreConfig, UnleashStore,
};
use unleash_testkit::local_test_dir;

#[test]
fn t_f012_002_publishing_one_candidate_does_not_release_another_producer_pin() {
    let directory = local_test_dir("f012-two-candidate-pins").unwrap();
    let store = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    let bytes = b"synthetic identical provider configuration";
    let hash = Sha256Digest::from_bytes(bytes);
    let request = |owner: &str| PrepareObjectRequest {
        object_hash: hash,
        byte_length: bytes.len() as u64,
        media_type: "application/json".to_owned(),
        owner_kind: "provider_profile_preparation".to_owned(),
        owner_id: owner.to_owned(),
    };
    let first = store.prepare_object(request("attempt-a")).unwrap();
    let second = store.prepare_object(request("attempt-b")).unwrap();
    assert_ne!(first.pin_id, second.pin_id);
    let held: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM object_pins WHERE object_hash=?1 AND state='held'",
                [hash.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(held, 2);
    store.write_prepared_object_bytes(&first, bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: first.pin_id.clone(),
            object_hash: hash,
            object_generation: first.object_generation,
            owner_kind: "provider_profile_preparation".to_owned(),
            owner_id: "attempt-a".to_owned(),
            source_field: "configuration".to_owned(),
        })
        .unwrap();
    let state: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT state FROM object_pins WHERE pin_id=?1",
                [&second.pin_id],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(
        state, "held",
        "publication must release only the publishing producer's exact pin"
    );
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: second.pin_id,
            object_hash: hash,
            object_generation: second.object_generation,
            owner_kind: "provider_profile_preparation".to_owned(),
            owner_id: "attempt-b".to_owned(),
            source_field: "configuration".to_owned(),
        })
        .unwrap();
}
