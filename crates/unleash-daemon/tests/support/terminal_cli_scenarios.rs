//! Binaire CLI réel, daemon réel et contrôle d'une même génération PTY.

use super::*;

struct TerminalCli<'a> {
    binary: &'a Path,
    socket: PathBuf,
    project: String,
    authority: String,
    terminal: String,
}

impl TerminalCli<'_> {
    fn invoke(&self, args: &[&str]) -> std::process::Output {
        Command::new(self.binary)
            .arg("--socket")
            .arg(&self.socket)
            .args(["--json", "--timeout-ms", "5000"])
            .args(args)
            .output()
            .expect("run the prebuilt CLI against the owned fixture socket")
    }

    fn run(&self, args: &[&str]) -> Value {
        let output = self.invoke(args);
        assert!(
            output.status.success(),
            "CLI refusal: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("structured CLI response")
    }

    fn assert_refused(&self, args: &[&str], reason: &str) {
        let output = self.invoke(args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "expected refusal for {reason}");
        assert!(stderr.contains(reason), "unexpected CLI refusal: {stderr}");
    }

    fn acquire(&self, variant: &str, command_id: &str, target: &[&str]) -> Value {
        let mut args = vec![
            "process",
            "acquire",
            variant,
            "--project-id",
            &self.project,
            "--scope-revision",
            "2",
            "--terminal-id",
            &self.terminal,
            "--command-id",
            command_id,
            "--authority-generation",
            &self.authority,
        ];
        args.extend_from_slice(target);
        let response = self.run(&args);
        assert_eq!(response.get("project_id"), Some(&json!(self.project)));
        assert_eq!(response.get("terminal_id"), Some(&json!(self.terminal)));
        assert_eq!(
            response.get("authority_generation"),
            Some(&json!(self.authority))
        );
        assert_eq!(response.pointer("/receipt/state"), Some(&json!("accepted")));
        assert_eq!(
            response.pointer("/receipt/command_id"),
            Some(&json!(command_id))
        );
        response
    }

    fn observe(&self, command_id: &str, accepted: &Value) -> Value {
        let result_ref = accepted
            .pointer("/receipt/result_ref")
            .and_then(Value::as_str)
            .expect("durable reference returned by the real daemon");
        let response = self.run(&[
            "receipt",
            command_id,
            "--project-id",
            &self.project,
            "--result-ref",
            result_ref,
        ]);
        assert!(response.get("watermark").is_some());
        assert_eq!(
            response.pointer("/result/terminal_id"),
            Some(&json!(self.terminal))
        );
        response.get("result").expect("causal result").clone()
    }
}

fn configure_project(cli: &TerminalCli<'_>, fixture: &LiveFixture) {
    let created = cli.run(&[
        "project",
        "create",
        "--project-id",
        &cli.project,
        "--name",
        "Terminal CLI",
    ]);
    assert_eq!(created.get("new_revision"), Some(&json!(1)));
    let root = fixture.project_root.path().canonicalize().unwrap();
    let file = root.join("project.json");
    let mut definition = json!({
        "name": "Terminal CLI",
        "description": "Projet provisionné par le binaire CLI",
        "weight": 2,
        "primary_root_path": root,
        "sources": [{"kind": "directory", "path_or_uri": root, "label": "source", "read_only": false}],
        "repositories": [{"repo_id": "project-repo", "root_path": root, "default_branch": "dev"}],
        "settings_json": {}
    });
    std::fs::write(&file, definition.to_string()).unwrap();
    let command = Uuid::from_u128(0xf011_4704).to_string();
    let revise = [
        "project",
        "revise",
        &cli.project,
        "--expected-revision",
        "1",
        "--file",
        file.to_str().unwrap(),
        "--command-id",
        &command,
        "--authority-generation",
        &cli.authority,
    ];
    let applied = cli.run(&revise);
    assert_eq!(applied.pointer("/receipt/new_revision"), Some(&json!(2)));
    assert_eq!(cli.run(&revise), applied);
    let recorded = cli.run(&["project", "show", &cli.project]);
    assert_eq!(recorded.pointer("/result/revision"), Some(&json!(2)));
    assert_eq!(
        recorded.pointer("/result/primary_root_path"),
        Some(&json!(root))
    );
    assert_eq!(
        recorded.pointer("/result/sources/0/label"),
        Some(&json!("source"))
    );
    assert_eq!(
        recorded.pointer("/result/repositories/0/default_branch"),
        Some(&json!("dev"))
    );
    let malformed = [
        "project",
        "revise",
        &cli.project,
        "--expected-revision",
        "2",
        "--file",
        file.to_str().unwrap(),
    ];
    let mut missing_branch = definition.clone();
    missing_branch
        .pointer_mut("/repositories/0")
        .and_then(Value::as_object_mut)
        .unwrap()
        .remove("default_branch");
    std::fs::write(&file, missing_branch.to_string()).unwrap();
    cli.assert_refused(&malformed, "default_branch");
    definition
        .as_object_mut()
        .unwrap()
        .remove("primary_root_path");
    std::fs::write(&file, definition.to_string()).unwrap();
    cli.assert_refused(&malformed, "primary_root_path");
    std::fs::write(
        &file,
        r#"{"primary_root_path":null,"primary_root_path":null}"#,
    )
    .unwrap();
    cli.assert_refused(&malformed, "clés dupliquées");
    assert_eq!(
        cli.run(&["project", "show", &cli.project]).get("result"),
        recorded.get("result")
    );
}

