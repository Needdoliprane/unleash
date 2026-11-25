//! REQ-TERM-003 : le secret de contrôle d'un hôte de processus ne dérive pas du générateur
//! d'identifiants, et une source d'aléa indisponible est un refus net rendu avant toute écriture
//! durable.
//!
//! T-F011-003. Les scénarios exécutent le chemin réel : démon en bibliothèque dans le processus de
//! test, IPC authentifié, `SQLite`, courtier d'actions et store réels. Seule la source d'aléa est
//! substituée, par le point d'entrée [`crate::terminal_service::start_with`].
#![allow(clippy::unwrap_used)]

use crate::ipc::DaemonRuntimeState;
use crate::terminal_host_launcher::TerminalHostLauncher;
use crate::terminal_service::{
    ControlSecretSource, SystemControlSecretSource, TerminalEffectError, start_with,
    terminal_effect_failure,
};
use crate::{AuthenticatedSession, DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};
use std::collections::HashSet;
use std::io::Stdio;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use unleash_action_broker::ActionExecutionFailure;
use unleash_client::{ClientConfig, UnleashClient};
use unleash_domain::{
    ActionState, ActorId, AuthorityGeneration, CommandId, DispatchId, ProjectId, RequestId,
    Revision, TerminalId,
};
use unleash_process_host::{HostControlSecret, ProcessHostError};
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{TerminalAllocationRecord, TerminalAllocationState};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DEADLINE: Duration = Duration::from_secs(20);
const PROJECT: ProjectId = ProjectId::from_u128(0xf011_0001);
const MISSION: Uuid = Uuid::from_u128(0xf011_0002);
const TERMINAL: Uuid = Uuid::from_u128(0xf011_0003);

#[test]
fn control_secret_does_not_follow_the_identifier_generator() {
    // Le générateur d'identifiants du démon donne une séquence reproductible : un secret qui en
    // dérive serait calculable par quiconque connaît la graine ou l'heure de démarrage.
    let source = SystemControlSecretSource;
    let first = source.generate().unwrap().expose_secret().to_owned();
    let second = source.generate().unwrap().expose_secret().to_owned();
    assert_ne!(
        first, second,
        "deux secrets de contrôle successifs doivent différer"
    );
    assert_eq!(first.len(), 64, "256 bits encodés en hexadécimal");
    assert!(
        !first.starts_with("terminal-control:"),
        "le secret de contrôle ne doit pas dériver du générateur d'identifiants"
    );
    assert!(
        first.chars().all(|c| c.is_ascii_hexdigit()),
        "le secret de contrôle est la forme hexadécimale de 32 octets aléatoires"
    );
}

/// Source d'aléa toujours indisponible : aucune lecture du système, aucun repli.
struct UnavailableRandomSource;

impl ControlSecretSource for UnavailableRandomSource {
    fn generate(&self) -> Result<HostControlSecret, ProcessHostError> {
        Err(ProcessHostError::RandomSourceUnavailable)
    }
}

/// Source indisponible qui relève l'état durable au moment précis du tirage.
struct PermitProbe {
    store: Arc<unleash_store::UnleashStore>,
    terminal: TerminalId,
    host_runtime: PathBuf,
    allocation_at_draw: Mutex<Option<TerminalAllocationState>>,
    host_runtime_at_draw: Mutex<bool>,
}

impl ControlSecretSource for PermitProbe {
    fn generate(&self) -> Result<HostControlSecret, ProcessHostError> {
        let allocation = self
            .store
            .inspect_terminal_allocation_snapshot(self.terminal)
            .ok()
            .and_then(|snapshot| snapshot.value)
            .map(|record| record.state);
        *self
            .allocation_at_draw
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = allocation;
        *self
            .host_runtime_at_draw
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = self.host_runtime.exists();
        Err(ProcessHostError::RandomSourceUnavailable)
    }
}

/// Un démon réel par test, avec projet, mission et configuration terminal nominale.
struct LaunchFixture {
    daemon: DaemonHandle,
    runtime: Arc<DaemonRuntimeState>,
    session: AuthenticatedSession,
    authority: AuthorityGeneration,
    _client: UnleashClient,
    /// Racines de la fixture : elles doivent vivre autant que le démon.
    _directories: Vec<tempfile::TempDir>,
}

