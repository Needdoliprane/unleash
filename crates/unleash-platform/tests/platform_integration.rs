// SPDX-License-Identifier: Apache-2.0
//! Tests d'intégration de `unleash-platform` (verrou d'autorité, rendez-vous `0700`/`0600`,
//! `LOCAL_PEERCRED`/`SO_PEERCRED`, identité OS processus, `KeyringPort`, chemins `ADR-14` et garde `.claude/`).

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;

use secrecy::{ExposeSecret, SecretString};
use unleash_domain::SecretRef;
use unleash_platform::{
    AuthorityLock, EnvSnapshot, KeyringBackendKind, KeyringError, KeyringPort, LocalSecureKeyring,
    MAX_UNIX_SOCKET_PATH_BYTES, PeerCredentials, PlatformError, PlatformOsKind, PlatformPaths,
    ProcessVerification, SecretStatus, UnavailableOsKeyring, assert_not_under_claude,
    bind_private_unix_listener, canonical_rendezvous_dir_with, current_boot_identity,
    current_process_identity, effective_uid, ensure_private_rendezvous_dir,
    extract_peer_credentials, inspect_process_identity, prepare_canonical_rendezvous_dir_with,
    prepare_rendezvous_dir_in_runtime_root, validate_rendezvous_dir, validate_socket_file,
    validate_unix_socket_path_len, verify_peer_uid, verify_process_identity,
    verify_same_uid_peer,
};
use unleash_testkit::local_test_dir;

// T-F001-001 / AC-F001-001 / REQ-CORE-001
#[test]
fn t_f001_001_exclusive_authority_lock_and_xdg_immunity() {
    let uid = effective_uid();

    let env_a = EnvSnapshot {
        home: Some(PathBuf::from("/Users/alice")),
        xdg_runtime_dir: Some(String::from("/tmp/custom-xdg-runtime-a")),
        tmpdir: Some(String::from("/tmp/custom-tmp-a")),
        ..EnvSnapshot::default()
    };
    let env_b = EnvSnapshot {
        home: Some(PathBuf::from("/Users/alice")),
        xdg_runtime_dir: Some(String::from("/tmp/custom-xdg-runtime-b")),
        tmpdir: Some(String::from("/tmp/custom-tmp-b")),
        ..EnvSnapshot::default()
    };

    for os in [PlatformOsKind::MacOs, PlatformOsKind::Linux] {
        let rdv_a = canonical_rendezvous_dir_with(os, uid, &env_a).unwrap();
        let rdv_b = canonical_rendezvous_dir_with(os, uid, &env_b).unwrap();
        assert_eq!(
            rdv_a, rdv_b,
            "Le chemin de rendez-vous doit être indépendant de XDG_RUNTIME_DIR et TMPDIR"
        );
        assert!(
            rdv_a.join("daemon.sock").as_os_str().len() <= MAX_UNIX_SOCKET_PATH_BYTES,
            "Le socket canonique doit respecter SUN_LEN"
        );
    }

    let sandbox = local_test_dir("authlock").unwrap();
    let rendezvous_dir = sandbox.path().join("rdv");
    let root_a = sandbox.path().join("data_a");
    let root_b = sandbox.path().join("data_b");
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();

    // Barrière de 10 clients concurrents sur 2 racines distinctes : 1 seul gagne, les autres échouent explicitement.
    let barrier = Arc::new(Barrier::new(10));
    let mut handles = Vec::with_capacity(10);
    for idx in 0..10 {
        let b = Arc::clone(&barrier);
        let rdv = rendezvous_dir.clone();
        let target_root = if idx % 2 == 0 {
            root_a.clone()
        } else {
            root_b.clone()
        };
        handles.push(thread::spawn(move || {
            b.wait();
            AuthorityLock::acquire_in(&rdv, &target_root)
        }));
    }

    let mut winner: Option<AuthorityLock> = None;
    let mut already_active = 0usize;
    let mut already_running = 0usize;
    for h in handles {
        match h.join().unwrap() {
            Ok(lock) => {
                assert!(winner.is_none(), "Au plus un détenteur du verrou exclusif");
                winner = Some(lock);
            }
            Err(PlatformError::AuthorityAlreadyActive { .. }) => {
                already_active += 1;
            }
            Err(PlatformError::DaemonAlreadyRunning { .. }) => {
                already_running += 1;
            }
            Err(other) => panic!("Erreur inattendue lors de la barrière : {other:?}"),
        }
    }

    let held_lock = winner.expect("Exactement un thread doit acquérir le verrou");
    assert_eq!(already_active + already_running, 9);

    let active_root = held_lock.metadata().data_root.clone();
    let other_root = if active_root == fs::canonicalize(&root_a).unwrap() {
        &root_b
    } else {
        &root_a
    };

    // Second démarrage sur une autre racine => AuthorityAlreadyActive
    let second_other = AuthorityLock::acquire_in(&rendezvous_dir, other_root);
    assert!(
        matches!(
            second_other,
            Err(PlatformError::AuthorityAlreadyActive {
                ref active_data_root,
                ..
            }) if active_data_root == &active_root
        ),
        "Attendu AuthorityAlreadyActive, obtenu : {second_other:?}"
    );

    // Second démarrage sur la même racine => DaemonAlreadyRunning
    let same_root_raw = if active_root == fs::canonicalize(&root_a).unwrap() {
        &root_a
    } else {
        &root_b
    };
    let second_same = AuthorityLock::acquire_in(&rendezvous_dir, same_root_raw);
    assert!(
        matches!(second_same, Err(PlatformError::DaemonAlreadyRunning { .. })),
        "Attendu DaemonAlreadyRunning, obtenu : {second_same:?}"
    );

    // Après libération contrôlée, une autre racine peut prendre l'autorité
    drop(held_lock);
    let switched = AuthorityLock::acquire_in(&rendezvous_dir, other_root)
        .expect("Réacquisition autorisée après libération du verrou");
    assert_eq!(
        switched.metadata().data_root,
        fs::canonicalize(other_root).unwrap()
    );
}

