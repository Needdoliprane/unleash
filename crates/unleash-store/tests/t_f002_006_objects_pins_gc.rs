// T-F002-006 / AC-F002-006 / REQ-PERSIST-006
//! Protocole objets hors base, pins, collecte exclusive et cutpoints C10a..d.

use std::sync::Arc;
use std::time::Duration;

use unleash_domain::Sha256Digest;
use unleash_store::{
    CutpointObserver, MonotonicClockPort, ObjectState, PrepareObjectDeletionRequest,
    PrepareObjectRequest, PublishObjectReferenceRequest, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore,
};
use unleash_testkit::{
    Cutpoint, CutpointAction, CutpointController, DeterministicIdGenerator, ManualClock,
    local_test_dir,
};

struct ClockAdapter(Arc<ManualClock>);
impl MonotonicClockPort for ClockAdapter {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

struct CutpointAdapter(CutpointController);
impl CutpointObserver for CutpointAdapter {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        let cp = match cutpoint {
            StoreCutpoint::C10aAfterPrepPin => Cutpoint::C10a,
            StoreCutpoint::C10bBeforeDeletingReserve => Cutpoint::C10b,
            StoreCutpoint::C10cAfterDeletingIntent => Cutpoint::C10c,
            StoreCutpoint::C10dAfterUnlinkSyncBeforeTombstone => Cutpoint::C10d,
            _ => return Ok(()),
        };
        self.0.hit(cp).map_err(|e| StoreError::CutpointInjected {
            cutpoint: cp.as_str().to_owned(),
            message: e.to_string(),
        })
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Couvre C10a..d, course concurrente C10b/C10c multi-thread dans les 2 ordres, réutilisation et quarantaine"
)]
fn test_t_f002_006_c10_a_b_c_d_pins_exclusive_deletion_and_recreation() {
    let dir = local_test_dir("f002-006").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0006));
    let ctrl = CutpointController::new();
    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg.id_generator = ids.clone();
    cfg.cutpoints = Arc::new(CutpointAdapter(ctrl.clone()));

    let store = Arc::new(UnleashStore::open(cfg.clone()).unwrap());
    let payload_bytes = b"durable artifact payload v1".to_vec();
    let hash_h = Sha256Digest::from_bytes(&payload_bytes);

    // 1. C10a : pin de préparation committé, crash avant écriture/publication ; horloge avancée ne libère pas le pin
    ctrl.set_action_on_hit(Cutpoint::C10a, 1, CutpointAction::Fail("crash-c10a"))
        .unwrap();
    let prep_err = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash_h,
            byte_length: u64::try_from(payload_bytes.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-1".to_owned(),
        })
        .unwrap_err();
    assert!(matches!(prep_err, StoreError::CutpointInjected { .. }));
    ctrl.clear(Cutpoint::C10a);

    // Reprise après crash + avance horloge de 30 jours : pin passe reconciling et BLOQUE toujours la collecte !
    clock.advance(Duration::from_secs(30 * 86_400));
    let rec_report = store.reconcile_objects_and_deletions().unwrap();
    assert_eq!(rec_report.prepared_objects_preserved, 1);

    let gc_blocked = store
        .prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: hash_h,
            expected_generation: 1,
            reason: "gc_test".to_owned(),
            decision_receipt_id: None,
        })
        .unwrap_err();
    assert!(matches!(gc_blocked, StoreError::ObjectPinned { .. }));

    // Finaliser proprement une nouvelle préparation/publication pour H
    let store2 = Arc::clone(&store);
    let handle = store2
        .prepare_object(PrepareObjectRequest {
            object_hash: hash_h,
            byte_length: u64::try_from(payload_bytes.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-1".to_owned(),
        })
        .unwrap();
    assert_eq!(handle.object_generation, 1);
    let gen1_rel_path = handle.relative_path.clone();
    store2
        .write_prepared_object_bytes(&handle, &payload_bytes)
        .unwrap();
    store2
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id.clone(),
            object_hash: hash_h,
            object_generation: handle.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-1".to_owned(),
            source_field: "object_hash".to_owned(),
        })
        .unwrap();
    assert_eq!(
        store2.read_verified_object_bytes(hash_h).unwrap(),
        payload_bytes
    );

    // La nouvelle publication ne neutralise pas le producteur C10a précédent.
    // Retirer la référence prévue pour le test GC ne suffit pas à lever son pin.
    store2
        .tombstone_object_references(hash_h, "candidate_before_c10b")
        .unwrap();
    let orphan_pin: String = store2.read_snapshot(|conn,_| {
        Ok(conn.query_row("SELECT pin_id FROM object_pins WHERE object_hash=?1 AND object_generation=1 AND owner_kind='evidence' AND owner_id='ev-1' AND pin_kind='preparation' AND state='reconciling'",[hash_h.to_string()],|row|row.get(0))?)
    }).unwrap();
    assert!(matches!(
        store2.prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: hash_h,
            expected_generation: 1,
            reason: "old_producer_still_protected".to_owned(),
            decision_receipt_id: None
        }),
        Err(StoreError::ObjectPinned { .. })
    ));
    // Fixture attestée : l'appel synchrone a rendu Err en C10a AVANT toute I/O
    // physique et ce banc n'a aucun ancien thread producteur encore en vol.
    // Ceci n'est pas une qualification de récupération automatique en produit.
    store2.release_pin(&orphan_pin).unwrap();

    // 2. C10b ordre 1 (vrai multi-thread sous barrière) :
    // GC présélectionne H et se suspend sur C10b avant tx_prepare_object_deletion ;
    // producteur concurrent acquiert un read pin puis republie pendant que GC attend -> GC échoue.
    let b_gc_lose = ctrl.arm_barrier(Cutpoint::C10b, Duration::from_secs(10));
    let s_gc1 = Arc::clone(&store2);
    let t_gc1 = std::thread::spawn(move || {
        s_gc1.prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: hash_h,
            expected_generation: 1,
            reason: "gc_loses_race".to_owned(),
            decision_receipt_id: None,
        })
    });
    b_gc_lose.wait_arrived(1, Duration::from_secs(5)).unwrap();

    // Producteur concurrent gagne sous la barrière pendant que le thread GC est bloqué en C10b
    let s_prod1 = Arc::clone(&store2);
    let prod1_len = u64::try_from(payload_bytes.len()).unwrap_or(0);
    let t_prod1 = std::thread::spawn(move || {
        let read_pin = s_prod1.acquire_read_pin(hash_h, "reader", "r-concurrent")?;
        let prep_again = s_prod1.prepare_object(PrepareObjectRequest {
            object_hash: hash_h,
            byte_length: prod1_len,
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-repub".to_owned(),
        })?;
        s_prod1.publish_object_reference(PublishObjectReferenceRequest {
            pin_id: prep_again.pin_id,
            object_hash: hash_h,
            object_generation: prep_again.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-repub".to_owned(),
            source_field: "object_hash".to_owned(),
        })?;
        Ok::<_, StoreError>(read_pin)
    });
    let concurrent_read_pin = t_prod1.join().unwrap().unwrap();

    // Libérer C10b : le collecteur doit échouer sur référence live / pin actif
    b_gc_lose.release();
    ctrl.clear(Cutpoint::C10b);
    let gc_denied_live = t_gc1.join().unwrap().unwrap_err();
    assert!(matches!(
        gc_denied_live,
        StoreError::ObjectPinned { .. } | StoreError::ObjectReferenced { .. }
    ));

    // Nettoyer pour préparer l'ordre inverse
    store2.release_pin(&concurrent_read_pin).unwrap();
    store2
        .tombstone_object_references(hash_h, "purge_approved")
        .unwrap();

    // 3. C10b ordre 2 (vrai multi-thread sous barrière) :
    // GC gagne `deleting` et s'arrête sur C10c avant unlink pendant que 3 threads producteurs/lecteurs
    // tentent concurrentiellement prepare_object, acquire_read_pin et publish_object_reference -> tous OBJECT_DELETING.
    let b_gc_c10c = ctrl.arm_barrier(Cutpoint::C10c, Duration::from_secs(10));
    let del_intent = store2
        .prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: hash_h,
            expected_generation: 1,
            reason: "gc_wins_race".to_owned(),
            decision_receipt_id: None,
        })
        .unwrap();
    let s_gc2 = Arc::clone(&store2);
    let intent_for_gc2 = del_intent.clone();
    let t_gc2 =
        std::thread::spawn(move || s_gc2.execute_and_finalize_object_deletion(&intent_for_gc2));
    b_gc_c10c.wait_arrived(1, Duration::from_secs(5)).unwrap();

    let race_gate = Arc::new(std::sync::Barrier::new(3));
    let s_p = Arc::clone(&store2);
    let g_p = Arc::clone(&race_gate);
    let len_u64 = u64::try_from(payload_bytes.len()).unwrap();
    let t_prep_err = std::thread::spawn(move || {
        g_p.wait();
        s_p.prepare_object(PrepareObjectRequest {
            object_hash: hash_h,
            byte_length: len_u64,
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-2".to_owned(),
        })
    });
    let s_r = Arc::clone(&store2);
    let g_r = Arc::clone(&race_gate);
    let t_read_err = std::thread::spawn(move || {
        g_r.wait();
        s_r.acquire_read_pin(hash_h, "reader", "r-concurrent-deleting")
    });
    let s_pub = Arc::clone(&store2);
    let g_pub = Arc::clone(&race_gate);
    let t_pub_err = std::thread::spawn(move || {
        g_pub.wait();
        s_pub.publish_object_reference(PublishObjectReferenceRequest {
            pin_id: "stale-pin".to_owned(),
            object_hash: hash_h,
            object_generation: 1,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-2".to_owned(),
            source_field: "object_hash".to_owned(),
        })
    });

    let err_del = t_prep_err.join().unwrap().unwrap_err();
    assert!(matches!(err_del, StoreError::ObjectDeleting { .. }));
    assert_eq!(
        err_del.reason_code(),
        Some(unleash_protocol::ReasonCode::ObjectDeleting)
    );
    assert!(matches!(
        t_read_err.join().unwrap().unwrap_err(),
        StoreError::ObjectDeleting { .. }
    ));
    assert!(matches!(
        t_pub_err.join().unwrap().unwrap_err(),
        StoreError::ObjectDeleting { .. }
    ));

    // Préparer aussi le test C10c fail-crash après libération de cette barrière : injecter avant reprise
    // On libère d'abord la barrière de ce thread GC mais avec un second cutpoint ou on reteste C10c sur un second cas ;
    // plus simple : libérer b_gc_c10c après l'assertion sur l'exclusion, puis vérifier le résultat final ou C10d!
    // Attention : `t_gc2` va unlink/tombstone sauf si on simule le crash C10c avant unlink.
    // On vérifie la symétrie concurrente pure sans biais d'ordonnancement sur un objet distinct `H_sym`,
    // puis on laisse `t_gc2` terminer et on teste `C10c`/`C10d` explicitement sur `H_crash`!
    b_gc_c10c.release();
    ctrl.clear(Cutpoint::C10c);
    t_gc2.join().unwrap().unwrap();
    assert_eq!(
        store2.object_record(hash_h).unwrap().state,
        ObjectState::Tombstone
    );

    // Course symétrique non biaisée sous Barrier(2) : soit GC réserve deleting et Producteur reçoit OBJECT_DELETING,
    // soit Producteur pose preparation pin et GC reçoit OBJECT_PINNED/OBJECT_REFERENCED (jamais les deux gagnants).
    let sym_bytes = b"symmetric-c10b-race-blob".to_vec();
    let hash_sym = Sha256Digest::from_bytes(&sym_bytes);
    let h_sym = store2
        .prepare_object(PrepareObjectRequest {
            object_hash: hash_sym,
            byte_length: u64::try_from(sym_bytes.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-sym".to_owned(),
        })
        .unwrap();
    store2
        .write_prepared_object_bytes(&h_sym, &sym_bytes)
        .unwrap();
    store2
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: h_sym.pin_id,
            object_hash: hash_sym,
            object_generation: 1,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-sym".to_owned(),
            source_field: "object_hash".to_owned(),
        })
        .unwrap();
    store2
        .tombstone_object_references(hash_sym, "sym_race_setup")
        .unwrap();

    let sym_barrier = Arc::new(std::sync::Barrier::new(2));
    let b_gc_s = Arc::clone(&sym_barrier);
    let s_gc_s = Arc::clone(&store2);
    let th_gc = std::thread::spawn(move || {
        b_gc_s.wait();
        s_gc_s.prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: hash_sym,
            expected_generation: 1,
            reason: "sym_gc".to_owned(),
            decision_receipt_id: None,
        })
    });
    let b_pr_s = Arc::clone(&sym_barrier);
    let s_pr_s = Arc::clone(&store2);
    let sym_len = u64::try_from(sym_bytes.len()).unwrap();
    let th_pr = std::thread::spawn(move || {
        b_pr_s.wait();
        s_pr_s.prepare_object(PrepareObjectRequest {
            object_hash: hash_sym,
            byte_length: sym_len,
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-sym-prod".to_owned(),
        })
    });
    match (th_gc.join().unwrap(), th_pr.join().unwrap()) {
        (Ok(intent_sym), Err(StoreError::ObjectDeleting { .. })) => {
            store2
                .execute_and_finalize_object_deletion(&intent_sym)
                .unwrap();
        }
        (
            Err(StoreError::ObjectPinned { .. } | StoreError::ObjectReferenced { .. }),
            Ok(prod_h),
        ) => {
            store2.release_pin(&prod_h.pin_id).unwrap();
            let intent_sym = store2
                .prepare_object_deletion(PrepareObjectDeletionRequest {
                    object_hash: hash_sym,
                    expected_generation: 1,
                    reason: "sym_cleanup".to_owned(),
                    decision_receipt_id: None,
                })
                .unwrap();
            store2
                .execute_and_finalize_object_deletion(&intent_sym)
                .unwrap();
        }
        (r_gc, r_pr) => panic!("exclusion mutuelle violée : gc={r_gc:?}, prod={r_pr:?}"),
    }

    // 4. Recréer H (gen=2) pour vérifier C10c -> C10d -> Tombstone puis gen=3
    let handle2_prep = store2
        .prepare_object(PrepareObjectRequest {
            object_hash: hash_h,
            byte_length: u64::try_from(payload_bytes.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-1-c10cd".to_owned(),
        })
        .unwrap();
    assert_eq!(handle2_prep.object_generation, 2);
    store2
        .write_prepared_object_bytes(&handle2_prep, &payload_bytes)
        .unwrap();
    store2
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: handle2_prep.pin_id,
            object_hash: hash_h,
            object_generation: 2,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-1-c10cd".to_owned(),
            source_field: "object_hash".to_owned(),
        })
        .unwrap();
    store2
        .tombstone_object_references(hash_h, "purge_c10cd")
        .unwrap();
    let del_intent = store2
        .prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: hash_h,
            expected_generation: 2,
            reason: "gc_c10c_c10d".to_owned(),
            decision_receipt_id: None,
        })
        .unwrap();

    // C10c : crash après deleting/prepared avant unlink -> reprise termine exactement (H, gen=2)
    ctrl.set_action_on_hit(
        Cutpoint::C10c,
        ctrl.hit_count(Cutpoint::C10c) + 1,
        CutpointAction::Fail("crash-c10c"),
    )
    .unwrap();
    assert!(
        store2
            .execute_and_finalize_object_deletion(&del_intent)
            .is_err()
    );
    ctrl.clear(Cutpoint::C10c);

    // 5. C10d : crash après unlink + dir sync avant tombstone -> reprise finalise tombstone/completed idempotente
    ctrl.set_action_on_hit(
        Cutpoint::C10d,
        ctrl.hit_count(Cutpoint::C10d) + 1,
        CutpointAction::Fail("crash-c10d"),
    )
    .unwrap();
    let rec_c10c = store2.reconcile_objects_and_deletions();
    assert!(rec_c10c.is_err());
    ctrl.clear(Cutpoint::C10d);

    let rec_done = store2.reconcile_objects_and_deletions().unwrap();
    assert_eq!(rec_done.deletions_finalized, 1);
    assert_eq!(
        store2.object_record(hash_h).unwrap().state,
        ObjectState::Tombstone
    );
    assert!(!cfg.artifacts_dir.join(&handle2_prep.relative_path).exists());

    // 6. Recréation du même hash H -> nouvelle génération=3 et nouveau chemin ; rejouer l'ancien collecteur gen=2 NE touche PAS gen=3 !
    let handle2 = store2
        .prepare_object(PrepareObjectRequest {
            object_hash: hash_h,
            byte_length: u64::try_from(payload_bytes.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-gen3".to_owned(),
        })
        .unwrap();
    assert_eq!(handle2.object_generation, 3);
    assert_ne!(handle2.relative_path, gen1_rel_path);
    assert_ne!(handle2.relative_path, handle2_prep.relative_path);
    store2
        .write_prepared_object_bytes(&handle2, &payload_bytes)
        .unwrap();
    store2
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: handle2.pin_id.clone(),
            object_hash: hash_h,
            object_generation: 3,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-gen3".to_owned(),
            source_field: "object_hash".to_owned(),
        })
        .unwrap();

    // Ancien collecteur ciblant del_intent (gen=2) refusé sans supprimer gen=3
    assert!(
        store2
            .execute_and_finalize_object_deletion(&del_intent)
            .is_err()
    );
    assert_eq!(
        store2.read_verified_object_bytes(hash_h).unwrap(),
        payload_bytes
    );

    // Fichier corrompu ou étranger à un chemin deleting -> quarantined sans unlink
    let bytes_q = b"quarantine target".to_vec();
    let hash_q = Sha256Digest::from_bytes(&bytes_q);
    let hq = store2
        .prepare_object(PrepareObjectRequest {
            object_hash: hash_q,
            byte_length: u64::try_from(bytes_q.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-q".to_owned(),
        })
        .unwrap();
    store2.write_prepared_object_bytes(&hq, &bytes_q).unwrap();
    store2
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: hq.pin_id,
            object_hash: hash_q,
            object_generation: hq.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-q".to_owned(),
            source_field: "object_hash".to_owned(),
        })
        .unwrap();
    store2
        .tombstone_object_references(hash_q, "purge_test")
        .unwrap();
    let intent_q = store2
        .prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: hash_q,
            expected_generation: 1,
            reason: "gc_q".to_owned(),
            decision_receipt_id: None,
        })
        .unwrap();
    // Remplacer le fichier par un contenu étranger sur disque avant finalisation
    std::fs::write(cfg.artifacts_dir.join(&hq.relative_path), b"FOREIGN").unwrap();
    assert!(
        store2
            .execute_and_finalize_object_deletion(&intent_q)
            .is_err()
    );
    assert_eq!(
        store2.object_record(hash_q).unwrap().state,
        ObjectState::Quarantined
    );
    assert!(cfg.artifacts_dir.join(&hq.relative_path).exists());
}
