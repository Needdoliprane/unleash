//! DTOs stricts (`#[serde(deny_unknown_fields)]`) alignés sur les 5 JSON Schemas V1
//! des contrats observables et reçus de découverte.

use core::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use unleash_domain::{Sha256Digest, StreamId, TimestampMs};
use unleash_protocol::canonical_sha256;

use crate::errors::SpikeError;

/// Convertit un [`TimestampMs`] du domaine en `DateTime<Utc>`.
///
/// # Errors
/// Renvoie [`SpikeError::InvalidTimestamp`] si la valeur sort des bornes `DateTime<Utc>`.
pub fn timestamp_ms_to_utc(ts: TimestampMs) -> Result<DateTime<Utc>, SpikeError> {
    DateTime::from_timestamp_millis(ts.as_i64())
        .ok_or_else(|| SpikeError::InvalidTimestamp(ts.as_i64()))
}

/// Empreinte SHA-256 sérialisée en 64 caractères hexadécimaux minuscules (`^[a-f0-9]{64}$`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HexSha256(pub Sha256Digest);

impl HexSha256 {
    /// Calcule l'empreinte SHA-256 brute d'une tranche d'octets.
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(Sha256Digest::from_bytes(bytes))
    }

    /// Calcule l'empreinte SHA-256 sur la canonisation RFC 8785 (JCS) d'une valeur sérialisable.
    ///
    /// # Errors
    /// Renvoie [`SpikeError::Protocol`] si la canonisation JCS échoue.
    pub fn canonical_json<T: Serialize>(value: &T) -> Result<Self, SpikeError> {
        Ok(Self(canonical_sha256(value)?))
    }

    /// Renvoie les 64 caractères hexadécimaux minuscules.
    #[must_use]
    pub fn as_hex(&self) -> String {
        self.0.hex()
    }
}

impl fmt::Display for HexSha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.hex())
    }
}

impl From<Sha256Digest> for HexSha256 {
    fn from(digest: Sha256Digest) -> Self {
        Self(digest)
    }
}

impl FromStr for HexSha256 {
    type Err = unleash_domain::IdValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Sha256Digest::parse_hex(s).map(Self)
    }
}

impl Serialize for HexSha256 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.hex())
    }
}

impl<'de> Deserialize<'de> for HexSha256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Sha256Digest::parse_hex(&raw)
            .map(Self)
            .map_err(de::Error::custom)
    }
}

/// Empreinte structurée conforme à `$defs/contentDigest` de `discovery-receipt.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentDigest {
    /// Algorithme utilisé (`"sha256"`).
    pub algorithm: String,
    /// Hexadécimal minuscule (`^[0-9a-f]+$`).
    pub hex: String,
}

impl ContentDigest {
    /// Construit un [`ContentDigest`] SHA-256 à partir d'un [`HexSha256`].
    #[must_use]
    pub fn sha256(digest: HexSha256) -> Self {
        Self {
            algorithm: String::from("sha256"),
            hex: digest.as_hex(),
        }
    }
}

/// Curseur d'événement (`stream_id` UUID + `sequence`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventCursorRef {
    /// Flux durable d'événements.
    pub stream_id: StreamId,
    /// Numéro de séquence dans le flux.
    pub sequence: u64,
}

/// Niveau de cohérence d'une capture de source (`shared-knowledge.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureConsistency {
    /// Lu depuis un objet tree Git déterminé sans transformation implicite.
    GitTreeCoherent,
    /// Capture atomique système de fichiers qualifiée.
    AtomicFsCoherent,
    /// Version immuable de registre ou révision documentaire qualifiée.
    ImmutableVersionCoherent,
    /// Capture séquentielle sans preuve d'instant commun.
    Unverified,
    /// Mutation ou mélange détecté pendant la capture.
    Inconsistent,
}