// T-F001-001 / AC-F001-001 / REQ-CORE-001
// Sous Linux, le rendez-vous canonique n'a qu'un seul candidat, dérivé de l'UID seul : ni l'état du
// disque, ni `XDG_RUNTIME_DIR`, ni `TMPDIR` ne le déplacent et aucun repli sous `/tmp` n'existe.
// Deux démons d'un même utilisateur ne peuvent ainsi pas se croire l'autorité sur deux rendez-vous.
#[test]
fn t_f001_001_linux_rendezvous_dir_is_a_single_candidate_derived_from_uid() {
    const FICTITIOUS_UID: u32 = 4242;
    let expected = PathBuf::from(format!("/run/user/{FICTITIOUS_UID}/unleash"));
    let runtime_root = PathBuf::from(format!("/run/user/{FICTITIOUS_UID}"));
    assert!(
        !runtime_root.exists(),
        "Précondition : la racine runtime de l'UID fictif doit être absente, sinon ce test ne prouve pas l'indépendance au disque"
    );

    let hostile = EnvSnapshot {
        home: Some(PathBuf::from("/home/alice")),
        xdg_runtime_dir: Some(String::from("/tmp/hostile-xdg-runtime")),
        tmpdir: Some(String::from("/tmp/hostile-tmpdir")),
        ..EnvSnapshot::default()
    };
    let neutral = EnvSnapshot {
        home: Some(PathBuf::from("/home/bob")),
        ..EnvSnapshot::default()
    };

    let under_hostile_env =
        canonical_rendezvous_dir_with(PlatformOsKind::Linux, FICTITIOUS_UID, &hostile).unwrap();
    let under_neutral_env =
        canonical_rendezvous_dir_with(PlatformOsKind::Linux, FICTITIOUS_UID, &neutral).unwrap();

    assert_eq!(
        under_hostile_env, expected,
        "Le candidat Linux ne doit dépendre que de l'UID, jamais de l'état du disque ni de l'environnement"
    );
    assert_eq!(
        under_neutral_env, expected,
        "Le candidat Linux ne doit pas dépendre de l'environnement"
    );
    assert!(
        !under_hostile_env.starts_with("/tmp"),
        "Aucun candidat sous /tmp ne doit être retenu"
    );
}

