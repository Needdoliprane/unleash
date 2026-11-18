// T-F001-008 / AC-F001-008 / REQ-CORE-008
//! Le cœur reste Rust, aucune écriture runtime n'intervient dans `.claude/`,
//! `unleash-cli` ne dépend jamais de `rusqlite` et les projets supervisés restent polyglottes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonError, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision};
use unleash_platform::PlatformError;
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope};
use unleash_testkit::local_test_dir;

#[tokio::test]
async fn t_f001_008_forbids_claude_paths_verifies_cli_boundary_and_polyglot_project() {
    // 1. Refus strict de toute racine ou rendez-vous sous `.claude/`
    let rdv_ok = local_test_dir("f001-08-rdv").unwrap();
    std::fs::set_permissions(
        rdv_ok.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let evil_data = PathBuf::from("/tmp/unleash-test/.claude/runtime-data");
    let err_data = UnleashDaemon::start(DaemonConfig {
        data_root: Some(evil_data),
        rendezvous_dir: Some(rdv_ok.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap_err();
    assert!(matches!(
        err_data,
        DaemonError::Platform(PlatformError::ForbiddenDocumentationPath { .. })
    ));

    let data_ok = local_test_dir("f001-08-dat").unwrap();
    let evil_rdv = PathBuf::from("/tmp/.claude/rdv");
    let err_rdv = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data_ok.path().to_path_buf()),
        rendezvous_dir: Some(evil_rdv),
        ..DaemonConfig::default()
    })
    .await
    .unwrap_err();
    assert!(matches!(
        err_rdv,
        DaemonError::Platform(PlatformError::ForbiddenDocumentationPath { .. })
    ));

    // 2. unleash-cli Cargo.toml ne doit contenir aucune dépendance rusqlite, unleash-store ou unleash-daemon
    let cli_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("unleash-cli/Cargo.toml");
    let cli_toml = std::fs::read_to_string(cli_manifest).unwrap();
    assert!(!cli_toml.contains("rusqlite"));
    assert!(!cli_toml.contains("unleash-store"));
    assert!(!cli_toml.contains("unleash-daemon"));

    // 3. Supervision d'un dépôt projet multilangage Python/TypeScript sous $TMPDIR
    let polyglot_repo = local_test_dir("f001-08-poly").unwrap();
    std::fs::write(
        polyglot_repo.path().join("pyproject.toml"),
        "[project]\nname='demo'\n",
    )
    .unwrap();
    std::fs::write(
        polyglot_repo.path().join("package.json"),
        "{\"name\":\"demo\"}",
    )
    .unwrap();
    let project_claude = polyglot_repo.path().join(".claude");
    std::fs::create_dir(&project_claude).unwrap();
    std::fs::write(project_claude.join("sentinel.txt"), "unchanged").unwrap();

    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data_ok.path().to_path_buf()),
        rendezvous_dir: Some(rdv_ok.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("Daemon attendu démarré");
    };

    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    let proj = ProjectId::from_u128(0x2801);
    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x1),
            command_id: CommandId::from_u128(0xC1),
            authority_generation: client.hello().authority_generation,
            actor_ref: None,
            project_id: Some(proj),
            aggregate_id: proj.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({
                "name": "PolyglotPythonTS",
                "weight": 1
            }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();

    let root = polyglot_repo.path().to_string_lossy().into_owned();
    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(2),
            command_id: CommandId::from_u128(0xC2),
            authority_generation: client.hello().authority_generation,
            actor_ref: None,
            project_id: Some(proj),
            aggregate_id: proj.as_uuid(),
            expected_revision: Revision::new(1),
            command_type: String::from("ReviseProject"),
            payload: json!({"name":"PolyglotPythonTS", "description":"", "weight":1,
            "primary_root_path":root, "sources":[{"kind":"directory", "path_or_uri":root,
                "label":"polyglot", "read_only":true}], "repositories":[], "settings_json":{}})
            .as_object()
            .unwrap()
            .clone(),
        })
        .await
        .unwrap();
    let project = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(3),
            query_type: "GetProject".into(),
            project_id: Some(proj),
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert_eq!(project.result["primary_root_path"], root);
    assert_eq!(project.result["sources"][0]["path_or_uri"], root);
    assert_eq!(
        std::fs::read_to_string(project_claude.join("sentinel.txt")).unwrap(),
        "unchanged"
    );
    assert_eq!(std::fs::read_dir(&project_claude).unwrap().count(), 1);
    assert_eq!(
        std::fs::read_to_string(polyglot_repo.path().join("pyproject.toml")).unwrap(),
        "[project]\nname='demo'\n"
    );
    assert_eq!(
        std::fs::read_to_string(polyglot_repo.path().join("package.json")).unwrap(),
        "{\"name\":\"demo\"}"
    );

    handle.shutdown_graceful("done").await.unwrap();
}
