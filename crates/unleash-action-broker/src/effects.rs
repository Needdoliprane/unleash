// SPDX-License-Identifier: Apache-2.0
//! Liste fermée et versionnée des 11 classes d'effets ([`EffectsClass`]), sujets typés d'action
//! et structure d'intention durable ([`ActionIntent`]) (`F017/interfaces.md`, `ai-guard.md`).

use core::fmt;
use core::str::FromStr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActorId, AgentId, AttemptId, DataLocality, InvocationId, LineageFence, MissionId,
    ProjectId, ReconcileStrategy, ReplayWorkerId, Revision, SecretRef, Sha256Digest, TerminalId,
    TimestampMs,
};
use unleash_protocol::canonical_sha256;

use crate::error::ActionBrokerError;
use crate::isolation::IsolationRequirements;

/// Liste fermée versionnée des 11 classes d'effets reconnues par l'`ActionBroker`.
///
/// Toute valeur inconnue est rejetée avant [`crate::GuardPort`] avec
/// `InvalidArgument/EFFECTS_CLASS_UNKNOWN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectsClass {
    /// Lecture locale de fichier/ressource dans le scope autorisé (seule classe non transmise à `GuardPort`).
    LocalRead,
    /// Écriture locale de fichier dans le scope autorisé.
    LocalWrite,
    /// Exécution de processus enfant ou script de projet.
    ProcessExec,
    /// Envoi d'entrée vers un terminal PTY interactif.
    TerminalInput,
    /// Mutation Git locale (commit, branche, worktree local).
    GitLocalWrite,
    /// Interaction Git distante (fetch/push vers remote autorisé).
    GitRemote,
    /// Lecture via connecteur documentaire (Drive, `OneDrive`, Notion).
    ConnectorRead,
    /// Écriture via connecteur externe.
    ConnectorWrite,
    /// Interaction navigateur médiée ou scriptée.
    BrowserAction,
    /// Appel sortant vers un modèle LLM local ou distant.
    ModelDispatch,
    /// Requête réseau sortante contrôlée.
    NetworkEgress,
}

impl EffectsClass {
    /// Identifiant de version du mapping fermé des classes d'effets.
    pub const MAPPING_VERSION: &'static str = "unleash.effects_class.v1";

    /// Tableau exhaustif des 11 classes d'effets V1.
    pub const ALL: [Self; 11] = [
        Self::LocalRead,
        Self::LocalWrite,
        Self::ProcessExec,
        Self::TerminalInput,
        Self::GitLocalWrite,
        Self::GitRemote,
        Self::ConnectorRead,
        Self::ConnectorWrite,
        Self::BrowserAction,
        Self::ModelDispatch,
        Self::NetworkEgress,
    ];

    /// Représentation textuelle canonique en `snake_case`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::LocalRead => "local_read",
            Self::LocalWrite => "local_write",
            Self::ProcessExec => "process_exec",
            Self::TerminalInput => "terminal_input",
            Self::GitLocalWrite => "git_local_write",
            Self::GitRemote => "git_remote",
            Self::ConnectorRead => "connector_read",
            Self::ConnectorWrite => "connector_write",
            Self::BrowserAction => "browser_action",
            Self::ModelDispatch => "model_dispatch",
            Self::NetworkEgress => "network_egress",
        }
    }

    /// Analyse strictement une chaîne brute ; refuse toute classe hors des 11 variantes fermées.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::EffectsClassUnknown`] (`InvalidArgument/EFFECTS_CLASS_UNKNOWN`).
    pub fn from_wire_str(raw: &str) -> Result<Self, ActionBrokerError> {
        match raw {
            "local_read" => Ok(Self::LocalRead),
            "local_write" => Ok(Self::LocalWrite),
            "process_exec" => Ok(Self::ProcessExec),
            "terminal_input" => Ok(Self::TerminalInput),
            "git_local_write" => Ok(Self::GitLocalWrite),
            "git_remote" => Ok(Self::GitRemote),
            "connector_read" => Ok(Self::ConnectorRead),
            "connector_write" => Ok(Self::ConnectorWrite),
            "browser_action" => Ok(Self::BrowserAction),
            "model_dispatch" => Ok(Self::ModelDispatch),
            "network_egress" => Ok(Self::NetworkEgress),
            other => Err(ActionBrokerError::EffectsClassUnknown {
                raw_class: other.to_owned(),
            }),
        }
    }

    /// Indique si cette classe d'effet doit être évaluée par [`crate::GuardPort`] lorsqu'AI-Guard
    /// est configuré (`ADR-10` R7 : toute classe hors `local_read`).
    #[must_use]
    pub const fn requires_guard_evaluation(&self) -> bool {
        !matches!(self, Self::LocalRead)
    }
}