impl LaunchFixture {
    async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        Self::start_with(Arc::new(
            crate::terminal_host_launcher::PackagedTerminalHostLauncher,
        ))
        .await
    }

    async fn start_with(
        launcher: Arc<dyn crate::terminal_host_launcher::TerminalHostLauncher>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let rendezvous = unleash_testkit::local_test_dir("req-term-003-rdv")?;
        let data = unleash_testkit::local_test_dir("req-term-003-data")?;
        let project_root = unleash_testkit::local_test_dir("req-term-003-project")?;
        std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700))?;
        let data_root = data.path().to_path_buf();
        let socket = rendezvous.path().join("daemon.sock");
        let config = DaemonConfig {
            data_root: Some(data_root),
            rendezvous_dir: Some(rendezvous.path().to_path_buf()),
            terminal_host_launcher: launcher,
            ..DaemonConfig::default()
        };
        let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config)
            .await
            .map_err(|e| format!("{e:?}"))?
        else {
            return Err("la fixture doit démarrer sa propre autorité".into());
        };
        daemon.wait_ready().await.map_err(|e| format!("{e:?}"))?;
        let runtime = daemon.test_runtime_state();
        let mut client =
            UnleashClient::connect(ClientConfig::new(socket, ClientKind::Cli, "req-term-003"))
                .await
                .map_err(|e| format!("{e:?}"))?;
        let authority = client.hello().authority_generation;
        let project_path = project_root.path().canonicalize()?;
        configure_nominal_terminal(
            &mut client,
            project_path.to_str().ok_or("chemin de projet non UTF-8")?,
        )
        .await?;
        let actor = ActorId::new(format!("uid:{}", unleash_platform::effective_uid()))?;
        let session = AuthenticatedSession {
            peer_creds: unleash_platform::PeerCredentials {
                uid: unleash_platform::effective_uid(),
                gid: 0,
                pid: None,
            },
            client_kind: ClientKind::Cli,
            actor_ref: actor.to_string(),
            actor_id: actor,
            capabilities: HashSet::new(),
            allowed_projects: None,
        };
        Ok(Self {
            daemon,
            runtime,
            session,
            authority,
            _client: client,
            _directories: vec![rendezvous, data, project_root],
        })
    }

    /// Lâche les références au store puis arrête le démon : l'arrêt attend que le store n'ait
    /// plus de propriétaire actif.
    async fn finish(self, reason: &str) -> Result<(), Box<dyn std::error::Error>> {
        let Self {
            daemon,
            runtime,
            session,
            _client,
            _directories,
        } = self;
        drop(runtime);
        drop(_client);
        drop(session);
        daemon
            .shutdown_graceful(reason)
            .await
            .map_err(|e| format!("{e:?}"))?;
        drop(_directories);
        Ok(())
    }

    /// Prépare et commet l'allocation sans lancer l'hôte : le lancement revient au test.
    async fn allocate(
        &self,
        readiness_timeout_ms: u64,
    ) -> Result<TerminalId, Box<dyn std::error::Error>> {
        let terminal = TerminalId::from_uuid(TERMINAL);
        let acquire = command(
            self.authority,
            0xf011_0015,
            TERMINAL,
            0,
            "AcquireTerminal",
            serde_json::json!({"mission_id":MISSION, "invocation_id":null,
                "purpose_key":"fixture-terminal", "reuse_policy":"reuse_compatible",
                "scope_id":PROJECT.to_string(), "scope_revision":2, "worktree_id":null,
                "cwd":".", "program":"/usr/bin/printf", "arguments":["terminal-ready\n"],
                "environment_profile_id":"terminal-test", "pty":{"rows":24,"cols":80},
                "readiness":{"kind":"process_started","timeout_ms":readiness_timeout_ms}}),
        )?;
        let request =
            crate::terminal_service::prepare_acquisition(&self.runtime, &self.session, acquire)
                .await
                .map_err(|e| format!("préparation de l'acquisition refusée : {e:?}"))?;
        let store = Arc::clone(&self.runtime.store);
        tokio::task::spawn_blocking(move || store.commit_command(request))
            .await?
            .map_err(|e| format!("commit de l'allocation refusé : {e:?}"))?;
        Ok(terminal)
    }

    /// Relit l'allocation une fois l'action du lancement dans un état terminal.
    async fn settled(
        &self,
        terminal: TerminalId,
    ) -> Result<TerminalAllocationRecord, Box<dyn std::error::Error>> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let store = Arc::clone(&self.runtime.store);
            let record = tokio::task::spawn_blocking(move || {
                store
                    .inspect_terminal_allocation_snapshot(terminal)
                    .map(|snapshot| snapshot.value)
            })
            .await?
            .map_err(|e| format!("relecture de l'allocation : {e:?}"))?
            .ok_or("allocation absente")?;
            if matches!(
                record.action_state,
                Some(
                    ActionState::ObservedFailure
                        | ActionState::OutcomeUnknown
                        | ActionState::ObservedSuccess
                )
            ) {
                return Ok(record);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "l'action du lancement n'a pas d'issue après {DEADLINE:?} : {:?}",
                    record.action_state
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn host_runtime(&self, record: &TerminalAllocationRecord) -> PathBuf {
        self.runtime
            .store
            .config()
            .data_root
            .join("process-hosts")
            .join(record.context.host_id.to_string())
    }
}