/// Origine dérivée côté serveur d'un [`DiscoveryReceipt`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryOrigin {
    /// Découverte effectuée dans une invocation d'agent.
    Invocation {
        /// Principal effectif.
        principal_id: String,
        /// Cause durable.
        cause: DiscoveryCause,
        /// Identifiant d'invocation.
        invocation_id: String,
        /// Identifiant de tentative.
        attempt_id: String,
        /// Identifiant de run si rattaché à un workflow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        /// Identifiant d'étape si rattaché à un workflow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step_id: Option<String>,
    },
    /// Découverte effectuée dans un worker de replay sans modèle.
    Replay {
        /// Principal effectif.
        principal_id: String,
        /// Cause durable.
        cause: DiscoveryCause,
        /// Identifiant du worker de replay.
        replay_worker_id: String,
        /// Identifiant du run.
        run_id: String,
        /// Identifiant de l'étape.
        step_id: String,
    },
    /// Découverte initiée depuis le plan de contrôle (CLI/TUI/GUI).
    ControlPlane {
        /// Principal effectif.
        principal_id: String,
        /// Cause durable.
        cause: DiscoveryCause,
        /// Principal du plan de contrôle.
        control_principal_id: String,
        /// Run source optionnel.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_run_id: Option<String>,
    },
}

/// Cause durable rattachée à l'origine d'un reçu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryCause {
    /// Nature de la cause (`command_receipt`, `action`, `timer`, `event`).
    pub kind: DiscoveryCauseKind,
    /// Identifiant de la cause.
    pub id: String,
}

/// Variantes autorisées pour `cause.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryCauseKind {
    /// Reçu de commande.
    CommandReceipt,
    /// Action du broker.
    Action,
    /// Minuterie.
    Timer,
    /// Événement du flux.
    Event,
}

/// Type de registre pour `registry_version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryKind {
    /// Skill.
    Skill,
    /// Rôle.
    Role,
    /// Contrat.
    Contract,
    /// Automatisation.
    Automation,
    /// Ressource technique.
    TechnicalResource,
}

/// Portée de namespace d'une ressource de registre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryNamespace {
    /// Portée globale.
    Global,
    /// Portée projet.
    Project,
    /// Portée dépôt.
    Repository,
}

/// Référence typée de source conforme à `$defs/typedSourceRef`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypedSourceRef {
    /// Source Git (`repo_id` + `worktree_id` nullable).
    GitSnapshot {
        /// Identifiant logique de la source.
        source_id: String,
        /// Identifiant exact de version/snapshot.
        version_id: String,
        /// Digest canonique du contenu ou manifeste.
        content_digest: ContentDigest,
        /// Génération monotone locale.
        generation: u64,
        /// Référence de politique d'accès.
        access_policy_ref: String,
        /// Révision d'ACL.
        acl_revision: u64,
        /// Horodatage d'observation de cette révision.
        observed_at: DateTime<Utc>,
        /// Cohérence de capture.
        capture_consistency: CaptureConsistency,
        /// Identifiant du dépôt Git.
        repo_id: String,
        /// Identifiant du worktree (null si dépôt nu/tree pur).
        worktree_id: Option<String>,
    },
    /// Source documentaire / connecteur F013.
    ResourceSnapshot {
        /// Identifiant logique de la source.
        source_id: String,
        /// Identifiant exact de version/snapshot.
        version_id: String,
        /// Digest canonique.
        content_digest: ContentDigest,
        /// Génération monotone.
        generation: u64,
        /// Référence de politique d'accès.
        access_policy_ref: String,
        /// Révision d'ACL.
        acl_revision: u64,
        /// Horodatage d'observation.
        observed_at: DateTime<Utc>,
        /// Cohérence de capture.
        capture_consistency: CaptureConsistency,
        /// Liaison de source.
        source_binding_id: String,
        /// Identifiant de la ressource distante.
        resource_ref_id: String,
    },
    /// Source fichiers locaux sans Git.
    FileSnapshot {
        /// Identifiant logique de la source.
        source_id: String,
        /// Identifiant exact de version/snapshot.
        version_id: String,
        /// Digest canonique.
        content_digest: ContentDigest,
        /// Génération monotone.
        generation: u64,
        /// Référence de politique d'accès.
        access_policy_ref: String,
        /// Révision d'ACL.
        acl_revision: u64,
        /// Horodatage d'observation.
        observed_at: DateTime<Utc>,
        /// Cohérence de capture.
        capture_consistency: CaptureConsistency,
        /// Liaison de source.
        source_binding_id: String,
        /// Racine autorisée.
        root_ref: String,
    },
    /// Version immuable de registre.
    RegistryVersion {
        /// Identifiant logique de la source.
        source_id: String,
        /// Identifiant exact de version.
        version_id: String,
        /// Digest canonique.
        content_digest: ContentDigest,
        /// Génération monotone.
        generation: u64,
        /// Référence de politique d'accès.
        access_policy_ref: String,
        /// Révision d'ACL.
        acl_revision: u64,
        /// Horodatage d'observation.
        observed_at: DateTime<Utc>,
        /// Cohérence de capture (`immutable_version_coherent` requis).
        capture_consistency: CaptureConsistency,
        /// Nature d'entrée de registre.
        registry_kind: RegistryKind,
        /// Identifiant de ressource.
        resource_id: String,
        /// Portée du namespace.
        namespace: RegistryNamespace,
        /// Clé propriétaire du namespace.
        namespace_ref: String,
    },
}

