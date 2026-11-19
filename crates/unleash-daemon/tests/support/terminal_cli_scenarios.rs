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

    fn log(&self, options: &[&str]) -> Value {
        let mut args = vec![
            "process",
            "log",
            "--project-id",
            &self.project,
            "--terminal-id",
            &self.terminal,
        ];
        args.extend_from_slice(options);
        self.run(&args)
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

async fn wait_for_log_marker(cli: &TerminalCli<'_>) -> String {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let reply = cli.log(&[]);
        assert!(reply.get("watermark").is_some_and(|value| !value.is_null()));
        let text = reply
            .pointer("/result/text")
            .and_then(Value::as_str)
            .unwrap();
        if let Some(offset) = text.find("live-terminal-ready") {
            assert!(text.is_ascii(), "owned fixture writes only ASCII");
            assert_eq!(
                reply.pointer("/result/log/segments/0/start_offset"),
                Some(&json!(0))
            );
            return offset.to_string();
        }
        assert!(Instant::now() < deadline, "owned PTY startup output");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn assert_log_pagination(cli: &TerminalCli<'_>, offset: &str) {
    let page = cli.log(&["--after-offset", offset, "--max-bytes", "4"]);
    assert_eq!(page.pointer("/result/text"), Some(&json!("live")));
    assert_eq!(
        page.pointer("/result/log/segments/0/start_offset")
            .and_then(Value::as_u64),
        Some(offset.parse().unwrap())
    );
    let next_offset = page
        .pointer("/result/log/segments")
        .and_then(Value::as_array)
        .and_then(|segments| segments.last())
        .and_then(|last| last.get("end_offset"))
        .and_then(Value::as_u64)
        .expect("byte offset of the last returned segment");
    let next = cli.log(&[
        "--after-offset",
        &next_offset.to_string(),
        "--max-bytes",
        "4",
    ]);
    assert_eq!(next.pointer("/result/text"), Some(&json!("-ter")));
}

fn assert_log_scope_and_limits(cli: &TerminalCli<'_>) {
    let wrong_project = Uuid::from_u128(0xf011_47ff).to_string();
    for (project, max_bytes) in [
        (&wrong_project, "4"),
        (&cli.project, "0"),
        (&cli.project, "65537"),
    ] {
        cli.assert_refused(
            &[
                "process",
                "log",
                "--project-id",
                project,
                "--terminal-id",
                &cli.terminal,
                "--max-bytes",
                max_bytes,
            ],
            "NotFoundOrHidden",
        );
    }
    let empty = cli.log(&["--after-offset", "9007199254740991"]);
    assert_eq!(empty.pointer("/result/log/segments"), Some(&json!([])));
    assert_eq!(empty.pointer("/result/text"), Some(&json!("")));
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
    let marker_offset = wait_for_log_marker(&cli).await;
    assert_log_pagination(&cli, &marker_offset);
    assert_log_scope_and_limits(&cli);
    let after_read = cli.observe(&allocation_command, &accepted);
    assert_eq!(
        after_read.get("lease_expires_at"),
        observed.get("lease_expires_at")
    );
    assert_eq!(after_read.get("fence"), observed.get("fence"));
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
    assert_log_pagination(&cli, &marker_offset);
    assert_eq!(
        std::fs::read_to_string(fixture.project_root.path().join("worker-starts")).unwrap(),
        "started\n"
    );
}

mod input_cli_scenarios {
    use super::*;
    use std::io::{BufRead, Write};

    const WORKER: &str = "reattach_scenarios::cli_scenarios::input_cli_scenarios::cli_input_worker";
    const INPUT: &[u8] = b"PAYLOAD-ONLY\0\xff\n";

    fn input_args(cli: &TerminalCli<'_>, observed: &Value, file: &Path) -> Vec<String> {
        let mut args = [
            "process",
            "input",
            "--project-id",
            &cli.project,
            "--terminal-id",
            &cli.terminal,
            "--process-run-uuid",
            observed["process_run_uuid"].as_str().unwrap(),
            "--lease-id",
            observed["lease_id"].as_str().unwrap(),
            "--authority-generation",
            &cli.authority,
            "--command-id",
            "00000000-0000-4000-8000-0000f0114802",
            "--file",
            file.to_str().unwrap(),
        ]
        .map(str::to_owned)
        .to_vec();
        for (flag, field, increment) in [
            ("--fence", "fence", 0),
            ("--control-revision", "control_revision", 0),
            ("--input-seq", "input_seq", 1),
        ] {
            args.push(flag.into());
            args.push(
                observed[field]
                    .as_u64()
                    .unwrap()
                    .checked_add(increment)
                    .unwrap()
                    .to_string(),
            );
        }
        args
    }

    fn replace(args: &mut [String], flag: &str, value: &str) {
        let index = args
            .iter()
            .position(|item| item == flag)
            .unwrap()
            .checked_add(1)
            .unwrap();
        *args.get_mut(index).unwrap() = value.to_owned();
    }

    fn invoke(cli: &TerminalCli<'_>, args: &[String]) -> std::process::Output {
        cli.invoke(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn input_counts(fixture: &LiveFixture) -> (i64, i64, i64) {
        let conn = rusqlite::Connection::open_with_flags(
            fixture.data.path().join("unleash-authority.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        conn.query_row("SELECT (SELECT COUNT(*) FROM terminal_input_intents),(SELECT COUNT(*) FROM actions WHERE action_type='terminal.send_input'),(SELECT COUNT(*) FROM action_dispatches d JOIN actions a ON a.action_id=d.action_id WHERE a.action_type='terminal.send_input')", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
    }

    fn assert_local_bounds(cli: &TerminalCli<'_>, fixture: &LiveFixture, observed: &Value) {
        for (name, bytes) in [
            ("empty-input", Vec::new()),
            ("oversized-input", vec![0; 65_537]),
        ] {
            let file = fixture.project_root.path().join(name);
            std::fs::write(&file, bytes).unwrap();
            let output = invoke(cli, &input_args(cli, observed, &file));
            assert!(!output.status.success());
            let error: Value = serde_json::from_slice(&output.stderr).unwrap();
            assert!(
                error["error"].as_str().unwrap().contains("1..=65536"),
                "bounded local refusal: {error}"
            );
            assert!(
                error.get("code").is_none(),
                "local bounds precede a remote mutation"
            );
            assert_eq!(input_counts(fixture), (0, 0, 0));
        }
    }

    fn assert_remote_fence_and_sequence(
        cli: &TerminalCli<'_>,
        fixture: &LiveFixture,
        args: &[String],
    ) {
        for (flag, identity) in [("--fence", 0xf011_48f1), ("--input-seq", 0xf011_48f2)] {
            let mut invalid = args.to_vec();
            let current = args
                .iter()
                .position(|item| item == flag)
                .unwrap()
                .checked_add(1)
                .unwrap();
            let different = args
                .get(current)
                .unwrap()
                .parse::<u64>()
                .unwrap()
                .checked_add(1)
                .unwrap();
            replace(&mut invalid, flag, &different.to_string());
            replace(
                &mut invalid,
                "--command-id",
                &Uuid::from_u128(identity).to_string(),
            );
            let output = invoke(cli, &invalid);
            assert!(!output.status.success());
            let error: Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(
                error.get("code"),
                Some(&json!("ContractViolation")),
                "writer refuses the mismatching {flag}: {error}"
            );
            assert_eq!(input_counts(fixture), (0, 0, 0));
        }
    }

    async fn wait_known(fixture: &LiveFixture) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let conn = rusqlite::Connection::open_with_flags(
                fixture.data.path().join("unleash-authority.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let known: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM terminal_input_intents i JOIN actions a ON a.action_id=i.action_id WHERE i.state='observed' AND a.state='observed_success' AND i.observation_object_hash IS NOT NULL)",[],|row|row.get(0)).unwrap();
            let effect = match std::fs::read(fixture.project_root.path().join("cli-input-effects"))
            {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(error) => panic!("owned effect file: {error}"),
            };
            if known && effect == b"effect\n" {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "native receipt and fsynced effect must both be observed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn fence_old_controller(fixture: &mut LiveFixture, cli: &TerminalCli<'_>, known: &Value) {
        let before = fixture.host_manifest();
        // Kill only the owned daemon after the explicit replays. Its old socket
        // can no longer schedule a later input. The existing transfer gate
        // serializes native writes and invalidates all old epoch/fence headers.
        fixture.restart_daemon_only().await;
        let command = Uuid::from_u128(0xf011_4804).to_string();
        let revision = known["control_revision"].as_u64().unwrap().to_string();
        let accepted = cli.acquire(
            "registered",
            &command,
            &[
                "--process-run-uuid",
                known["process_run_uuid"].as_str().unwrap(),
                "--control-revision",
                &revision,
            ],
        );
        wait_for_control(
            fixture,
            &cli.terminal,
            known["control_revision"]
                .as_u64()
                .unwrap()
                .checked_add(1)
                .unwrap(),
        )
        .await;
        let transferred = cli.observe(&command, &accepted);
        assert_eq!(
            transferred["input_seq"], 1,
            "fresh native summary after fencing the old controller"
        );
        assert_eq!(transferred["process_run_uuid"], known["process_run_uuid"]);
        let after = fixture.host_manifest();
        assert_eq!(after.host_id, before.host_id);
        assert_eq!(after.pid, before.pid);
        assert!(after.highest_fence > before.highest_fence);
    }

    #[tokio::test]
    #[ignore = "explicit UNLEASH_TEST_CLI_BIN plus daemon/host built from this tree"]
    async fn t_f011_cli_input_uses_observed_control_and_replays_one_binary_effect() {
        let binary = PathBuf::from(std::env::var_os("UNLEASH_TEST_CLI_BIN").unwrap());
        assert!(binary.is_absolute() && binary.is_file());
        let mut fixture = LiveFixture::empty().await;
        let cli = TerminalCli {
            binary: &binary,
            socket: fixture.rendezvous.path().join("daemon.sock"),
            project: fixture.project.to_string(),
            authority: fixture.authority.to_string(),
            terminal: Uuid::from_u128(0xf011_4801).to_string(),
        };
        configure_project(&cli, &fixture);
        configure_terminal_and_mission(&cli, &fixture);
        let program = std::env::current_exe().unwrap();
        let command = Uuid::from_u128(0xf011_4803).to_string();
        let accepted = cli.acquire(
            "new",
            &command,
            &[
                "--mission-id",
                &fixture.mission.to_string(),
                "--purpose-key",
                "cli-binary-input",
                "--program",
                program.to_str().unwrap(),
                "--environment-profile-id",
                "terminal-test",
                "--",
                "--ignored",
                "--exact",
                WORKER,
                "--nocapture",
            ],
        );
        wait_for_control(&mut fixture, &cli.terminal, 0).await;
        wait_for_log_marker(&cli).await;
        let observed = cli.observe(&command, &accepted);
        assert_eq!(observed.get("input_seq"), Some(&json!(0)));
        let help = cli.invoke(&["process", "input", "--help"]);
        assert!(
            help.status.success(),
            "real CLI process input is absent: {}",
            String::from_utf8_lossy(&help.stderr)
        );
        assert_local_bounds(&cli, &fixture, &observed);
        let file = fixture.project_root.path().join("input.bin");
        std::fs::write(&file, INPUT).unwrap();
        let args = input_args(&cli, &observed, &file);
        assert_remote_fence_and_sequence(&cli, &fixture, &args);
        let sent = invoke(&cli, &args);
        assert!(
            sent.status.success(),
            "real CLI input: {}",
            String::from_utf8_lossy(&sent.stderr)
        );
        let receipt: Value = serde_json::from_slice(&sent.stdout).unwrap();
        assert_eq!(receipt.pointer("/receipt/state"), Some(&json!("accepted")));
        assert_eq!(
            receipt.pointer("/receipt/command_id"),
            Some(&json!("00000000-0000-4000-8000-0000f0114802"))
        );
        assert!(!String::from_utf8_lossy(&sent.stdout).contains("PAYLOAD-ONLY"));
        assert!(!String::from_utf8_lossy(&sent.stderr).contains("PAYLOAD-ONLY"));
        wait_known(&fixture).await;
        let known = cli.observe(&command, &accepted);
        assert_eq!(known.get("input_seq"), Some(&json!(1)));
        assert_eq!(known.get("lease_id"), observed.get("lease_id"));
        for _ in 0..3 {
            let replay = invoke(&cli, &args);
            assert!(replay.status.success());
            assert_eq!(
                serde_json::from_slice::<Value>(&replay.stdout).unwrap(),
                receipt
            );
        }
        fence_old_controller(&mut fixture, &cli, &known).await;
        let replay = invoke(&cli, &args);
        assert!(
            replay.status.success(),
            "the old exact command remains an inspection, not a new permission"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&replay.stdout).unwrap(),
            receipt
        );
        assert_eq!(input_counts(&fixture), (1, 1, 1));
        assert_eq!(
            std::fs::read(fixture.project_root.path().join("cli-input-effects")).unwrap(),
            b"effect\n"
        );
        assert_eq!(
            std::fs::read(fixture.project_root.path().join("worker-starts")).unwrap(),
            b"started\n"
        );
    }

    #[test]
    #[ignore = "owned native PTY worker selected only by exact fixture argv"]
    fn cli_input_worker() {
        let mut starts = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("worker-starts")
            .unwrap();
        starts.write_all(b"started\n").unwrap();
        starts.sync_all().unwrap();
        println!("live-terminal-ready");
        std::io::stdout().flush().unwrap();
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            if reader.read_until(b'\n', &mut bytes).unwrap() == 0 {
                break;
            }
            assert_eq!(bytes, INPUT, "native child receives the exact binary input");
            let mut effect = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("cli-input-effects")
                .unwrap();
            effect.write_all(b"effect\n").unwrap();
            effect.sync_all().unwrap();
        }
    }
}
