//! Ports et traits abstraits pour l'injection de dépendances (horloge, générateurs d'identifiants).
//!
//! Permet à `unleash-domain` de demeurer une bibliothèque pure sans I/O :
//! aucune horloge système directe (`std::time::SystemTime::now`), aucun aléa implicite (`thread_rng`).

use uuid::Uuid;

use crate::fences::TimestampMs;
use crate::ids::{ActionId, AgentId, CommandId, InvocationId, RunId, SessionId, StepId, TaskId};

/// Port abstrait fournissant le temps courant en millisecondes UTC.
pub trait ClockPort: Send + Sync {
    /// Renvoie l'horodatage actuel en millisecondes UTC.
    fn now_ms(&self) -> TimestampMs;
}

/// Port abstrait pour la génération d'identifiants uniques.
pub trait IdGeneratorPort: Send + Sync {
    /// Génère un nouvel UUID aléatoire.
    fn new_uuid(&self) -> Uuid;

    /// Génère un nouveau `CommandId`.
    fn new_command_id(&self) -> CommandId {
        CommandId::from_uuid(self.new_uuid())
    }

    /// Génère un nouveau `RunId`.
    fn new_run_id(&self) -> RunId {
        RunId::from_uuid(self.new_uuid())
    }

    /// Génère un nouveau `StepId`.
    fn new_step_id(&self) -> StepId {
        StepId::from_uuid(self.new_uuid())
    }

    /// Génère un nouveau `TaskId`.
    fn new_task_id(&self) -> TaskId {
        TaskId::from_uuid(self.new_uuid())
    }

    /// Génère un nouveau `AgentId`.
    fn new_agent_id(&self) -> AgentId {
        AgentId::from_uuid(self.new_uuid())
    }

    /// Génère un nouveau `SessionId`.
    fn new_session_id(&self) -> SessionId {
        SessionId::from_uuid(self.new_uuid())
    }

    /// Génère un nouveau `InvocationId`.
    fn new_invocation_id(&self) -> InvocationId {
        InvocationId::from_uuid(self.new_uuid())
    }

    /// Génère un nouveau `ActionId`.
    fn new_action_id(&self) -> ActionId {
        ActionId::from_uuid(self.new_uuid())
    }
}