fn command(
    authority: AuthorityGeneration,
    id: u128,
    aggregate: Uuid,
    revision: u64,
    name: &str,
    payload: serde_json::Value,
) -> Result<CommandEnvelope, Box<dyn std::error::Error>> {
    let payload = payload
        .as_object()
        .ok_or("la charge utile de fixture doit être un objet")?
        .clone();
    Ok(CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: authority,
        actor_ref: None,
        project_id: Some(PROJECT),
        aggregate_id: aggregate,
        expected_revision: Revision::new(revision),
        command_type: name.into(),
        payload,
    })
}

async fn send(
    client: &mut UnleashClient,
    command: &CommandEnvelope,
) -> Result<(), Box<dyn std::error::Error>> {
    let receipt = tokio::time::timeout(DEADLINE, client.send_command(command))
        .await?
        .map_err(|e| format!("commande de fixture refusée : {e:?}"))?;
    assert_eq!(receipt.state, ReceiptState::Accepted);
    Ok(())
}

async fn configure_nominal_terminal(
    client: &mut UnleashClient,
    project_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let authority = client.hello().authority_generation;
    let setup = [
        command(
            authority,
            0xf011_0011,
            PROJECT.as_uuid(),
            0,
            "CreateProject",
            serde_json::json!({"name":"F011 fixture", "weight":1}),
        )?,
        command(
            authority,
            0xf011_0012,
            PROJECT.as_uuid(),
            1,
            "ReviseProject",
            serde_json::json!({"name":"F011 fixture", "description":"", "weight":1,
                "primary_root_path":project_path,
                "sources":[{"kind":"directory", "path_or_uri":project_path,
                            "label":"fixture", "read_only":false}],
                "repositories":[], "settings_json":{}}),
        )?,
        command(
            authority,
            0xf011_0013,
            MISSION,
            0,
            "AssignMission",
            serde_json::json!({}),
        )?,
    ];
    for cmd in &setup {
        send(client, cmd).await?;
    }
    let mut configuration = command(
        authority,
        0xf011_0016,
        authority.as_uuid(),
        0,
        "SetSettings",
        serde_json::json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":{
            "schema_version":1,"profiles":[{"profile_id":"terminal-test","version":1,
                "variables":{"LANG":"C","TERM":"xterm"}}],
            "max_hosts":2,"max_processes":2,"max_ptys":2,"lease_ttl_ms":60000
        }}}),
    )?;
    configuration.project_id = None;
    send(client, &configuration).await
}

// T-F011-003 / AC-F011-003 / REQ-TERM-003
// Propriété (b) : le refus survient avant la résolution du permis à usage unique.
#[tokio::test]
async fn t_f011_003_launch_draws_the_control_secret_before_the_launch_permit() -> TestResult {
    let fixture = LaunchFixture::start().await?;
    let terminal = fixture.allocate(10_000).await?;
    let host_root = fixture
        .runtime
        .store
        .config()
        .data_root
        .join("process-hosts");
    assert!(
        !host_root.exists(),
        "précondition : aucun répertoire d'hôte avant le lancement"
    );
    let probe = Arc::new(PermitProbe {
        store: Arc::clone(&fixture.runtime.store),
        terminal,
        host_runtime: host_root,
        host_runtime_at_draw: Mutex::new(false),
        allocation_at_draw: Mutex::new(None),
    });
    start_with(&fixture.runtime, terminal, probe.clone());
    let settled = fixture.settled(terminal).await?;
    let allocation_at_draw = *probe
        .allocation_at_draw
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let host_root_at_draw = *probe
        .host_runtime_at_draw
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        allocation_at_draw,
        Some(TerminalAllocationState::Allocated),
        "le permis doit être intact au moment du tirage ; action finale : {:?}",
        settled.action_state
    );
    assert!(
        !host_root_at_draw,
        "aucun répertoire d'hôte ne doit exister au moment du tirage"
    );
    // La sonde détient le store : elle doit le lâcher avant l'arrêt du démon.
    drop(probe);
    fixture.finish("req-term-003 ordre").await?;
    Ok(())
}