impl TypedSourceRef {
    /// Renvoie le niveau de cohérence de capture de la référence.
    #[must_use]
    pub const fn capture_consistency(&self) -> CaptureConsistency {
        match self {
            Self::GitSnapshot {
                capture_consistency,
                ..
            }
            | Self::ResourceSnapshot {
                capture_consistency,
                ..
            }
            | Self::FileSnapshot {
                capture_consistency,
                ..
            }
            | Self::RegistryVersion {
                capture_consistency,
                ..
            } => *capture_consistency,
        }
    }

    /// Renvoie `version_id` de la source.
    #[must_use]
    pub fn version_id(&self) -> &str {
        match self {
            Self::GitSnapshot { version_id, .. }
            | Self::ResourceSnapshot { version_id, .. }
            | Self::FileSnapshot { version_id, .. }
            | Self::RegistryVersion { version_id, .. } => version_id,
        }
    }

    /// Renvoie le digest hexadécimal de contenu de la source.
    #[must_use]
    pub fn content_hex(&self) -> &str {
        match self {
            Self::GitSnapshot { content_digest, .. }
            | Self::ResourceSnapshot { content_digest, .. }
            | Self::FileSnapshot { content_digest, .. }
            | Self::RegistryVersion { content_digest, .. } => &content_digest.hex,
        }
    }
}

/// Finalité d'un reçu de découverte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryPurpose {
    /// Avant création structurée.
    PreCreation,
    /// Revalidation à la clôture.
    CompletionRevalidation,
    /// Réconciliation a posteriori dérogatoire.
    PosthocReconciliation,
}

/// Décision prise à l'issue de la découverte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryDecision {
    /// Réutiliser une ressource existante.
    Reuse,
    /// Créer une nouvelle ressource.
    Create,
}

/// Stratégie associée à `reuse` ou `create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryStrategy {
    /// Utiliser la ressource telle quelle.
    UseExisting,
    /// Coordonner avec le propriétaire existant.
    Coordinate,
    /// Attendre une ressource en développement.
    Wait,
    /// Copier comme point de départ d'une ressource distincte (`create` uniquement).
    Copy,
    /// Adapter (sur même ressource pour `reuse`, nouvelle ressource pour `create`).
    Adapt,
    /// Créer sans candidat préalable (`create` uniquement).
    New,
}

/// Éligibilité observée à `validation_basis.checked_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Valide au contrôle.
    Valid,
    /// Périmée.
    Stale,
    /// Inconnue.
    Unknown,
}

/// Relation temporelle entre la découverte et la création.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalRelation {
    /// Avant création.
    BeforeCreation,
    /// Observée après création.
    ObservedAfterCreation,
}

