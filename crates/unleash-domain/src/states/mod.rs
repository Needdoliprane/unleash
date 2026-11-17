//! Machines d'état et types du cycle de vie opérationnel d'Unleash.
//!
//! Expose l'ensemble des états et fonctions pures de transition :
//! - Mode du daemon (`DaemonMode`, `DaemonStartupIssue`, `StopOutcome`)
//! - Objectifs et missions (`GoalState`, `MissionState`, `CompletionPolicy`)
//! - Exécutions de workflow (`RunState`, `GuardTruth`, `RunEvent`)
//! - Étapes de workflow (`StepState`)
//! - Agents logiques (`AgentLevel`, `AgentState`)
//! - Invocations et tentatives (`InvocationState`, `AttemptState`, `WaitReason`, `AdmissionClass`)
//! - Actions et réconciliation (`ActionState`, `ActionOrigin`, `ReconcileStrategy`, `ReconcileOutcome`, `RecoveryClassification`, `GuardMode`, `ActionEvent`)
//! - Politiques et publications (`PublicationState`, `ModelPolicy`, `DataLocality`)

pub mod action;
pub mod agent;
pub mod daemon;
pub mod invocation;
pub mod mission;
pub mod policy;
pub mod run;
pub mod step;

pub use action::{
    ActionEvent, ActionOrigin, ActionState, GuardMode, ReconcileOutcome, ReconcileStrategy,
    RecoveryClassification, transition_action,
};
pub use agent::{AgentLevel, AgentState, transition_agent_state};
pub use daemon::{DaemonMode, DaemonStartupIssue, StopOutcome, transition_daemon_mode};
pub use invocation::{
    AdmissionClass, AttemptState, InvocationState, WaitReason, transition_attempt,
    transition_invocation_state,
};
pub use mission::{
    CompletionPolicy, GoalState, MissionState, transition_goal_state, transition_mission_state,
};
pub use policy::{DataLocality, ModelPolicy, PublicationState, transition_publication_state};
pub use run::{GuardTruth, RunEvent, RunState, transition_run, validate_predecessor_run};
pub use step::{StepState, transition_step};