/// Champs d'un refus de racine runtime, ou `None` pour tout autre résultat.
fn runtime_root_refusal_details(
    outcome: &Result<PathBuf, PlatformError>,
) -> Option<(&str, &str, &str, &str)> {
    match outcome {
        Err(PlatformError::RuntimeRootUnavailable {
            root,
            rendezvous_dir,
            reason,
            remediation,
        }) => Some((root, rendezvous_dir, reason, remediation)),
        _ => None,
    }
}

// T-F001-001 / AC-F001-001 / REQ-CORE-001
// Sans chemin explicite, une racine runtime manquante est un refus, pas une création : le processus
// qui crée le rendez-vous ne crée jamais la racine runtime et ne retient aucun candidat sous `/tmp`.
#[test]
fn t_f001_001_missing_runtime_root_is_a_refusal_not_a_creation() {
    const FICTITIOUS_UID: u32 = 4242;
    let runtime_root = PathBuf::from(format!("/run/user/{FICTITIOUS_UID}"));
    let expected_root = runtime_root.display().to_string();
    let expected_rendezvous = format!("/run/user/{FICTITIOUS_UID}/unleash");
    let tmp_candidate = Path::new("/tmp/unleash-4242");
    assert!(
        !runtime_root.exists(),
        "Précondition : la racine runtime de l'UID fictif doit être absente pour exercer le refus"
    );
    assert!(
        !tmp_candidate.exists(),
        "Précondition : aucun candidat sous /tmp ne doit préexister"
    );

    let outcome = prepare_canonical_rendezvous_dir_with(PlatformOsKind::Linux, FICTITIOUS_UID);
    let Some((root, rendezvous_dir, reason, remediation)) = runtime_root_refusal_details(&outcome)
    else {
        panic!("Attendu le refus d'une racine runtime absente, obtenu : {outcome:?}");
    };
    assert_eq!(root, expected_root, "le refus doit nommer la racine runtime absente");
    assert_eq!(
        rendezvous_dir, expected_rendezvous,
        "le refus doit nommer le rendez-vous canonique"
    );
    assert!(reason.contains("absente"), "motif inattendu : {reason}");
    assert!(
        remediation.contains("logind"),
        "la remédiation doit être indiquée : {remediation}"
    );
    let reported = outcome.to_string();
    assert!(
        reported.contains(&expected_rendezvous),
        "le refus doit nommer le chemin : {reported}"
    );
    assert!(
        !reported.contains("/tmp"),
        "aucun candidat sous /tmp ne doit être retenu : {reported}"
    );

    // Rien n'a été créé : ni la racine, ni un ancêtre, ni un candidat de repli.
    assert!(
        !runtime_root.exists(),
        "la racine runtime ne doit jamais être créée"
    );
    assert!(
        !tmp_candidate.exists(),
        "aucun candidat sous /tmp ne doit être retenu"
    );
}