/// Base de validation temporelle scellée dans un [`DiscoveryReceipt`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationBasis {
    /// Révision de l'intention.
    pub intent_revision: u64,
    /// Référence du scope.
    pub scope_ref: String,
    /// Révision du scope.
    pub scope_revision: u64,
    /// Référence versionnée de politique.
    pub policy_version_ref: String,
    /// Version du validateur.
    pub validator_version: String,
    /// Curseur d'événements du flux.
    pub event_cursor: EventCursorRef,
    /// Instant de vérification.
    pub checked_at: DateTime<Utc>,
    /// Borne maximale de validité (`<= checked_at + max_age_ms`).
    pub valid_until: DateTime<Utc>,
    /// Durée maximale en millisecondes (> 0).
    pub max_age_ms: u64,
}

/// Reçu durable de découverte conforme à `discovery-receipt.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryReceipt {
    /// Version du schéma (`"unleash.discovery.v1"`).
    pub schema_version: String,
    /// Identifiant unique du reçu.
    pub receipt_id: String,
    /// Identifiant de l'intention.
    pub intent_id: String,
    /// Origine dérivée côté serveur.
    pub origin: DiscoveryOrigin,
    /// Identifiants des requêtes exécutées.
    pub query_ids: Vec<String>,
    /// Révision du catalogue consulté.
    pub catalog_revision: u64,
    /// Finalité (`pre_creation`, `completion_revalidation`, `posthoc_reconciliation`).
    pub purpose: DiscoveryPurpose,
    /// Décision (`reuse` ou `create`).
    pub decision: DiscoveryDecision,
    /// Stratégie retenue.
    pub strategy: DiscoveryStrategy,
    /// Identifiants des candidats examinés.
    pub candidate_ids: Vec<String>,
    /// Cible existante (`reuse` ou post-création), `null` en `create/pre_creation`.
    pub target_resource_id: Option<String>,
    /// Version cible (`reuse` ou post-création), `null` en `create/pre_creation`.
    pub target_version_ref: Option<String>,
    /// Justification explicite.
    pub justification: String,
    /// Sources consultées avec provenance et cohérence de capture.
    pub source_refs: Vec<TypedSourceRef>,
    /// Fraîcheur observée à `checked_at`.
    pub freshness: Freshness,
    /// Relation temporelle à la création.
    pub temporal_relation: TemporalRelation,
    /// Base de validation temporelle.
    pub validation_basis: ValidationBasis,
    /// Horodatage d'enregistrement durable.
    pub recorded_at: DateTime<Utc>,
    /// Identifiant du reçu remplacé (non nul en `completion_revalidation`).
    pub supersedes_receipt_id: Option<String>,
    /// Identifiant d'usage de création (non nul en `completion_revalidation`).
    pub creation_use_id: Option<String>,
    /// Référence vers l'enregistrement durable de couverture complète.
    pub coverage_ref: String,
}

/// Sorte de snapshot d'un sujet (`expected-test-execution.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotKind {
    /// `git_snapshot` avec `capture_consistency = git_tree_coherent`.
    GitTree,
    /// Toute autre entrée de `source_snapshots` (dirty Git, fichier, ressource).
    SourceSnapshot,
    /// Donnée d'automatisation (`AutomationDataRevision`).
    DataSnapshot,
}

/// Référence de snapshot d'un sujet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRef {
    /// Variante de snapshot.
    pub kind: SnapshotKind,
    /// `version_id` résolu (jamais un OID Git brut comme substitut).
    pub id: String,
    /// Digest SHA-256 du manifeste canonique de cette version.
    pub sha256: HexSha256,
}

/// Référence exacte de sujet alignée sur `$defs/subject`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectRef {
    /// Identifiant du sujet.
    pub subject_id: String,
    /// Révision du sujet.
    pub subject_revision: u64,
    /// Identifiant du scope.
    pub scope_id: String,
    /// Révision du scope.
    pub scope_revision: u64,
    /// Snapshot exact résolu.
    pub snapshot_ref: SnapshotRef,
}

/// Référence exacte adressée par contenu (`$defs/exact_ref`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactRef {
    /// Identifiant logique.
    pub id: String,
    /// Numéro de version (>= 1).
    pub version: u64,
    /// Empreinte SHA-256 canonique.
    pub sha256: HexSha256,
}