impl fmt::Display for EffectsClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EffectsClass {
    type Err = ActionBrokerError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_wire_str(s)
    }
}

/// Nature discriminée d'un sujet d'action (`ai-guard.md` R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    /// Action initiée par une invocation d'agent.
    Invocation,
    /// Action initiée par un worker mécanique de replay sans modèle.
    Replay,
    /// Action administrative ou système du plan de contrôle.
    ControlPlane,
}

/// Référence versionnée d'un rôle logique.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleVersionRef {
    /// Identifiant du rôle.
    pub role_id: String,
    /// Numéro de version exact.
    pub version: u64,
}

/// Référence versionnée d'un skill.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillVersionRef {
    /// Identifiant du skill.
    pub skill_id: String,
    /// Numéro de version exact.
    pub version: u64,
}

/// Identifiant typé d'un sujet ciblé par une suspension ou révocation (`F017`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum TypedSubjectRef {
    /// Agent logique durable.
    Agent(AgentId),
    /// Invocation particulière.
    Invocation(InvocationId),
    /// Worker mécanique de replay.
    ReplayWorker(ReplayWorkerId),
    /// Principal authentifié (acteur OS/client).
    Principal(ActorId),
    /// Version exacte d'un rôle.
    RoleVersion(RoleVersionRef),
    /// Version exacte d'un skill.
    SkillVersion(SkillVersionRef),
}

impl TypedSubjectRef {
    /// Retourne la clé canonique stable utilisée pour le stockage et l'invalidation de cache.
    #[must_use]
    pub fn canonical_key(&self) -> String {
        match self {
            Self::Agent(id) => format!("agent:{id}"),
            Self::Invocation(id) => format!("invocation:{id}"),
            Self::ReplayWorker(id) => format!("replay_worker:{id}"),
            Self::Principal(id) => format!("principal:{id}"),
            Self::RoleVersion(r) => format!("role:{}@{}", r.role_id, r.version),
            Self::SkillVersion(s) => format!("skill:{}@{}", s.skill_id, s.version),
        }
    }
}

impl fmt::Display for TypedSubjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical_key())
    }
}

/// Sujet authentifié d'une [`ActionIntent`], interdisant toute identité agente factice sur
/// `Replay` et `ControlPlane` (`ai-guard.md` R6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSubject {
    kind: SubjectKind,
    principal_id: ActorId,
    cause_ref: String,
    invocation_id: Option<InvocationId>,
    attempt_id: Option<AttemptId>,
    replay_worker_id: Option<ReplayWorkerId>,
    agent_id: Option<AgentId>,
    mission_id: Option<MissionId>,
    role_version: Option<RoleVersionRef>,
}