// T-F001-001 / AC-F001-001 / REQ-CORE-001
// La racine runtime est qualifiée avant toute création : absente, fichier, lien symbolique, autre
// propriétaire ou mode non privé sont des refus, et un refus ne crée ni le rendez-vous ni la racine.
#[test]
fn t_f001_001_runtime_root_is_qualified_before_the_rendezvous_dir_is_created() {
    let uid = effective_uid();
    let sandbox = local_test_dir("rdvroot").unwrap();

    // 1. Racine absente : refus qui nomme la racine et la remédiation ; rien n'est créé.
    let missing = sandbox.path().join("missing");
    let outcome = prepare_rendezvous_dir_in_runtime_root(&missing, uid);
    let Some((root, rendezvous_dir, reason, remediation)) = runtime_root_refusal_details(&outcome)
    else {
        panic!("Attendu le refus d'une racine absente, obtenu : {outcome:?}");
    };
    assert_eq!(root, missing.display().to_string());
    assert_eq!(
        rendezvous_dir,
        missing.join("unleash").display().to_string()
    );
    assert!(reason.contains("absente"), "motif inattendu : {reason}");
    assert!(
        remediation.contains("logind"),
        "remédiation manquante : {remediation}"
    );
    assert!(
        !missing.exists(),
        "la racine runtime ne doit jamais être créée"
    );

    // 2. Un fichier à la place du répertoire : refus, fichier inchangé.
    let as_file = sandbox.path().join("as-file");
    fs::write(&as_file, b"not a directory").unwrap();
    let outcome = prepare_rendezvous_dir_in_runtime_root(&as_file, uid);
    let Some((_, _, reason, _)) = runtime_root_refusal_details(&outcome) else {
        panic!("Attendu le refus d'une racine qui n'est pas un répertoire, obtenu : {outcome:?}");
    };
    assert!(reason.contains("répertoire"), "motif inattendu : {reason}");
    assert_eq!(fs::read(&as_file).unwrap(), b"not a directory");

    // 3. Lien symbolique, même vers un répertoire privé de l'UID : refus, cible intacte.
    let link_target = sandbox.path().join("link-target");
    fs::create_dir(&link_target).unwrap();
    fs::set_permissions(&link_target, PermissionsExt::from_mode(0o700)).unwrap();
    let symlink_root = sandbox.path().join("symlink-root");
    symlink(&link_target, &symlink_root).unwrap();
    let outcome = prepare_rendezvous_dir_in_runtime_root(&symlink_root, uid);
    let Some((_, _, reason, _)) = runtime_root_refusal_details(&outcome) else {
        panic!("Attendu le refus d'une racine symbolique, obtenu : {outcome:?}");
    };
    assert!(
        reason.contains("lien symbolique"),
        "motif inattendu : {reason}"
    );
    assert_eq!(fs::read_dir(&link_target).unwrap().count(), 0);

    // 4. Répertoire de mode 0755 : refus, aucun rendez-vous créé.
    let shared_root = sandbox.path().join("shared-root");
    fs::create_dir(&shared_root).unwrap();
    fs::set_permissions(&shared_root, PermissionsExt::from_mode(0o755)).unwrap();
    let outcome = prepare_rendezvous_dir_in_runtime_root(&shared_root, uid);
    let Some((_, _, reason, _)) = runtime_root_refusal_details(&outcome) else {
        panic!("Attendu le refus d'une racine non privée, obtenu : {outcome:?}");
    };
    assert!(reason.contains("0o755"), "motif inattendu : {reason}");
    assert_eq!(fs::read_dir(&shared_root).unwrap().count(), 0);

    // 5. Répertoire privé d'un autre UID : refus, aucun rendez-vous créé.
    let foreign_root = sandbox.path().join("foreign-root");
    fs::create_dir(&foreign_root).unwrap();
    fs::set_permissions(&foreign_root, PermissionsExt::from_mode(0o700)).unwrap();
    let outcome = prepare_rendezvous_dir_in_runtime_root(&foreign_root, uid.wrapping_add(1));
    let Some((_, _, reason, _)) = runtime_root_refusal_details(&outcome) else {
        panic!("Attendu le refus d'une racine d'un autre propriétaire, obtenu : {outcome:?}");
    };
    assert!(reason.contains("UID attendu"), "motif inattendu : {reason}");
    assert_eq!(fs::read_dir(&foreign_root).unwrap().count(), 0);
}

// T-F001-001 / AC-F001-001 / REQ-CORE-001
// Une racine runtime qualifiée (répertoire réel, propriété de l'UID attendu, mode 0700) reçoit le
// seul sous-répertoire de rendez-vous `unleash`, en `0700` ; un rendez-vous non privé est refusé.
#[test]
fn t_f001_001_qualified_runtime_root_receives_only_the_private_rendezvous_dir() {
    let uid = effective_uid();
    let sandbox = local_test_dir("rdvok").unwrap();
    let runtime_root = sandbox.path().join("runtime");
    fs::create_dir(&runtime_root).unwrap();
    fs::set_permissions(&runtime_root, PermissionsExt::from_mode(0o700)).unwrap();

    let rendezvous = prepare_rendezvous_dir_in_runtime_root(&runtime_root, uid).unwrap();
    assert_eq!(rendezvous, runtime_root.join("unleash"));
    let meta = fs::symlink_metadata(&rendezvous).unwrap();
    assert!(meta.is_dir() && !meta.file_type().is_symlink());
    assert_eq!(meta.uid(), uid);
    assert_eq!(meta.mode() & 0o7777, 0o700);
    validate_rendezvous_dir(&rendezvous).unwrap();

    // Répété : le rendez-vous existant est revalidé, jamais recréé ni doublé.
    assert_eq!(
        prepare_rendezvous_dir_in_runtime_root(&runtime_root, uid).unwrap(),
        rendezvous
    );
    let created: Vec<std::ffi::OsString> = fs::read_dir(&runtime_root)
        .unwrap()
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        created,
        vec![std::ffi::OsString::from("unleash")],
        "la racine ne doit recevoir que le rendez-vous"
    );

    // Un rendez-vous préexistant non privé est refusé, jamais corrigé.
    fs::set_permissions(&rendezvous, PermissionsExt::from_mode(0o755)).unwrap();
    assert!(
        matches!(
            prepare_rendezvous_dir_in_runtime_root(&runtime_root, uid),
            Err(PlatformError::InsecurePermissions {
                expected_mode: 0o700,
                actual_mode: 0o755,
                ..
            })
        ),
        "un rendez-vous non privé doit être refusé"
    );
    assert_eq!(
        fs::symlink_metadata(&rendezvous).unwrap().mode() & 0o7777,
        0o755
    );
}

