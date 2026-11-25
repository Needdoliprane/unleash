//! T-F010-001 : inventaire IPC d’un checkout externe dirty, sans mutation.

use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};

use serde_json::json;
use unleash_client::{ClientConfig, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, ReceiptState};
use unleash_testkit::local_test_dir;

#[allow(
    clippy::unwrap_used,
    reason = "Les commandes Git ne préparent que le dépôt temporaire de la fixture"
)]
async fn git(repo: &Path, template: &Path, args: &[&str]) -> String {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(repo)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TEMPLATE_DIR", template)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true)
        .args(args);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[allow(
    clippy::unwrap_used,
    reason = "La création et le rattachement sont des prérequis du parcours IPC"
)]
async fn attach_repository(client: &mut UnleashClient, project: ProjectId, repo: &Path) {
    let mut command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x00f0_1001),
        command_id: CommandId::from_u128(0x00f0_1002),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: Some(project),
        aggregate_id: project.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "CreateProject".to_owned(),
        payload: serde_json::from_value(json!({"name":"F010 fixture", "weight":1})).unwrap(),
    };
    let receipt = client.send_command(&command).await.unwrap();
    // La base M4 rend un reçu `accepted` pour `CreateProject` et `ReviseProject`, comportement
    // épinglé par e2e_m1_restore_lineage. `accepted` annonce un engagement, pas une fin : l'effet
    // est attendu par un sondage borné avant de poursuivre.
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.new_revision, Revision::new(1));
    wait_for_project(client, project, Revision::new(1), None).await;
    command.request_id = RequestId::from_u128(0x00f0_1003);
    command.command_id = CommandId::from_u128(0x00f0_1004);
    command.command_type = "ReviseProject".to_owned();
    command.expected_revision = receipt.new_revision;
    command.payload = serde_json::from_value(json!({
        "name": "F010 fixture",
        "description": "dépôt local temporaire",
        "weight": 1,
        "primary_root_path": null,
        "sources": [],
        "repositories": [{
            "repo_id": "repo-01",
            "root_path": repo.to_str().unwrap(),
            "default_branch": "integration"
        }],
        "settings_json": {}
    }))
    .unwrap();
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.new_revision, Revision::new(2));
    wait_for_project(client, project, Revision::new(2), Some(repo)).await;
}

/// Attend que `GetProject` rende le projet à la révision attendue, dépôt rattaché compris quand
/// `repo` est fourni. L'inventaire ne court ainsi jamais après un état qui n'existe pas encore.
#[allow(
    clippy::indexing_slicing,
    reason = "La fixture lit les champs de la projection qu'elle attend"
)]
async fn wait_for_project(
    client: &mut UnleashClient,
    project: ProjectId,
    revision: Revision,
    repo: Option<&Path>,
) {
    let reached = |result: &serde_json::Value| {
        result["revision"] == json!(revision)
            && repo.is_none_or(|repo| {
                result["repositories"].as_array().is_some_and(|repositories| {
                    repositories
                        .iter()
                        .any(|entry| entry["root_path"].as_str() == repo.to_str())
                })
            })
    };
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let query = QueryEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0x00f0_10ff),
                query_type: "GetProject".to_owned(),
                project_id: Some(project),
                payload: serde_json::Map::new(),
            };
            if client.query(&query).await.is_ok_and(|response| reached(&response.result)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    assert!(observed, "projet attendu à la révision {}", revision.as_u64());
}

#[tokio::test]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "Le scénario IPC conserve ensemble la fixture, la réponse et ses preuves avant/après"
)]
async fn t_f010_001_lists_dirty_external_worktree_without_mutation() {
    let repo_dir = local_test_dir("f010-repo").unwrap();
    let repo = std::fs::canonicalize(repo_dir.path()).unwrap();
    let template = local_test_dir("f010-git-template").unwrap();
    let version = git(&repo, template.path(), &["--version"]).await;
    assert!(version.starts_with("git version "));
    git(
        &repo,
        template.path(),
        &["init", "--object-format=sha1", "--initial-branch=integration"],
    )
    .await;
    let tracked = repo.join("tracked.txt");
    std::fs::write(&tracked, b"base\n").unwrap();
    git(&repo, template.path(), &["add", "--", "tracked.txt"]).await;
    git(
        &repo,
        template.path(),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    )
    .await;
    let head_oid = git(&repo, template.path(), &["rev-parse", "HEAD"]).await;
    assert_eq!(head_oid.len(), 40);
    std::fs::write(&tracked, b"modified\r\n").unwrap();

    let data = local_test_dir("f010-data").unwrap();
    let rendezvous = local_test_dir("f010-rdv").unwrap();
    std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config).await.unwrap() else {
        panic!("daemon attendu démarré");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        daemon.socket_path().to_owned(),
        ClientKind::Cli,
        "test",
    ))
    .await
    .unwrap();
    let project = ProjectId::from_u128(0x00f0_1005);
    attach_repository(&mut client, project, &repo).await;

    let index = repo.join(".git/index");
    let head = repo.join(".git/HEAD");
    let branch = repo.join(".git/refs/heads/integration");
    let before = [
        std::fs::read(&tracked).unwrap(),
        std::fs::read(&index).unwrap(),
        std::fs::read(&head).unwrap(),
        std::fs::read(&branch).unwrap(),
    ];
    let mut query: QueryEnvelope = serde_json::from_str(include_str!(
        "../../../.claude/examples/ipc-list-worktrees-query.json"
    ))
    .unwrap();
    query.project_id = Some(project);
    let response = client.query(&query).await.unwrap();
    assert_eq!(response.request_id, query.request_id);
    let result = &response.result;
    let digest = result["git_profile_digest"].as_str().unwrap();
    assert_eq!(digest.len(), 71);
    assert!(digest.starts_with("sha256:"));
    assert!(
        digest[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let observed_at = result["observed_at"].as_str().unwrap();
    chrono::DateTime::parse_from_rfc3339(observed_at).unwrap();
    assert_eq!(*result, json!({
        "project_id": project,
        "repo_id": "repo-01",
        "object_format": "sha1",
        "git_profile_digest": digest,
        "observed_at": observed_at,
        "items": [{
            "path": repo.to_str().unwrap(),
            "head_oid": {"format": "sha1", "hex": head_oid},
            "branch_ref": "refs/heads/integration",
            "dirty": "dirty",
            "operations": [],
            "operations_complete": true,
            "operation_branches": [],
            "operation_branches_complete": true,
            "ownership": "external"
        }],
        "coverage": {"status": "complete"},
        "next_cursor": null
    }));
    let after = [
        std::fs::read(&tracked).unwrap(),
        std::fs::read(&index).unwrap(),
        std::fs::read(&head).unwrap(),
        std::fs::read(&branch).unwrap(),
    ];
    assert_eq!(before, after);
    drop(client);
    daemon.shutdown_graceful("F010 fixture terminée").await.unwrap();
}