fn configure_terminal_and_mission(cli: &TerminalCli<'_>, fixture: &LiveFixture) {
    let settings = cli.run(&["process", "settings"]);
    assert_eq!(settings.pointer("/result/revision"), Some(&json!(0)));
    assert_eq!(
        settings.pointer("/result/terminal_runtime"),
        Some(&Value::Null)
    );
    let file = fixture.project_root.path().join("terminal-runtime.json");
    let runtime = terminal_configuration();
    std::fs::write(&file, runtime.to_string()).unwrap();
    let command = Uuid::from_u128(0xf011_4705).to_string();
    let mut configure = [
        "process",
        "configure",
        "--expected-revision",
        "0",
        "--file",
        file.to_str().unwrap(),
        "--command-id",
        &command,
        "--authority-generation",
        &cli.authority,
    ];
    let applied = cli.run(&configure);
    assert_eq!(applied.pointer("/receipt/new_revision"), Some(&json!(1)));
    assert_eq!(cli.run(&configure), applied);
    let different_command = Uuid::from_u128(0xf011_4706).to_string();
    configure[7] = &different_command;
    cli.assert_refused(&configure, "RevisionConflict");
    let recorded = cli.run(&["process", "settings"]);
    assert_eq!(recorded.pointer("/result/revision"), Some(&json!(1)));
    assert_eq!(recorded.pointer("/result/terminal_runtime"), Some(&runtime));
    let assigned = cli.run(&[
        "mission",
        "assign",
        "--project-id",
        &cli.project,
        "--mission-id",
        &fixture.mission.to_string(),
    ]);
    assert_eq!(assigned.get("state"), Some(&json!("accepted")));
}

async fn wait_for_control(fixture: &mut LiveFixture, terminal: &str, revision: u64) {
    let deadline = Instant::now() + DEADLINE;
    let terminal = json!(terminal);
    loop {
        let snapshot = fixture.snapshot().await;
        if snapshot
            .pointer("/terminals/items")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("terminal_id") == Some(&terminal)
                        && item.get("lease_state").and_then(Value::as_str) == Some("held")
                        && item.get("control_revision").and_then(Value::as_u64) == Some(revision)
                })
            })
        {
            return;
        }
        assert!(Instant::now() < deadline, "control revision {revision}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
#[ignore = "fournir UNLEASH_TEST_CLI_BIN et le process-host construits depuis le même arbre"]
async fn t_f011_cli_acquires_observes_and_replays_the_same_terminal() {
    let binary = PathBuf::from(
        std::env::var_os("UNLEASH_TEST_CLI_BIN").expect("explicit absolute path to the real CLI"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let mut fixture = LiveFixture::empty().await;
    let cli = TerminalCli {
        binary: &binary,
        socket: fixture.rendezvous.path().join("daemon.sock"),
        project: fixture.project.to_string(),
        authority: fixture.authority.to_string(),
        terminal: Uuid::from_u128(0xf011_4701).to_string(),
    };
    configure_project(&cli, &fixture);
    configure_terminal_and_mission(&cli, &fixture);
    let allocation_command = Uuid::from_u128(0xf011_4702).to_string();
    let program = std::env::current_exe().unwrap();
    let mission = fixture.mission.to_string();
    let allocate = [
        "--mission-id",
        &mission,
        "--purpose-key",
        "real-cli-terminal",
        "--program",
        program.to_str().expect("UTF-8 fixture executable path"),
        "--environment-profile-id",
        "terminal-test",
        "--",
        "--ignored",
        "--exact",
        "reattach_scenarios::terminal_worker_entrypoint",
        "--nocapture",
    ];
    let accepted = cli.acquire("new", &allocation_command, &allocate);
    wait_for_control(&mut fixture, &cli.terminal, 0).await;
    let observed = cli.observe(&allocation_command, &accepted);
    assert_eq!(cli.acquire("new", &allocation_command, &allocate), accepted);
    let before = fixture.host_manifest();

    fixture.restart_daemon_only().await;
    let control_command = Uuid::from_u128(0xf011_4703).to_string();
    let process_run = observed
        .get("process_run_uuid")
        .and_then(Value::as_str)
        .expect("original process generation");
    let control = ["--process-run-uuid", process_run, "--control-revision", "0"];
    let accepted = cli.acquire("registered", &control_command, &control);
    wait_for_control(&mut fixture, &cli.terminal, 1).await;
    let recovered = cli.observe(&control_command, &accepted);
    assert_eq!(
        cli.acquire("registered", &control_command, &control),
        accepted
    );
    assert_eq!(recovered.get("process_run_uuid"), Some(&json!(process_run)));
    assert_eq!(recovered.get("control_revision"), Some(&json!(1)));
    let after = fixture.host_manifest();
    assert_eq!(after.host_id, before.host_id);
    assert_eq!(after.pid, before.pid);
    assert!(after.highest_fence > before.highest_fence);
    assert_eq!(
        std::fs::read_to_string(fixture.project_root.path().join("worker-starts")).unwrap(),
        "started\n"
    );
}