// T-F001-004 / AC-F001-004 / REQ-CORE-004
#[test]
fn t_f001_004_rendezvous_permissions_symlinks_and_peer_cred() {
    let sandbox = local_test_dir("rdvsec").unwrap();

    // 1. Répertoire 0755 refusé
    let insecure_dir = sandbox.path().join("bad755");
    fs::create_dir_all(&insecure_dir).unwrap();
    fs::set_permissions(&insecure_dir, PermissionsExt::from_mode(0o755)).unwrap();
    assert!(matches!(
        validate_rendezvous_dir(&insecure_dir),
        Err(PlatformError::InsecurePermissions {
            expected_mode: 0o700,
            actual_mode: 0o755,
            ..
        })
    ));

    // 2. Lien symbolique refusé pour le répertoire de rendez-vous
    let real_dir = sandbox.path().join("real700");
    ensure_private_rendezvous_dir(&real_dir).unwrap();
    let link_dir = sandbox.path().join("link_rdv");
    symlink(&real_dir, &link_dir).unwrap();
    assert!(matches!(
        validate_rendezvous_dir(&link_dir),
        Err(PlatformError::SymlinkNotAllowed { .. })
    ));

    // 3. Limite SUN_LEN (104 octets incluant NUL => max 103 octets)
    let too_long_name = format!("{}.sock", "x".repeat(95));
    let too_long_sock = real_dir.join(too_long_name);
    assert!(too_long_sock.as_os_str().len() > MAX_UNIX_SOCKET_PATH_BYTES);
    assert!(matches!(
        validate_unix_socket_path_len(&too_long_sock),
        Err(PlatformError::SocketPathTooLong { .. })
    ));

    // 4. Socket réel 0600 + LOCAL_PEERCRED (macOS) / SO_PEERCRED (Linux)
    let sock_path = real_dir.join("ipc.sock");
    let listener = bind_private_unix_listener(&sock_path).unwrap();
    validate_socket_file(&sock_path).unwrap();

    let client_stream = UnixStream::connect(&sock_path).unwrap();
    let (server_stream, _) = listener.accept().unwrap();

    let creds: PeerCredentials = extract_peer_credentials(&server_stream).unwrap();
    assert_eq!(creds.uid, effective_uid());
    if let Some(pid) = creds.pid {
        assert_eq!(pid, std::process::id());
    }

    let verified = verify_same_uid_peer(&server_stream).unwrap();
    assert_eq!(verified.uid, effective_uid());

    // Cas négatif : UID peer différent rejeté
    let other_uid = effective_uid().wrapping_add(1);
    assert!(matches!(
        verify_peer_uid(creds, other_uid),
        Err(PlatformError::PeerUidMismatch { .. })
    ));

    drop(client_stream);
    drop(server_stream);
    drop(listener);

    // 5. Socket aux permissions 0644 refusé par validate_socket_file
    fs::set_permissions(&sock_path, PermissionsExt::from_mode(0o644)).unwrap();
    assert!(matches!(
        validate_socket_file(&sock_path),
        Err(PlatformError::InsecurePermissions {
            expected_mode: 0o600,
            actual_mode: 0o644,
            ..
        })
    ));
}