/// Nature de l'autorité d'un [`ExpectedTestExecution`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityKind {
    /// Contrat publié.
    Contract,
    /// Plan d'exécution accepté.
    ExecutionPlan,
    /// Mandat de mission.
    MissionMandate,
}

/// Référence vers l'autorité d'un attendu.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRef {
    /// Sorte d'autorité.
    pub kind: AuthorityKind,
    /// Identifiant.
    pub id: String,
    /// Version (>= 1).
    pub version: u64,
    /// Empreinte SHA-256.
    pub sha256: HexSha256,
}

/// Référence simple `(id, version)` d'un outil, parseur ou producteur.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolOrParserRef {
    /// Identifiant slug (`^[a-z0-9]+(?:[._-][a-z0-9]+)*$`).
    pub id: String,
    /// Version entière (>= 1).
    pub version: u64,
}

/// Spécification de commande comparée entre attendu et observation (`$defs/command`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    /// Outil d'exécution (`tool.process.run`, etc.).
    pub tool_ref: ToolOrParserRef,
    /// Empreinte SHA-256 de `argv` canonique.
    pub argv_sha256: HexSha256,
    /// Identité canonique du répertoire courant.
    pub cwd_resource_id: String,
    /// Profil d'environnement versionné.
    pub environment_profile_ref: ExactRef,
    /// Empreinte SHA-256 de l'environnement pertinent.
    pub environment_sha256: HexSha256,
    /// Suite de tests attendue.
    pub suite_ref: ExactRef,
    /// Parseur de résultats versionné.
    pub parser_ref: ToolOrParserRef,
}

/// Politique d'évaluation du résultat d'une exécution de test attendue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultPolicy {
    /// Code de sortie attendu.
    pub expected_exit_code: i32,
    /// Nombre minimal de cas exécutés requis (>= 1).
    pub min_cases: u32,
    /// Exige `passed == total` et `failed == skipped == 0`.
    pub require_all_passed: bool,
}

/// Objet immuable `ExpectedTestExecution V1` conforme à `expected-test-execution.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedTestExecution {
    /// `"unleash.expected-test-execution.v1"`.
    pub schema_version: String,
    /// Identifiant de l'attendu.
    pub expectation_id: String,
    /// Version (>= 1).
    pub version: u64,
    /// Source autoritaire (contrat, plan ou mandat).
    pub authority_ref: AuthorityRef,
    /// Sujet et snapshot exacts.
    pub subject: SubjectRef,
    /// Commande exacte attendue.
    pub command: CommandSpec,
    /// Politique de résultat.
    pub result_policy: ResultPolicy,
}

impl ExpectedTestExecution {
    /// Calcule l'[`ExactRef`] adressée par contenu (RFC 8785 + SHA-256) de cet attendu.
    ///
    /// # Errors
    /// Renvoie [`SpikeError::Protocol`] si la canonisation JCS échoue.
    pub fn exact_ref(&self) -> Result<ExactRef, SpikeError> {
        Ok(ExactRef {
            id: self.expectation_id.clone(),
            version: self.version,
            sha256: HexSha256::canonical_json(self)?,
        })
    }
}

/// Référence d'activation et révision courante (`activation_ref`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRef {
    /// Identifiant d'activation.
    pub id: String,
    /// Révision (>= 0).
    pub revision: u64,
}

/// Objet de scellement avant effet `ExpectedExecutionBinding V1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedExecutionBinding {
    /// `"unleash.expected-execution-binding.v1"`.
    pub schema_version: String,
    /// Identifiant unique du binding.
    pub binding_id: String,
    /// Contrat épinglé.
    pub contract_ref: ExactRef,
    /// Règles partageant ce slot.
    pub rule_ids: Vec<String>,
    /// Slot littéral déclaré.
    pub expectation_slot: String,
    /// Attendu lié par identité, version et digest.
    pub expectation_ref: ExactRef,
    /// Sujet et snapshot liés.
    pub subject: SubjectRef,
    /// Activation et révision.
    pub activation_ref: ActivationRef,
    /// Action réservée avant dispatch.
    pub action_id: String,
    /// Curseur scellé antérieur à l'observation (`sequence >= 1`).
    pub sealed_cursor: EventCursorRef,
}