// T-F011-003 / AC-F011-003 / REQ-TERM-003
// Propriété (a) : une source d'aléa indisponible est un refus certain, sans aucune écriture durable.
#[tokio::test]
async fn t_f011_003_unavailable_random_source_refuses_the_launch_without_durable_write()
-> TestResult {
    let fixture = LaunchFixture::start().await?;
    let terminal = fixture.allocate(10_000).await?;
    let before = tokio::task::spawn_blocking({
        let store = Arc::clone(&fixture.runtime.store);
        move || store.inspect_terminal_allocation_snapshot(terminal)
    })
    .await?
    .map_err(|e| format!("{e:?}"))?
    .value
    .ok_or("allocation absente avant le lancement")?;
    assert!(matches!(before.state, TerminalAllocationState::Allocated));
    start_with(
        &fixture.runtime,
        terminal,
        Arc::new(UnavailableRandomSource),
    );
    let settled = fixture.settled(terminal).await?;
    // Allocation, action et permis relus après l'échec.
    assert_eq!(
        settled.revision, before.revision,
        "l'allocation n'est pas modifiée"
    );
    assert_eq!(
        settled.dispatch_id, None,
        "le permis à usage unique est intact"
    );
    assert!(
        matches!(settled.state, TerminalAllocationState::Allocated),
        "le lancement n'est pas résolu : {:?}",
        settled.state
    );
    assert_eq!(
        settled.action_state,
        Some(ActionState::ObservedFailure),
        "le refus est certain, pas une issue à réconcilier"
    );
    assert!(
        !fixture.host_runtime(&settled).exists(),
        "aucun répertoire ni fichier d'hôte n'est créé"
    );
    fixture.finish("req-term-003 refus").await?;
    Ok(())
}

/// Lance un processus court qui ne sera jamais un hôte : ni socket, ni manifeste.
struct SilentHostLauncher;

impl TerminalHostLauncher for SilentHostLauncher {
    fn spawn(&self, _executable: &Path, _bootstrap: &Path) -> std::io::Result<Child> {
        let mut command = Command::new("/bin/sleep");
        command
            .arg("2")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unleash_process_host::configure_setsid_pre_exec(&mut command);
        command.spawn()
    }
}

// T-F011-003 / AC-F011-003 / REQ-TERM-003
// Propriété (c) : une erreur survenue après l'envoi à l'hôte reste une issue inconnue.
#[tokio::test]
async fn t_f011_003_launch_failure_after_host_creation_stays_outcome_unknown() -> TestResult {
    let fixture = LaunchFixture::start_with(Arc::new(SilentHostLauncher)).await?;
    let terminal = fixture.allocate(300).await?;
    start_with(
        &fixture.runtime,
        terminal,
        Arc::new(SystemControlSecretSource),
    );
    let settled = fixture.settled(terminal).await?;
    assert!(
        settled.dispatch_id.is_some(),
        "précondition : le permis a été consommé, l'hôte a été créé"
    );
    assert!(
        fixture.host_runtime(&settled).is_dir(),
        "précondition : les fichiers de l'hôte existent"
    );
    assert_eq!(
        settled.action_state,
        Some(ActionState::OutcomeUnknown),
        "l'impossibilité d'observer un hôte créé n'est jamais un refus certain"
    );
    fixture.finish("req-term-003 après envoi").await?;
    Ok(())
}

// T-F011-003 / AC-F011-003 / REQ-TERM-003
// L'échec rendu au courtier conserve la cause typée d'un refus avant envoi, et ne transforme jamais
// une erreur survenue à partir de l'envoi en refus certain.
#[test]
fn t_f011_003_effect_failure_keeps_the_cause_of_a_refusal_before_host_dispatch() {
    let dispatch = DispatchId::from_u128(0xf011_00a1);
    let refused = terminal_effect_failure(
        dispatch,
        "launch",
        TerminalEffectError::BeforeSend(ProcessHostError::RandomSourceUnavailable.into()),
    );
    assert!(
        matches!(
            refused,
            ActionExecutionFailure::ConfirmedFailureNoEffect { ref dispatch_id, ref reason }
                if dispatch_id == dispatch
                    && reason.contains("RANDOM_SOURCE_UNAVAILABLE")
                    && reason.contains("before host dispatch")
        ),
        "un refus avant envoi est un échec certain qui nomme sa cause typée"
    );
    let unknown = terminal_effect_failure(
        dispatch,
        "input",
        TerminalEffectError::AfterSend("connexion fermée".into()),
    );
    assert!(
        matches!(
            unknown,
            ActionExecutionFailure::OutcomeUnknown { ref dispatch_id, ref reason }
                if dispatch_id == dispatch
                    && reason == "Terminal input requires reconciliation"
        ),
        "une erreur à partir de l'envoi reste une issue inconnue, jamais un refus certain"
    );
}