impl<'de> Deserialize<'de> for ActionSubject {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct SubjectWire {
            kind: SubjectKind,
            principal_id: ActorId,
            cause_ref: String,
            invocation_id: Option<InvocationId>,
            attempt_id: Option<AttemptId>,
            replay_worker_id: Option<ReplayWorkerId>,
            agent_id: Option<AgentId>,
            mission_id: Option<MissionId>,
            role_version: Option<RoleVersionRef>,
        }
        let wire = SubjectWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.kind,
            wire.principal_id,
            wire.cause_ref,
            wire.invocation_id,
            wire.attempt_id,
            wire.replay_worker_id,
            wire.agent_id,
            wire.mission_id,
            wire.role_version,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ActionSubject {
    /// Construit un sujet d'invocation agente valide.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::InvalidArgument`] si `cause_ref` est vide.
    pub fn new_invocation(
        principal_id: ActorId,
        cause_ref: String,
        invocation_id: InvocationId,
        attempt_id: AttemptId,
        agent_id: Option<AgentId>,
        mission_id: Option<MissionId>,
        role_version: Option<RoleVersionRef>,
    ) -> Result<Self, ActionBrokerError> {
        Self::from_parts(
            SubjectKind::Invocation,
            principal_id,
            cause_ref,
            Some(invocation_id),
            Some(attempt_id),
            None,
            agent_id,
            mission_id,
            role_version,
        )
    }

    /// Construit un sujet de replay mécanique sans modèle ni `agent_id`.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::InvalidArgument`] si `cause_ref` est vide.
    pub fn new_replay(
        principal_id: ActorId,
        cause_ref: String,
        replay_worker_id: ReplayWorkerId,
    ) -> Result<Self, ActionBrokerError> {
        Self::from_parts(
            SubjectKind::Replay,
            principal_id,
            cause_ref,
            None,
            None,
            Some(replay_worker_id),
            None,
            None,
            None,
        )
    }

    /// Construit un sujet du plan de contrôle ou sonde sans `agent_id`.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::InvalidArgument`] si `cause_ref` est vide.
    pub fn new_control_plane(
        principal_id: ActorId,
        cause_ref: String,
    ) -> Result<Self, ActionBrokerError> {
        Self::from_parts(
            SubjectKind::ControlPlane,
            principal_id,
            cause_ref,
            None,
            None,
            None,
            None,
            None,
            None,
        )
    }

    /// Construit et valide un sujet à partir de ses champs discriminés.
    ///
    /// Refuse toute identité agente factice sur un sujet `Replay` ou `ControlPlane`.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::InvalidArgument`] si les champs ne respectent pas le discriminant.
    #[expect(
        clippy::too_many_arguments,
        reason = "Constructeur validant toutes les variantes discriminées d'ActionSubject"
    )]
    pub fn from_parts(
        kind: SubjectKind,
        principal_id: ActorId,
        cause_ref: String,
        invocation_id: Option<InvocationId>,
        attempt_id: Option<AttemptId>,
        replay_worker_id: Option<ReplayWorkerId>,
        agent_id: Option<AgentId>,
        mission_id: Option<MissionId>,
        role_version: Option<RoleVersionRef>,
    ) -> Result<Self, ActionBrokerError> {
        if cause_ref.trim().is_empty() {
            return Err(ActionBrokerError::InvalidArgument {
                reason: String::from("cause_ref du sujet d'action ne peut pas être vide"),
            });
        }
        match kind {
            SubjectKind::Invocation => {
                if invocation_id.is_none() || attempt_id.is_none() {
                    return Err(ActionBrokerError::InvalidArgument {
                        reason: String::from(
                            "un sujet Invocation exige invocation_id et attempt_id",
                        ),
                    });
                }
                if replay_worker_id.is_some() {
                    return Err(ActionBrokerError::InvalidArgument {
                        reason: String::from(
                            "un sujet Invocation ne peut pas porter replay_worker_id",
                        ),
                    });
                }
            }
            SubjectKind::Replay => {
                if replay_worker_id.is_none() {
                    return Err(ActionBrokerError::InvalidArgument {
                        reason: String::from("un sujet Replay exige replay_worker_id"),
                    });
                }
                if invocation_id.is_some() || attempt_id.is_some() || agent_id.is_some() {
                    return Err(ActionBrokerError::InvalidArgument {
                        reason: String::from(
                            "un sujet Replay ne doit pas fabriquer d'agent_id ni d'invocation_id",
                        ),
                    });
                }
            }
            SubjectKind::ControlPlane => {
                if invocation_id.is_some()
                    || attempt_id.is_some()
                    || replay_worker_id.is_some()
                    || agent_id.is_some()
                {
                    return Err(ActionBrokerError::InvalidArgument {
                        reason: String::from(
                            "un sujet ControlPlane ne peut pas porter d'agent_id ou d'identifiant worker/invocation",
                        ),
                    });
                }
            }
        }

        Ok(Self {
            kind,
            principal_id,
            cause_ref,
            invocation_id,
            attempt_id,
            replay_worker_id,
            agent_id,
            mission_id,
            role_version,
        })
    }

    /// Nature discriminée du sujet.
    #[must_use]
    pub const fn kind(&self) -> SubjectKind {
        self.kind
    }

    /// Principal authentifié responsable.
    #[must_use]
    pub const fn principal_id(&self) -> &ActorId {
        &self.principal_id
    }

    /// Référence causale authentifiée.
    #[must_use]
    pub fn cause_ref(&self) -> &str {
        &self.cause_ref
    }

    /// Invocation associée si `kind == SubjectKind::Invocation`.
    #[must_use]
    pub const fn invocation_id(&self) -> Option<InvocationId> {
        self.invocation_id
    }

    /// Tentative associée si `kind == SubjectKind::Invocation`.
    #[must_use]
    pub const fn attempt_id(&self) -> Option<AttemptId> {
        self.attempt_id
    }

    /// Worker de replay si `kind == SubjectKind::Replay`.
    #[must_use]
    pub const fn replay_worker_id(&self) -> Option<ReplayWorkerId> {
        self.replay_worker_id
    }

    /// Identifiant d'agent si applicable.
    #[must_use]
    pub const fn agent_id(&self) -> Option<AgentId> {
        self.agent_id
    }

    /// Identifiant de mission optionnel.
    #[must_use]
    pub const fn mission_id(&self) -> Option<MissionId> {
        self.mission_id
    }

    /// Référence de version de rôle si applicable.
    #[must_use]
    pub const fn role_version(&self) -> Option<&RoleVersionRef> {
        self.role_version.as_ref()
    }

    /// Clé d'identité canonique principale utilisée pour le cache et la traçabilité.
    #[must_use]
    pub fn primary_identity_key(&self) -> String {
        if let Some(agent_id) = self.agent_id {
            format!("agent:{agent_id}")
        } else if let Some(rw) = self.replay_worker_id {
            format!("replay_worker:{rw}")
        } else if let Some(inv) = self.invocation_id {
            format!("invocation:{inv}")
        } else {
            format!("principal:{}", self.principal_id)
        }
    }

    /// Retourne toutes les références de sujets correspondantes à contrôler lors d'une suspension ou révocation.
    #[must_use]
    pub fn matching_subject_refs(&self) -> Vec<TypedSubjectRef> {
        let mut refs = vec![TypedSubjectRef::Principal(self.principal_id.clone())];
        if let Some(agent_id) = self.agent_id {
            refs.push(TypedSubjectRef::Agent(agent_id));
        }
        if let Some(invocation_id) = self.invocation_id {
            refs.push(TypedSubjectRef::Invocation(invocation_id));
        }
        if let Some(replay_worker_id) = self.replay_worker_id {
            refs.push(TypedSubjectRef::ReplayWorker(replay_worker_id));
        }
        if let Some(role) = &self.role_version {
            refs.push(TypedSubjectRef::RoleVersion(role.clone()));
        }
        refs
    }
}