// T-F001-008 / AC-F001-008 / REQ-CORE-008
#[test]
fn t_f001_008_adr14_paths_and_claude_write_guard() {
    let mac_env = EnvSnapshot {
        home: Some(PathBuf::from("/Users/louis")),
        ..EnvSnapshot::default()
    };
    let mac = PlatformPaths::resolve_for(PlatformOsKind::MacOs, &mac_env).unwrap();
    assert_eq!(
        mac.config_dir,
        PathBuf::from("/Users/louis/Library/Application Support/Unleash/config")
    );
    assert_eq!(
        mac.data_dir,
        PathBuf::from("/Users/louis/Library/Application Support/Unleash/data")
    );
    assert_eq!(
        mac.logs_dir,
        PathBuf::from("/Users/louis/Library/Logs/Unleash")
    );
    assert_eq!(
        mac.cache_dir,
        PathBuf::from("/Users/louis/Library/Caches/Unleash")
    );
    assert_eq!(
        mac.default_worktrees_dir,
        PathBuf::from("/Users/louis/Library/Application Support/Unleash/data/worktrees")
    );

    let linux_env = EnvSnapshot {
        home: Some(PathBuf::from("/home/louis")),
        xdg_config_home: Some(String::from("/var/xdg/cfg")),
        xdg_data_home: Some(String::from("/var/xdg/data")),
        xdg_state_home: Some(String::from("/var/xdg/state")),
        xdg_cache_home: Some(String::from("/var/xdg/cache")),
        ..EnvSnapshot::default()
    };
    let linux = PlatformPaths::resolve_for(PlatformOsKind::Linux, &linux_env).unwrap();
    assert_eq!(linux.config_dir, PathBuf::from("/var/xdg/cfg/unleash"));
    assert_eq!(linux.data_dir, PathBuf::from("/var/xdg/data/unleash"));
    assert_eq!(linux.logs_dir, PathBuf::from("/var/xdg/state/unleash/logs"));
    assert_eq!(linux.cache_dir, PathBuf::from("/var/xdg/cache/unleash"));

    // Cas négatifs : XDG relatif ou pointant vers .claude refusé
    let bad_xdg_rel = EnvSnapshot {
        home: Some(PathBuf::from("/home/louis")),
        xdg_data_home: Some(String::from("relative/path")),
        ..EnvSnapshot::default()
    };
    assert!(matches!(
        PlatformPaths::resolve_for(PlatformOsKind::Linux, &bad_xdg_rel),
        Err(PlatformError::InvalidPath { .. })
    ));

    let bad_xdg_claude = EnvSnapshot {
        home: Some(PathBuf::from("/home/louis")),
        xdg_data_home: Some(String::from("/tmp/repo/.claude/runtime")),
        ..EnvSnapshot::default()
    };
    assert!(matches!(
        PlatformPaths::resolve_for(PlatformOsKind::Linux, &bad_xdg_claude),
        Err(PlatformError::ForbiddenDocumentationPath { .. })
    ));

    // Garde .claude/ directe, traversée et symlink
    assert!(matches!(
        assert_not_under_claude(&PathBuf::from("/tmp/project/.claude/state")),
        Err(PlatformError::ForbiddenDocumentationPath { .. })
    ));
    assert!(matches!(
        assert_not_under_claude(&PathBuf::from("/tmp/project/sub/../.claude")),
        Err(PlatformError::ForbiddenDocumentationPath { .. })
    ));

    let sandbox = local_test_dir("claudeguard").unwrap();
    let claude_target = sandbox.path().join(".claude");
    fs::create_dir_all(&claude_target).unwrap();
    let evil_link = sandbox.path().join("alias");
    symlink(&claude_target, &evil_link).unwrap();
    assert!(matches!(
        assert_not_under_claude(&evil_link.join("db.sqlite")),
        Err(PlatformError::ForbiddenDocumentationPath { .. })
    ));
}