/// État final du processus observé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    /// Processus terminé normalement avec code de sortie.
    Exited,
    /// Processus interrompu par signal ou timeout.
    Terminated,
    /// État final non observé.
    Unknown,
}

/// État du parseur de sortie de tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseState {
    /// Format structuré complet analysé avec succès.
    Complete,
    /// Sortie partielle, tronquée ou texte brut non structuré (`cargo test`).
    Incomplete,
    /// Parseur ou sortie indisponible.
    Unavailable,
}

/// Compteurs de cas de tests parsés (`total == passed + failed + skipped`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestCasesSummary {
    /// Nombre total de cas.
    pub total: u32,
    /// Cas réussis.
    pub passed: u32,
    /// Cas en échec.
    pub failed: u32,
    /// Cas ignorés.
    pub skipped: u32,
}

/// Résultat observé d'une exécution de test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResult {
    /// État de terminaison.
    pub process_state: ProcessState,
    /// Code de sortie (non nul si `Exited`, `null` sinon).
    pub exit_code: Option<i32>,
    /// État du parseur.
    pub parse_state: ParseState,
    /// Compteurs détaillés (non nul si `Complete`, `null` sinon).
    pub cases: Option<TestCasesSummary>,
}

/// Projection authentifiée `TestExecutionObservation V1` (`test-execution-observation.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestExecutionObservation {
    /// `"unleash.test-execution-observation.v1"`.
    pub schema_version: String,
    /// Identifiant de l'observation.
    pub observation_id: String,
    /// Adaptateur authentifié producteur.
    pub producer_ref: ToolOrParserRef,
    /// Action liée dans le journal.
    pub action_id: String,
    /// Binding scellé avant dispatch.
    pub execution_binding_id: String,
    /// Référence d'attendu observée.
    pub expectation_ref: ExactRef,
    /// Sujet observé.
    pub subject: SubjectRef,
    /// Commande effectivement collectée (jamais copiée depuis l'attendu).
    pub observed_command: CommandSpec,
    /// Résultat structuré.
    pub result: ExecutionResult,
}

/// Prédicat fermé V1 supporté par les claims observables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClosedPredicate {
    /// `process.exit_code` version 1.
    ProcessExitCodeV1,
    /// `test.expectation_satisfied` version 1.
    TestExpectationSatisfiedV1,
}

impl ClosedPredicate {
    /// Identifiant et version du prédicat.
    #[must_use]
    pub const fn as_id_version(self) -> (&'static str, u64) {
        match self {
            Self::ProcessExitCodeV1 => ("process.exit_code", 1),
            Self::TestExpectationSatisfiedV1 => ("test.expectation_satisfied", 1),
        }
    }
}