/// Ressource normalisée ciblée par une action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum NormalizedResourceRef {
    /// Chemin filesystem (vérifié contre traversal/symlinks/TOCTOU).
    Path(PathBuf),
    /// Référence Git dans un dépôt local autorisé.
    GitRef {
        /// Racine du dépôt Git.
        repo: PathBuf,
        /// Nom complet de référence (`refs/heads/...`).
        ref_name: String,
    },
    /// Destination réseau (hôte et port).
    NetworkEndpoint {
        /// Nom d'hôte ou adresse normalisée.
        host: String,
        /// Port TCP optionnel.
        port: Option<u16>,
        /// Indique si la destination est loopback ou réseau privé explicite.
        is_private_or_loopback: bool,
    },
    /// Ressource d'un connecteur documentaire.
    ConnectorResource {
        /// Nom du connecteur (`gdrive`, `onedrive`, `notion`).
        connector: String,
        /// Identifiant de ressource distant.
        resource_id: String,
    },
    /// Session PTY de terminal supervisée.
    Terminal(TerminalId),
    /// Cible d'inférence modèle avec localité déclarée.
    ModelTarget {
        /// Identifiant du fournisseur configuré.
        provider: String,
        /// Identifiant exact du modèle.
        model_id: String,
        /// Localité déclarée du profil cible.
        locality: DataLocality,
    },
}