#[test]
fn test_process_identity_boot_and_start() {
    let boot = current_boot_identity().unwrap();
    assert!(!boot.as_str().is_empty());

    let self_id = current_process_identity().unwrap();
    assert_eq!(self_id.pid, std::process::id());
    assert_eq!(self_id.owner_uid, effective_uid());
    assert_eq!(self_id.boot_identity, boot);
    assert!(!self_id.start_identity.as_str().is_empty());
    assert_eq!(
        verify_process_identity(&self_id).unwrap(),
        ProcessVerification::Match
    );

    // Inspecter un vrai processus enfant puis vérifier après exit
    let mut child = Command::new("/bin/sleep")
        .arg("5")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let child_pid = child.id();
    let child_identity = inspect_process_identity(child_pid).unwrap();
    assert_eq!(child_identity.pid, child_pid);
    assert_eq!(child_identity.owner_uid, effective_uid());
    assert_eq!(
        verify_process_identity(&child_identity).unwrap(),
        ProcessVerification::Match
    );

    // Simuler une divergence de start_identity (réutilisation de PID)
    let mut forged = child_identity.clone();
    forged.start_identity = unleash_platform::StartIdentity::new("forged-start-identity").unwrap();
    assert!(matches!(
        verify_process_identity(&forged).unwrap(),
        ProcessVerification::IdentityMismatch { .. }
    ));

    // Simuler un reboot (boot_identity différent)
    let mut stale_boot = child_identity.clone();
    stale_boot.boot_identity = unleash_platform::BootIdentity::new("old-boot-session").unwrap();
    assert_eq!(
        verify_process_identity(&stale_boot).unwrap(),
        ProcessVerification::BootChanged
    );

    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        verify_process_identity(&child_identity).unwrap(),
        ProcessVerification::Exited
    );
}

#[test]
fn test_keyring_port_and_local_secure_adapter() {
    let keyring = LocalSecureKeyring::new();
    assert_eq!(keyring.backend_kind(), KeyringBackendKind::LocalSecureTest);

    let secret_ref = SecretRef::new("os-secret:ovpt-unleash/openai/test").unwrap();
    let canary = "sk-CANARY-DO-NOT-LEAK-998877";

    assert_eq!(keyring.status(&secret_ref).unwrap(), SecretStatus::Missing);
    assert!(matches!(
        keyring.load(&secret_ref),
        Err(KeyringError::NotFound)
    ));

    keyring
        .store(&secret_ref, SecretString::from(canary))
        .unwrap();
    assert_eq!(keyring.status(&secret_ref).unwrap(), SecretStatus::Present);

    let loaded = keyring.load(&secret_ref).unwrap();
    assert_eq!(loaded.expose_secret(), canary);

    // Aucun Debug/Display ne doit exposer le canari
    let dbg_keyring = format!("{keyring:?}");
    let dbg_ref = format!("{secret_ref:?}");
    assert!(!dbg_keyring.contains(canary));
    assert!(!dbg_ref.contains(canary));
    assert!(!dbg_ref.contains("openai"));

    // Simulation coffre verrouillé
    keyring.lock();
    assert_eq!(keyring.status(&secret_ref).unwrap(), SecretStatus::Locked);
    assert!(matches!(
        keyring.load(&secret_ref),
        Err(KeyringError::Locked)
    ));
    assert!(matches!(
        keyring.store(&secret_ref, SecretString::from("new")),
        Err(KeyringError::Locked)
    ));
    keyring.unlock();

    // Simulation refus d'accès ciblé
    keyring.deny_ref(&secret_ref);
    assert!(matches!(
        keyring.load(&secret_ref),
        Err(KeyringError::AccessDenied { .. })
    ));
    keyring.allow_ref(&secret_ref);

    // Simulation coffre indisponible
    keyring.set_unavailable(true);
    assert!(matches!(
        keyring.load(&secret_ref),
        Err(KeyringError::Unavailable { .. })
    ));
    keyring.set_unavailable(false);

    // Suppression & zéroïsation
    keyring.delete(&secret_ref).unwrap();
    assert_eq!(keyring.status(&secret_ref).unwrap(), SecretStatus::Missing);
    assert!(matches!(
        keyring.load(&secret_ref),
        Err(KeyringError::NotFound)
    ));

    // Adaptateur OS fail-closed quand non provisionné (EXT-DIST)
    let unavailable = UnavailableOsKeyring::new("Keychain non configuré sans profil signé");
    assert_eq!(unavailable.backend_kind(), KeyringBackendKind::Unavailable);
    assert!(matches!(
        unavailable.load(&secret_ref),
        Err(KeyringError::Unavailable { .. })
    ));
}