/// Proposition discriminée d'un [`ContractClaim`] conforme à `contract-claim.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimProposition {
    /// `process.exit_code` / 1 (`i32`).
    ObservableProcessExitCode {
        /// Attendu ciblé.
        expectation_ref: ExactRef,
        /// Binding ciblé.
        execution_binding_id: String,
        /// Valeur affirmée.
        asserted_value: i32,
    },
    /// `test.expectation_satisfied` / 1 (`bool`).
    ObservableTestExpectationSatisfied {
        /// Attendu ciblé.
        expectation_ref: ExactRef,
        /// Binding ciblé.
        execution_binding_id: String,
        /// Valeur booléenne affirmée.
        asserted_value: bool,
    },
    /// Claim purement qualitatif (jamais interprété comme prédicat).
    Qualitative,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PredicateRefWire {
    id: String,
    version: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPropositionWire {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    predicate_ref: Option<PredicateRefWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expectation_ref: Option<ExactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    execution_binding_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asserted_value: Option<serde_json::Value>,
}

impl Serialize for ClaimProposition {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match self {
            Self::ObservableProcessExitCode {
                expectation_ref,
                execution_binding_id,
                asserted_value,
            } => RawPropositionWire {
                kind: String::from("observable"),
                predicate_ref: Some(PredicateRefWire {
                    id: String::from("process.exit_code"),
                    version: 1,
                }),
                expectation_ref: Some(expectation_ref.clone()),
                execution_binding_id: Some(execution_binding_id.clone()),
                asserted_value: Some(serde_json::Value::from(*asserted_value)),
            },
            Self::ObservableTestExpectationSatisfied {
                expectation_ref,
                execution_binding_id,
                asserted_value,
            } => RawPropositionWire {
                kind: String::from("observable"),
                predicate_ref: Some(PredicateRefWire {
                    id: String::from("test.expectation_satisfied"),
                    version: 1,
                }),
                expectation_ref: Some(expectation_ref.clone()),
                execution_binding_id: Some(execution_binding_id.clone()),
                asserted_value: Some(serde_json::Value::from(*asserted_value)),
            },
            Self::Qualitative => RawPropositionWire {
                kind: String::from("qualitative"),
                predicate_ref: None,
                expectation_ref: None,
                execution_binding_id: None,
                asserted_value: None,
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ClaimProposition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawPropositionWire::deserialize(deserializer)?;
        match raw.kind.as_str() {
            "qualitative" => {
                if raw.predicate_ref.is_some()
                    || raw.expectation_ref.is_some()
                    || raw.execution_binding_id.is_some()
                    || raw.asserted_value.is_some()
                {
                    return Err(de::Error::custom(
                        "Un claim qualitatif ne doit contenir aucun champ de prédicat observable",
                    ));
                }
                Ok(Self::Qualitative)
            }
            "observable" => {
                let pred = raw
                    .predicate_ref
                    .ok_or_else(|| de::Error::missing_field("predicate_ref"))?;
                let expectation_ref = raw
                    .expectation_ref
                    .ok_or_else(|| de::Error::missing_field("expectation_ref"))?;
                let execution_binding_id = raw
                    .execution_binding_id
                    .ok_or_else(|| de::Error::missing_field("execution_binding_id"))?;
                let val = raw
                    .asserted_value
                    .ok_or_else(|| de::Error::missing_field("asserted_value"))?;

                match (pred.id.as_str(), pred.version) {
                    ("process.exit_code", 1) => {
                        let code_i64 = val.as_i64().ok_or_else(|| {
                            de::Error::custom("asserted_value doit être un entier i32")
                        })?;
                        let code = i32::try_from(code_i64)
                            .map_err(|_| de::Error::custom("asserted_value hors bornes i32"))?;
                        Ok(Self::ObservableProcessExitCode {
                            expectation_ref,
                            execution_binding_id,
                            asserted_value: code,
                        })
                    }
                    ("test.expectation_satisfied", 1) => {
                        let b = val.as_bool().ok_or_else(|| {
                            de::Error::custom("asserted_value doit être un booléen")
                        })?;
                        Ok(Self::ObservableTestExpectationSatisfied {
                            expectation_ref,
                            execution_binding_id,
                            asserted_value: b,
                        })
                    }
                    (id, version) => Err(de::Error::custom(format!(
                        "UnknownClaimPredicate: '{id}' v{version}"
                    ))),
                }
            }
            other => Err(de::Error::custom(format!(
                "kind de proposition inconnu : '{other}'"
            ))),
        }
    }
}

/// Objet `Claim V1` conforme à `contract-claim.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractClaim {
    /// `"unleash.contract-claim.v1"`.
    pub schema_version: String,
    /// Identifiant du claim.
    pub claim_id: String,
    /// Auteur authentifié côté serveur.
    pub author_ref: String,
    /// Sujet exact.
    pub subject: SubjectRef,
    /// Prose explicative (jamais interprétée comme prédicat).
    pub statement: String,
    /// Proposition fermée ou qualitative.
    pub proposition: ClaimProposition,
    /// Observations invoquées par l'auteur (peut omettre des preuves existantes).
    pub observation_ids: Vec<String>,
}