impl NormalizedResourceRef {
    /// Représentation textuelle sans secret destinée aux requêtes `GuardPort` et aux journaux.
    #[must_use]
    pub fn as_sanitized_label(&self) -> String {
        match self {
            Self::Path(p) => format!("path:{}", p.display()),
            Self::GitRef { repo, ref_name } => format!("git:{}:{ref_name}", repo.display()),
            Self::NetworkEndpoint { host, port, .. } => match port {
                Some(p) => format!("net:{host}:{p}"),
                None => format!("net:{host}"),
            },
            Self::ConnectorResource {
                connector,
                resource_id,
            } => format!("connector:{connector}:{resource_id}"),
            Self::Terminal(tid) => format!("terminal:{tid}"),
            Self::ModelTarget {
                provider,
                model_id,
                locality,
            } => format!("model:{provider}:{model_id}:{locality}"),
        }
    }
}

/// Niveau de confiance de la provenance d'une proposition d'action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Produit directement par un composant interne de confiance du moteur Unleash.
    TrustedEngine,
    /// Issu d'une source externe non fiable (document, DOM, MCP, stdout, skill importé, proposition modèle).
    UntrustedExternal,
}

/// Catégorie d'origine externe non fiable (`REQ-SEC-006`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSourceKind {
    /// Contenu d'un fichier README ou document de dépôt.
    ReadmeDocument,
    /// Sortie ou annotation d'un outil MCP.
    McpToolOutput,
    /// Sortie standard/erreur d'un test ou build.
    TestOutput,
    /// Skill ou rôle importé de source externe.
    ImportedSkill,
    /// Page web ou DOM.
    DomContent,
    /// Proposition libre d'un modèle LLM.
    ModelProposal,
}

/// Provenance tracée d'une action soumise au broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum ActionProvenance {
    /// Action interne construite par le harness Unleash.
    EngineInternal,
    /// Proposition dérivée d'une source externe identifiée.
    ExternalSource {
        /// Catégorie de source externe.
        source_kind: ExternalSourceKind,
        /// Étiquette de provenance sans secret.
        source_label: String,
        /// Niveau de confiance (jamais promu en autorité de contrôle).
        trust_level: TrustLevel,
    },
}

/// Intention durable et validée soumise à [`crate::ActionBroker`] (`F017/interfaces.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionIntent {
    /// Identifiant unique durable de l'action.
    pub action_id: ActionId,
    /// Sujet typé et authentifié.
    pub subject: ActionSubject,
    /// Projet cible si applicable.
    pub project_id: Option<ProjectId>,
    /// Verbe typé de l'action (ex: `"fs.write"`, `"git.commit"`).
    pub verb: String,
    /// Ressources normalisées ciblées.
    pub resource_refs: Vec<NormalizedResourceRef>,
    /// Empreinte canonique RFC 8785 + SHA-256 (`actions.payload_hash`).
    pub payload_digest: Sha256Digest,
    /// Stratégie de tolérance aux pannes et réconciliation.
    pub reconcile_strategy: ReconcileStrategy,
    /// Classe d'effet fermée (11 variantes).
    pub effects_class: EffectsClass,
    /// Révision courante du scope/mandat.
    pub scope_revision: Revision,
    /// Révision courante de la politique de sécurité.
    pub policy_revision: Revision,
    /// Barrière de lignée/époque/fence en vigueur.
    pub lineage_fence: LineageFence,
    /// Date limite UTC de l'action (distincte de l'`expiry` d'un `Allow`).
    pub action_deadline: TimestampMs,
    /// Exigences d'isolation dérivées de l'autorité effective.
    pub isolation_requirements: IsolationRequirements,
    /// Références opaques vers le coffre OS (jamais les secrets en clair).
    pub secret_refs: Vec<SecretRef>,
    /// Provenance tracée de la proposition d'action.
    pub provenance: ActionProvenance,
}

impl ActionIntent {
    /// Calcule l'empreinte canonique RFC 8785 (JCS) + SHA-256 d'un payload JSON d'action.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::InvalidArgument`] si la canonisation JCS échoue.
    pub fn compute_payload_digest(
        payload: &serde_json::Value,
    ) -> Result<Sha256Digest, ActionBrokerError> {
        canonical_sha256(payload).map_err(ActionBrokerError::from)
    }
}

/// Intention brute dont `effects_class` n'a pas encore été validée contre la liste fermée.
///
/// Permet au broker de refuser toute classe inconnue avec `InvalidArgument/EFFECTS_CLASS_UNKNOWN`
/// avant tout appel à [`crate::GuardPort`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnvalidatedActionIntent {
    /// Identifiant unique durable de l'action.
    pub action_id: ActionId,
    /// Sujet typé et authentifié.
    pub subject: ActionSubject,
    /// Projet cible si applicable.
    pub project_id: Option<ProjectId>,
    /// Verbe typé de l'action.
    pub verb: String,
    /// Ressources normalisées ciblées.
    pub resource_refs: Vec<NormalizedResourceRef>,
    /// Empreinte canonique du payload.
    pub payload_digest: Sha256Digest,
    /// Stratégie de réconciliation.
    pub reconcile_strategy: ReconcileStrategy,
    /// Chaîne brute de classe d'effet (doit appartenir aux 11 variantes fermées).
    pub effects_class: String,
    /// Révision courante du scope.
    pub scope_revision: Revision,
    /// Révision courante de la politique.
    pub policy_revision: Revision,
    /// Fence courant.
    pub lineage_fence: LineageFence,
    /// Date limite UTC de l'action.
    pub action_deadline: TimestampMs,
    /// Exigences d'isolation.
    pub isolation_requirements: IsolationRequirements,
    /// Références opaques de secrets.
    pub secret_refs: Vec<SecretRef>,
    /// Provenance tracée.
    pub provenance: ActionProvenance,
}

impl UnvalidatedActionIntent {
    /// Valide la classe d'effet fermée et convertit en [`ActionIntent`] typée.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::EffectsClassUnknown`] si `effects_class` est inconnu.
    pub fn validate(self) -> Result<ActionIntent, ActionBrokerError> {
        let effects_class = EffectsClass::from_wire_str(&self.effects_class)?;
        Ok(ActionIntent {
            action_id: self.action_id,
            subject: self.subject,
            project_id: self.project_id,
            verb: self.verb,
            resource_refs: self.resource_refs,
            payload_digest: self.payload_digest,
            reconcile_strategy: self.reconcile_strategy,
            effects_class,
            scope_revision: self.scope_revision,
            policy_revision: self.policy_revision,
            lineage_fence: self.lineage_fence,
            action_deadline: self.action_deadline,
            isolation_requirements: self.isolation_requirements,
            secret_refs: self.secret_refs,
            provenance: self.provenance,
        })
    }
}
