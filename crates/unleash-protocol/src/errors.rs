//! # Modèle d'erreurs protocolaire et DTO d'erreur IPC
//!
//! Responsabilité exclusive : Définition des 17 classes communes d'erreur [`ErrorCode`],
//! des 5 classes de reprise [`RetryClass`], de l'énumération fermée des [`ReasonCode`],
//! du DTO wire [`IpcError`] (conforme à `ipc-error.schema.json`) et de l'erreur
//! d'implémentation de la crate [`ProtocolError`].

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use ts_rs::TS;
use unleash_domain::{CommandId, Revision};

/// Erreur interne survenue lors du traitement, décodage ou validation protocolaire.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProtocolError {
    /// Clé dupliquée détectée dans un objet JSON (interdit en décodage strict).
    #[error("Clé dupliquée '{key}' détectée")]
    DuplicateKey {
        /// Nom de la clé JSON dupliquée.
        key: String,
    },

    /// Version de protocole incompatible ou refusée (ex: `unleash.ipc.v1`).
    #[error("Version de protocole incompatible : '{version}' (version attendue : unleash.ipc.v2)")]
    ProtocolIncompatible {
        /// Version fournie dans la requête.
        version: String,
    },

    /// Dépassement du plafond de taille de trame (8 MiB = `8_388_608` octets).
    #[error("Trame trop volumineuse : {length} octets (plafond strict : {max} octets)")]
    FrameTooLarge {
        /// Longueur observée de la trame.
        length: u32,
        /// Plafond maximal autorisé.
        max: u32,
    },

    /// Fin de flux inattendue (EOF prématuré).
    #[error("Fin de flux inattendue lors de la lecture de la trame IPC")]
    UnexpectedEof,

    /// Erreur d'I/O sous-jacente synchrone.
    #[error("Erreur d'I/O protocolaire : {0}")]
    Io(#[from] std::io::Error),

    /// Erreur de syntaxe JSON ou de désérialisation Serde.
    #[error("Erreur de décodage JSON : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur survenue lors de la canonisation RFC 8785 (JCS).
    #[error("Erreur de canonisation JCS : {0}")]
    Jcs(String),

    /// Erreur de validation de contrat ou d'invariant.
    #[error("Erreur de validation protocolaire : {0}")]
    Validation(String),
}

/// Les dix-sept classes d'erreur communes canoniques (ADR-02 / `ipc-error.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Argument invalide ou schéma non respecté.
    InvalidArgument,
    /// Échec d'authentification du client ou du pair.
    AuthenticationFailed,
    /// Capacité requise absente, révoquée ou non attribuée.
    CapabilityDenied,
    /// Ressource ou projet hors du scope autorisé.
    ScopeDenied,
    /// Conflit de révision optimiste (`expected_revision` mismatch).
    RevisionConflict,
    /// Même `command_id` avec un payload/digest canonique différent.
    IdempotencyConflict,
    /// Ressource inexistante ou masquée pour préserver la sécurité.
    NotFoundOrHidden,
    /// Ressource momentanément occupée, verrouillée ou saturée (ex: backpressure).
    ResourceBusy,
    /// Dépassement de quota ou de budget d'exécution/modèle.
    BudgetExceeded,
    /// Limite de débit atteinte.
    RateLimited,
    /// Version de protocole incompatible (ex: tentative de soumission v1).
    ProtocolIncompatible,
    /// Barrière ou epoch obsolète (lignée serveur ou bail expiré).
    StaleFence,
    /// Confinement d'isolation ou de profil requis non disponible.
    IsolationUnavailable,
    /// Règle de contrat ou postcondition métier violée.
    ContractViolation,
    /// Issue d'un effet externe inconnue ; réconciliation obligatoire avant toute reprise.
    OutcomeUnknown,
    /// Panne ou indisponibilité interne non recouvrable automatiquement.
    InternalFailure,
    /// Décalage de flux nécessitant rechargement d'un snapshot et reprise de curseur.
    ResyncRequired,
}

impl ErrorCode {
    /// Renvoie le nom textuel wire canonique du code d'erreur.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidArgument => "InvalidArgument",
            Self::AuthenticationFailed => "AuthenticationFailed",
            Self::CapabilityDenied => "CapabilityDenied",
            Self::ScopeDenied => "ScopeDenied",
            Self::RevisionConflict => "RevisionConflict",
            Self::IdempotencyConflict => "IdempotencyConflict",
            Self::NotFoundOrHidden => "NotFoundOrHidden",
            Self::ResourceBusy => "ResourceBusy",
            Self::BudgetExceeded => "BudgetExceeded",
            Self::RateLimited => "RateLimited",
            Self::ProtocolIncompatible => "ProtocolIncompatible",
            Self::StaleFence => "StaleFence",
            Self::IsolationUnavailable => "IsolationUnavailable",
            Self::ContractViolation => "ContractViolation",
            Self::OutcomeUnknown => "OutcomeUnknown",
            Self::InternalFailure => "InternalFailure",
            Self::ResyncRequired => "ResyncRequired",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Les cinq classes de retry canoniques (`ipc-error.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RetryClass {
    /// Aucun retry permis : refus définitif.
    Never,
    /// Retry autorisé après délai exponentiel (backoff).
    AfterBackoff,
    /// Retry autorisé après rafraîchissement de l'état ou du snapshot.
    AfterRefresh,
    /// Retry subordonné à une phase explicite de réconciliation.
    AfterReconciliation,
    /// Retry subordonné à une décision humaine explicite.
    AfterHumanDecision,
}

impl RetryClass {
    /// Renvoie la chaîne textuelle normalisée de la classe de reprise.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::AfterBackoff => "after_backoff",
            Self::AfterRefresh => "after_refresh",
            Self::AfterReconciliation => "after_reconciliation",
            Self::AfterHumanDecision => "after_human_decision",
        }
    }
}

impl fmt::Display for RetryClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Catalogue fermé et typé des raisons d'erreur canoniques d'Unleash.
///
/// Invariants :
/// - Chaque raison appartient à exactement une classe [`ErrorCode`].
/// - Aucune raison ne porte le nom d'une classe commune.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[non_exhaustive]
pub enum ReasonCode {
    // --- RevisionConflict ---
    /// Conflit lors de la publication d'un artefact.
    #[serde(rename = "PUBLICATION_CONFLICT")]
    PublicationConflict,
    /// Conflit de version lors de l'application d'un contrat.
    #[serde(rename = "CONTRACT_VERSION_CONFLICT")]
    ContractVersionConflict,
    /// La cible Git a bougé depuis l'état attendu (`oldOID` divergent).
    #[serde(rename = "TARGET_MOVED")]
    TargetMoved,
    /// La référence ou l'objet source Git a changé depuis sa résolution.
    #[serde(rename = "SOURCE_CHANGED")]
    SourceChanged,
    /// La cible Git créée avec précondition `ref_absent` existe déjà.
    #[serde(rename = "TARGET_ALREADY_EXISTS")]
    TargetAlreadyExists,

    // --- StaleFence ---
    /// Époque d'exécution, de tentative ou de lease obsolète.
    #[serde(rename = "STALE_EPOCH")]
    StaleEpoch,
    /// Affectation de mission ou de goal réattribuée.
    #[serde(rename = "OWNER_EPOCH_CHANGED")]
    OwnerEpochChanged,
    /// Lignée serveur de la commande différente de la lignée active.
    #[serde(rename = "STALE_AUTHORITY_GENERATION")]
    StaleAuthorityGeneration,

    // --- ContractViolation ---
    /// Rôle ou slot déjà engagé dans une liaison immuable.
    #[serde(rename = "BINDING_ALREADY_ENGAGED")]
    BindingAlreadyEngaged,
    /// Référence de preuve devenue périmée ou non vérifiable.
    #[serde(rename = "STALE_EVIDENCE")]
    StaleEvidence,
    /// Critères de fin ou définition of done non satisfaits.
    #[serde(rename = "DONE_NOT_MET")]
    DoneNotMet,
    /// Borne d'itération maximale de boucle atteinte.
    #[serde(rename = "LOOP_EXHAUSTED")]
    LoopExhausted,
    /// Contrat formellement rejeté par le vérificateur.
    #[serde(rename = "CONTRACT_REJECTED")]
    ContractRejected,
    /// Tentative de mutation sur un agrégat terminal immuable.
    #[serde(rename = "TERMINAL_IMMUTABLE")]
    TerminalImmutable,
    /// Fenêtre d'idempotence expirée (tombstone minimal conservé).
    #[serde(rename = "IDEMPOTENCY_WINDOW_EXPIRED")]
    IdempotencyWindowExpired,
    /// Conflit lors de l'application d'un draft guidé.
    #[serde(rename = "APPLIED_DRAFT_CONFLICT")]
    AppliedDraftConflict,
    /// Tentative de modification d'un draft déjà appliqué.
    #[serde(rename = "APPLIED_DRAFT_IMMUTABLE")]
    AppliedDraftImmutable,
    /// Exécution de test attendue non fournie.
    #[serde(rename = "EXPECTED_EXECUTION_MISSING")]
    ExpectedExecutionMissing,
    /// Divergence entre observation d'exécution et attendu.
    #[serde(rename = "EVIDENCE_EXECUTION_MISMATCH")]
    EvidenceExecutionMismatch,
    /// Projection d'observation invalide ou corrompue.
    #[serde(rename = "EVIDENCE_PROJECTION_INVALID")]
    EvidenceProjectionInvalid,
    /// Observation ambiguë ne permettant pas de conclure.
    #[serde(rename = "EVIDENCE_AMBIGUOUS")]
    EvidenceAmbiguous,
    /// Divergence entre l'empreinte de snapshot et la preuve.
    #[serde(rename = "EVIDENCE_SNAPSHOT_MISMATCH")]
    EvidenceSnapshotMismatch,
    /// Affirmation formelle contredite par l'observation vérifiée.
    #[serde(rename = "CLAIM_CONTRADICTED")]
    ClaimContradicted,
    /// Reçu de recherche préalable obligatoire absent avant création.
    #[serde(rename = "DISCOVERY_REQUIRED")]
    DiscoveryRequired,
    /// Trace de recherche préalable manquante à la clôture.
    #[serde(rename = "DISCOVERY_EVIDENCE_MISSING")]
    DiscoveryEvidenceMissing,
    /// Restauration incomplète ou capsules manquantes.
    #[serde(rename = "RESTORE_INCOMPLETE")]
    RestoreIncomplete,
    /// Preuve obligatoire indisponible ou purgée.
    #[serde(rename = "EVIDENCE_UNAVAILABLE")]
    EvidenceUnavailable,
    /// Incompatibilité entre le mode d'automatisation et la politique de modèle.
    #[serde(rename = "AUTOMATION_MODE_POLICY_MISMATCH")]
    AutomationModePolicyMismatch,
    /// Bundle d'automatisation requis indisponible.
    #[serde(rename = "AUTOMATION_BUNDLE_UNAVAILABLE")]
    AutomationBundleUnavailable,
    /// Cible de l'automatisation absente ou introuvable.
    #[serde(rename = "AUTOMATION_TARGET_MISSING")]
    AutomationTargetMissing,
    /// Cible de l'automatisation ambiguë (plusieurs correspondances).
    #[serde(rename = "AUTOMATION_TARGET_AMBIGUOUS")]
    AutomationTargetAmbiguous,
    /// Dérive observée de l'identité de la cible métier.
    #[serde(rename = "AUTOMATION_TARGET_DRIFT")]
    AutomationTargetDrift,
    /// Contrainte métier violée avant dispatch.
    #[serde(rename = "BUSINESS_CONSTRAINT_VIOLATED")]
    BusinessConstraintViolated,
    /// Postcondition métier non vérifiée après dispatch.
    #[serde(rename = "BUSINESS_POSTCONDITION_FAILED")]
    BusinessPostconditionFailed,
    /// Incompatibilité de compte lors de l'exécution de l'automatisation.
    #[serde(rename = "AUTOMATION_ACCOUNT_MISMATCH")]
    AutomationAccountMismatch,
    /// État externe modifié de manière concurrente.
    #[serde(rename = "EXTERNAL_STATE_CHANGED")]
    ExternalStateChanged,
    /// Délai d'actionnabilité dépassé avant dispatch.
    #[serde(rename = "ACTIONABILITY_TIMEOUT")]
    ActionabilityTimeout,
    /// Délai d'assertion dépassé.
    #[serde(rename = "ASSERTION_TIMEOUT")]
    AssertionTimeout,
    /// Rôle incompatible avec le profil d'entrée ou les contraintes contractuelles.
    #[serde(rename = "INCOMPATIBLE_ROLE")]
    IncompatibleRole,
    /// Préconditions du reçu de découverte modifiées avant effet ou à la clôture.
    #[serde(rename = "RECEIPT_STALE")]
    ReceiptStale,
    /// Borne temporelle de validité du reçu de découverte expirée.
    #[serde(rename = "RECEIPT_VALIDATION_EXPIRED")]
    ReceiptValidationExpired,
    /// Chronologie de découverte ou réconciliation historique insuffisante.
    #[serde(rename = "DISCOVERY_HISTORY_UNKNOWN")]
    DiscoveryHistoryUnknown,
    /// Dépendance entre missions introduisant un cycle dans le graphe.
    #[serde(rename = "DEPENDENCY_CYCLE")]
    DependencyCycle,
    /// Conflit textuel lors de la préparation d'une intégration Git.
    #[serde(rename = "MERGE_CONFLICT")]
    MergeConflict,
    /// Validation obligatoire absente ou rattachée à un ancien candidat.
    #[serde(rename = "VALIDATION_MISSING")]
    ValidationMissing,
    /// Conflit de configuration de modèle ou de sélection.
    #[serde(rename = "MODEL_CONFIGURATION_CONFLICT")]
    ModelConfigurationConflict,
    /// Filiation de contexte incomplète pour satisfaire la politique d'inclusion.
    #[serde(rename = "CONTEXT_LINEAGE_INCOMPLETE")]
    ContextLineageIncomplete,
    /// Politiques de données inconciliables entre sources et destination.
    #[serde(rename = "DATA_POLICY_CONFLICT")]
    DataPolicyConflict,
    /// Requête refusée par le fournisseur de modèle (`ModelOutcome::refusal`).
    #[serde(rename = "PROVIDER_REFUSED")]
    ProviderRefused,
    /// Sortie modèle tronquée avant achèvement contractuel (`ModelOutcome::truncated`).
    #[serde(rename = "OUTPUT_TRUNCATED")]
    OutputTruncated,
    /// Snapshot de capacités locales périmé ou invalidé.
    #[serde(rename = "CAPABILITY_SNAPSHOT_STALE")]
    CapabilitySnapshotStale,
    /// Format de ressource documentaire non pris en charge.
    #[serde(rename = "UNSUPPORTED_FORMAT")]
    UnsupportedFormat,

    // --- OutcomeUnknown ---
    /// Effet externe sans observation probante : statut incertain.
    #[serde(rename = "UNRESOLVED_EFFECT")]
    UnresolvedEffect,
    /// Effet Git possiblement accompli mais non attribuable avec certitude.
    #[serde(rename = "GIT_OUTCOME_UNKNOWN")]
    GitOutcomeUnknown,
    /// Issue de lancement d'un processus ou terminal inconnue après coupure.
    #[serde(rename = "LAUNCH_OUTCOME_UNKNOWN")]
    LaunchOutcomeUnknown,
    /// Issue d'envoi d'entrée vers un terminal inconnue après interruption.
    #[serde(rename = "TERMINAL_INPUT_UNKNOWN")]
    TerminalInputUnknown,
    /// Issue d'un appel fournisseur modèle inconnue après envoi réseau.
    #[serde(rename = "MODEL_CALL_OUTCOME_UNKNOWN")]
    ModelCallOutcomeUnknown,

    // --- CapabilityDenied ---
    /// Capacité ou runtime manquant.
    #[serde(rename = "CAPABILITY_UNAVAILABLE")]
    CapabilityUnavailable,
    /// Décision humaine ou autorité habilitée requise.
    #[serde(rename = "DECISION_AUTHORITY_REQUIRED")]
    DecisionAuthorityRequired,
    /// Définition ou version révoquée : nouveaux démarrages interdits.
    #[serde(rename = "DEFINITION_REVOKED")]
    DefinitionRevoked,
    /// Appel modèle strictement interdit sous la politique en vigueur.
    #[serde(rename = "MODEL_CALLS_FORBIDDEN")]
    ModelCallsForbidden,
    /// Opération non autorisée par le mandat ou l'ACL active.
    #[serde(rename = "OPERATION_NOT_AUTHORIZED")]
    OperationNotAuthorized,
    /// Identité du pair ou de l'exécuteur non vérifiable.
    #[serde(rename = "IDENTITY_UNVERIFIED")]
    IdentityUnverified,
    /// Tentative d'élargissement de permissions par un auteur ou une version.
    #[serde(rename = "PERMISSION_EXPANSION_DENIED")]
    PermissionExpansionDenied,
    /// Inadéquation entre les capacités du rôle et le plafond du slot.
    #[serde(rename = "CAPABILITY_MISMATCH")]
    CapabilityMismatch,
    /// Action non autorisée par le moteur de contrats ou les capacités du slot.
    #[serde(rename = "UNAUTHORIZED_ACTION")]
    UnauthorizedAction,
    /// Serveur ou profil d'analyse de connaissances inutilisable.
    #[serde(rename = "PROFILE_UNAVAILABLE")]
    ProfileUnavailable,
    /// Politique Git ou stratégie demandée refusée.
    #[serde(rename = "POLICY_REFUSED")]
    PolicyRefused,
    /// Cible ou référence de conservation Git symbolique non supportée.
    #[serde(rename = "SYMBOLIC_TARGET_UNSUPPORTED")]
    SymbolicTargetUnsupported,
    /// Version de Git système inférieure au minimum requis (2.39).
    #[serde(rename = "GIT_VERSION_UNSUPPORTED")]
    GitVersionUnsupported,
    /// Outil externe requis non disponible pour le pool de terminaux.
    #[serde(rename = "TOOL_UNAVAILABLE")]
    ToolUnavailable,
    /// Autorité humaine manquante sur les sources ou destination pour déclassifier.
    #[serde(rename = "DECLASSIFICATION_AUTHORITY_REQUIRED")]
    DeclassificationAuthorityRequired,
    /// Référence distante de continuation de contexte interdite en V1.
    #[serde(rename = "REMOTE_CONTEXT_REFERENCE_FORBIDDEN")]
    RemoteContextReferenceForbidden,
    /// Violation de la politique de localité des données (`local_only`).
    #[serde(rename = "DATA_LOCALITY_VIOLATION")]
    DataLocalityViolation,
    /// Lieu réel d'inférence inconnu sous une politique exigeant la localité.
    #[serde(rename = "INFERENCE_LOCALITY_UNKNOWN")]
    InferenceLocalityUnknown,
    /// Authentification du connecteur documentaire requise.
    #[serde(rename = "CONNECTOR_AUTH_REQUIRED")]
    ConnectorAuthRequired,
    /// Accès du connecteur documentaire révoqué.
    #[serde(rename = "CONNECTOR_ACCESS_REVOKED")]
    ConnectorAccessRevoked,
    /// Accès du connecteur documentaire non vérifiable (403 de raison inconnue).
    #[serde(rename = "CONNECTOR_ACCESS_UNVERIFIED")]
    ConnectorAccessUnverified,
    /// Application cliente non autorisée sur la ressource du connecteur.
    #[serde(rename = "CONNECTOR_APP_NOT_AUTHORIZED")]
    ConnectorAppNotAuthorized,
    /// Identité du compte de connecteur différente de la liaison attendue.
    #[serde(rename = "ACCOUNT_IDENTITY_MISMATCH")]
    AccountIdentityMismatch,
    /// Trousseau ou coffre de secrets verrouillé.
    #[serde(rename = "SECRET_STORE_LOCKED")]
    SecretStoreLocked,
    /// Accès au secret refusé par la politique d'autorisation.
    #[serde(rename = "SECRET_ACCESS_DENIED")]
    SecretAccessDenied,
    /// Action refusée par la politique locale de sécurité.
    #[serde(rename = "POLICY_DENIED")]
    PolicyDenied,
    /// Action refusée par le port AI-Guard.
    #[serde(rename = "GUARD_DENIED")]
    GuardDenied,
    /// Réponse Allow d'AI-Guard porteuse d'une obligation non satisfaite ou non acceptée.
    #[serde(rename = "GUARD_OBLIGATION_UNSATISFIED")]
    GuardObligationUnsatisfied,
    /// Approbation humaine requise par AI-Guard sur le scope et digest exacts.
    #[serde(rename = "GUARD_APPROVAL_REQUIRED")]
    GuardApprovalRequired,
    /// Demande d'approbation AI-Guard non prise en charge par le profil de mapping.
    #[serde(rename = "GUARD_APPROVAL_UNSUPPORTED")]
    GuardApprovalUnsupported,
    /// Service AI-Guard indisponible, en timeout ou réponse malformée.
    #[serde(rename = "GUARD_UNAVAILABLE")]
    GuardUnavailable,
    /// Sujet suspendu localement (ou via AI-Guard).
    #[serde(rename = "SUBJECT_SUSPENDED")]
    SubjectSuspended,
    /// Sujet définitivement révoqué.
    #[serde(rename = "SUBJECT_REVOKED")]
    SubjectRevoked,

    // --- ScopeDenied ---
    /// Source ou ressource hors autorisation du scope courant.
    #[serde(rename = "ACCESS_DENIED")]
    AccessDenied,
    /// Origine ou attribution du reçu de découverte incompatible avec le canal ou mandat.
    #[serde(rename = "DISCOVERY_ORIGIN_MISMATCH")]
    DiscoveryOriginMismatch,
    /// Autorisation d'accès aux preuves ou rapports révoquée.
    #[serde(rename = "ACCESS_REVOKED")]
    AccessRevoked,

    // --- BudgetExceeded ---
    /// Budget monétaire ou de tokens épuisé.
    #[serde(rename = "BUDGET_EXHAUSTED")]
    BudgetExhausted,
    /// Budget alloué à la supervision atteint.
    #[serde(rename = "SUPERVISION_BUDGET_EXHAUSTED")]
    SupervisionBudgetExhausted,
    /// Budget de taille de contexte dépassé sans compaction possible.
    #[serde(rename = "CONTEXT_BUDGET_EXCEEDED")]
    ContextBudgetExceeded,
    /// Catalogue tarifaire périmé sous politique `pricing_stale_policy = block`.
    #[serde(rename = "PRICING_STALE")]
    PricingStale,
    /// Modèle distant sans tarif applicable dans le catalogue.
    #[serde(rename = "PRICING_UNKNOWN")]
    PricingUnknown,

    // --- RateLimited ---
    /// Limite de débit atteinte auprès d'un fournisseur externe (429/529 ou quota).
    #[serde(rename = "PROVIDER_RATE_LIMITED")]
    ProviderRateLimited,

    // --- ResourceBusy ---
    /// Le prédécesseur du Run n'a pas encore atteint un état terminal.
    #[serde(rename = "PREDECESSOR_NOT_TERMINAL")]
    PredecessorNotTerminal,
    /// Publication ou rattachement visant un objet en cours de purge.
    #[serde(rename = "OBJECT_DELETING")]
    ObjectDeleting,
    /// File d'attente du writer saturée (backpressure).
    #[serde(rename = "BACKPRESSURE")]
    Backpressure,
    /// Base de données momentanément verrouillée (`SQLITE_BUSY`).
    #[serde(rename = "STORAGE_BUSY")]
    StorageBusy,
    /// Espace de stockage disponible sous le seuil critique (1 GiB).
    #[serde(rename = "STORAGE_LOW_SPACE")]
    StorageLowSpace,
    /// Procédure de reprise après incident en cours d'exécution.
    #[serde(rename = "RECOVERY_IN_PROGRESS")]
    RecoveryInProgress,
    /// Une autorité active est déjà engagée pour cette instance.
    #[serde(rename = "AUTHORITY_ALREADY_ACTIVE")]
    AuthorityAlreadyActive,
    /// Un daemon actif tourne déjà pour ce compte utilisateur.
    #[serde(rename = "DAEMON_ALREADY_RUNNING")]
    DaemonAlreadyRunning,
    /// Procédure de restauration bloquée par un prérequis manquant.
    #[serde(rename = "RESTORATION_BLOCKED")]
    RestorationBlocked,
    /// Incohérence de comptabilité ou d'inventaire des ressources.
    #[serde(rename = "ACCOUNTING_MISMATCH")]
    AccountingMismatch,
    /// Intervention humaine obligatoire avant de poursuivre l'automatisation.
    #[serde(rename = "AUTOMATION_HUMAN_STEP_REQUIRED")]
    AutomationHumanStepRequired,
    /// Mutation détectée pendant la capture d'un snapshot de connaissances.
    #[serde(rename = "SNAPSHOT_INCONSISTENT")]
    SnapshotInconsistent,
    /// Génération de l'index de connaissances en retard sur la source requise.
    #[serde(rename = "INDEX_STALE")]
    IndexStale,
    /// Génération de la source avancée avant publication de l'index.
    #[serde(rename = "SOURCE_ADVANCED")]
    SourceAdvanced,
    /// La branche cible Git est actuellement checkout dans un worktree.
    #[serde(rename = "TARGET_CHECKED_OUT")]
    TargetCheckedOut,
    /// Chemin d'allocation de worktree déjà occupé.
    #[serde(rename = "PATH_OCCUPIED")]
    PathOccupied,
    /// Branche Git déjà checkout ou allouée à un autre worktree.
    #[serde(rename = "BRANCH_IN_USE")]
    BranchInUse,
    /// Worktree contenant du travail non committé ou non atteignable.
    #[serde(rename = "DIRTY_WORKTREE")]
    DirtyWorktree,
    /// Un processus actif utilise encore le worktree ciblé.
    #[serde(rename = "ACTIVE_PROCESS")]
    ActiveProcess,
    /// Terminal momentanément indisponible ou en attente de réconciliation.
    #[serde(rename = "TERMINAL_UNAVAILABLE")]
    TerminalUnavailable,
    /// Port réseau déjà utilisé lors de l'allocation d'un service local.
    #[serde(rename = "PORT_IN_USE")]
    PortInUse,
    /// Fournisseur de modèle distant momentanément indisponible.
    #[serde(rename = "PROVIDER_UNAVAILABLE")]
    ProviderUnavailable,
    /// Modèle ou serveur d'inférence local momentanément indisponible.
    #[serde(rename = "LOCAL_MODEL_UNAVAILABLE")]
    LocalModelUnavailable,
    /// File d'attente d'inférence du serveur local pleine.
    #[serde(rename = "LOCAL_QUEUE_FULL")]
    LocalQueueFull,
    /// Délai de chargement du modèle local dépassé.
    #[serde(rename = "LOCAL_LOAD_TIMEOUT")]
    LocalLoadTimeout,
    /// Ressources matérielles locales épuisées (ex: mémoire / VRAM).
    #[serde(rename = "LOCAL_RESOURCE_EXHAUSTED")]
    LocalResourceExhausted,

    // --- InvalidArgument ---
    /// Le prédécesseur a déjà un successeur direct.
    #[serde(rename = "SUCCESSOR_EXISTS")]
    SuccessorExists,
    /// Incohérence détectée lors de l'upload de segments par le client.
    #[serde(rename = "CLIENT_UPLOAD_MISMATCH")]
    ClientUploadMismatch,
    /// Paramètres de réserve de contrôle incohérents.
    #[serde(rename = "CONTROL_RESERVE_INCOHERENT")]
    ControlReserveIncoherent,
    /// Données d'entrée d'automatisation (CSV/mapping) invalides.
    #[serde(rename = "AUTOMATION_INPUT_INVALID")]
    AutomationInputInvalid,
    /// Validateur de contrat inconnu au registre local.
    #[serde(rename = "UNKNOWN_VALIDATOR")]
    UnknownValidator,
    /// Paramètres de règle contractuelle invalides.
    #[serde(rename = "INVALID_RULE_PARAMETERS")]
    InvalidRuleParameters,
    /// Prédicat de claim non reconnu par le moteur.
    #[serde(rename = "UNKNOWN_CLAIM_PREDICATE")]
    UnknownClaimPredicate,
    /// Taille ou complexité du document dépassant la limite autorisée.
    #[serde(rename = "SCHEMA_LIMIT_EXCEEDED")]
    SchemaLimitExceeded,
    /// Référence non qualifiée correspondant à plusieurs entrées de registre.
    #[serde(rename = "AMBIGUOUS_REFERENCE")]
    AmbiguousReference,
    /// Dépôt ou branche source unborn sans commit candidat à intégrer.
    #[serde(rename = "SOURCE_NOT_COMMITTED")]
    SourceNotCommitted,
    /// Configuration ou dialecte de profil modèle invalide.
    #[serde(rename = "INVALID_MODEL_CONFIG")]
    InvalidModelConfig,
    /// Classe d'effet inconnue dans l'`ActionIntent`.
    #[serde(rename = "EFFECTS_CLASS_UNKNOWN")]
    EffectsClassUnknown,

    // --- InternalFailure ---
    /// Disque plein lors de l'écriture en base (`SQLITE_FULL`).
    #[serde(rename = "STORAGE_FULL")]
    StorageFull,
    /// Erreur d'I/O système sur le stockage (`SQLITE_IOERR`).
    #[serde(rename = "STORAGE_UNAVAILABLE")]
    StorageUnavailable,
    /// Base de données corrompue (`SQLITE_CORRUPT`).
    #[serde(rename = "STORAGE_CORRUPT")]
    StorageCorrupt,
    /// Build `SQLite` compilé non conforme aux exigences qualifiées.
    #[serde(rename = "STORAGE_BUILD_UNQUALIFIED")]
    StorageBuildUnqualified,
    /// Invariant de domaine critique violé en base.
    #[serde(rename = "INVARIANT_VIOLATION")]
    InvariantViolation,
    /// Journal durable d'événements ou d'audit indisponible en écriture.
    #[serde(rename = "JOURNAL_UNAVAILABLE")]
    JournalUnavailable,
    /// Backend du coffre de secrets momentanément indisponible.
    #[serde(rename = "SECRET_STORE_UNAVAILABLE")]
    SecretStoreUnavailable,

    // --- NotFoundOrHidden ---
    /// Preuve obligatoire introuvable dans le journal.
    #[serde(rename = "EVIDENCE_MISSING")]
    EvidenceMissing,
    /// Dépendance de skill ou version requise absente ou masquée.
    #[serde(rename = "MISSING_DEPENDENCY")]
    MissingDependency,
    /// Bundle d'évaluation ou de version épinglé indisponible à la reprise.
    #[serde(rename = "BUNDLE_UNAVAILABLE")]
    BundleUnavailable,
    /// Source de connaissances ou référence Git absente, purgée ou introuvable.
    #[serde(rename = "SOURCE_MISSING")]
    SourceMissing,
    /// Ressource documentaire absente ou masquée par contrôle d'accès.
    #[serde(rename = "RESOURCE_MISSING_OR_HIDDEN")]
    ResourceMissingOrHidden,
    /// Rapport ou archive historique absent ou purgé.
    #[serde(rename = "HISTORY_UNAVAILABLE")]
    HistoryUnavailable,
    /// Référence de secret introuvable dans le coffre.
    #[serde(rename = "SECRET_REF_NOT_FOUND")]
    SecretRefNotFound,

    // --- IsolationUnavailable ---
    /// Profil d'isolation navigateur ou système non disponible.
    #[serde(rename = "AUTOMATION_PROFILE_UNAVAILABLE")]
    AutomationProfileUnavailable,
    /// Backend d'isolation OS requis absent ou en échec en mode strict.
    #[serde(rename = "ISOLATION_BACKEND_MISSING")]
    IsolationBackendMissing,
    /// Obligation d'isolation requise non imposable par le backend OS actif.
    #[serde(rename = "ISOLATION_OBLIGATION_UNENFORCEABLE")]
    IsolationObligationUnenforceable,

    // --- ProtocolIncompatible ---
    /// Capacité ou version de protocole AI-Guard non négociée ou incompatible.
    #[serde(rename = "GUARD_PROTOCOL_UNSUPPORTED")]
    GuardProtocolUnsupported,

    // --- ResyncRequired ---
    /// Curseur de lecture incrémentale expiré.
    #[serde(rename = "CURSOR_EXPIRED")]
    CursorExpired,
}

impl ReasonCode {
    /// Renvoie la classe d'erreur unique [`ErrorCode`] associée à cette raison.
    #[expect(
        clippy::too_many_lines,
        reason = "Correspondance exhaustive des 156 ReasonCode canoniques enregistrés"
    )]
    #[must_use]
    pub const fn error_code(&self) -> ErrorCode {
        match self {
            Self::PublicationConflict
            | Self::ContractVersionConflict
            | Self::TargetMoved
            | Self::SourceChanged
            | Self::TargetAlreadyExists => ErrorCode::RevisionConflict,

            Self::StaleEpoch | Self::OwnerEpochChanged | Self::StaleAuthorityGeneration => {
                ErrorCode::StaleFence
            }

            Self::BindingAlreadyEngaged
            | Self::StaleEvidence
            | Self::DoneNotMet
            | Self::LoopExhausted
            | Self::ContractRejected
            | Self::TerminalImmutable
            | Self::IdempotencyWindowExpired
            | Self::AppliedDraftConflict
            | Self::AppliedDraftImmutable
            | Self::ExpectedExecutionMissing
            | Self::EvidenceExecutionMismatch
            | Self::EvidenceProjectionInvalid
            | Self::EvidenceAmbiguous
            | Self::EvidenceSnapshotMismatch
            | Self::ClaimContradicted
            | Self::DiscoveryRequired
            | Self::DiscoveryEvidenceMissing
            | Self::RestoreIncomplete
            | Self::EvidenceUnavailable
            | Self::AutomationModePolicyMismatch
            | Self::AutomationBundleUnavailable
            | Self::AutomationTargetMissing
            | Self::AutomationTargetAmbiguous
            | Self::AutomationTargetDrift
            | Self::BusinessConstraintViolated
            | Self::BusinessPostconditionFailed
            | Self::AutomationAccountMismatch
            | Self::ExternalStateChanged
            | Self::ActionabilityTimeout
            | Self::AssertionTimeout
            | Self::IncompatibleRole
            | Self::ReceiptStale
            | Self::ReceiptValidationExpired
            | Self::DiscoveryHistoryUnknown
            | Self::DependencyCycle
            | Self::MergeConflict
            | Self::ValidationMissing
            | Self::ModelConfigurationConflict
            | Self::ContextLineageIncomplete
            | Self::DataPolicyConflict
            | Self::ProviderRefused
            | Self::OutputTruncated
            | Self::CapabilitySnapshotStale
            | Self::UnsupportedFormat => ErrorCode::ContractViolation,

            Self::UnresolvedEffect
            | Self::GitOutcomeUnknown
            | Self::LaunchOutcomeUnknown
            | Self::TerminalInputUnknown
            | Self::ModelCallOutcomeUnknown => ErrorCode::OutcomeUnknown,

            Self::CapabilityUnavailable
            | Self::DecisionAuthorityRequired
            | Self::DefinitionRevoked
            | Self::ModelCallsForbidden
            | Self::OperationNotAuthorized
            | Self::IdentityUnverified
            | Self::PermissionExpansionDenied
            | Self::CapabilityMismatch
            | Self::UnauthorizedAction
            | Self::ProfileUnavailable
            | Self::PolicyRefused
            | Self::SymbolicTargetUnsupported
            | Self::GitVersionUnsupported
            | Self::ToolUnavailable
            | Self::DeclassificationAuthorityRequired
            | Self::RemoteContextReferenceForbidden
            | Self::DataLocalityViolation
            | Self::InferenceLocalityUnknown
            | Self::ConnectorAuthRequired
            | Self::ConnectorAccessRevoked
            | Self::ConnectorAccessUnverified
            | Self::ConnectorAppNotAuthorized
            | Self::AccountIdentityMismatch
            | Self::SecretStoreLocked
            | Self::SecretAccessDenied
            | Self::PolicyDenied
            | Self::GuardDenied
            | Self::GuardObligationUnsatisfied
            | Self::GuardApprovalRequired
            | Self::GuardApprovalUnsupported
            | Self::GuardUnavailable
            | Self::SubjectSuspended
            | Self::SubjectRevoked => ErrorCode::CapabilityDenied,

            Self::AccessDenied | Self::DiscoveryOriginMismatch | Self::AccessRevoked => {
                ErrorCode::ScopeDenied
            }

            Self::BudgetExhausted
            | Self::SupervisionBudgetExhausted
            | Self::ContextBudgetExceeded
            | Self::PricingStale
            | Self::PricingUnknown => ErrorCode::BudgetExceeded,

            Self::ProviderRateLimited => ErrorCode::RateLimited,

            Self::PredecessorNotTerminal
            | Self::ObjectDeleting
            | Self::Backpressure
            | Self::StorageBusy
            | Self::StorageLowSpace
            | Self::RecoveryInProgress
            | Self::AuthorityAlreadyActive
            | Self::DaemonAlreadyRunning
            | Self::RestorationBlocked
            | Self::AccountingMismatch
            | Self::AutomationHumanStepRequired
            | Self::SnapshotInconsistent
            | Self::IndexStale
            | Self::SourceAdvanced
            | Self::TargetCheckedOut
            | Self::PathOccupied
            | Self::BranchInUse
            | Self::DirtyWorktree
            | Self::ActiveProcess
            | Self::TerminalUnavailable
            | Self::PortInUse
            | Self::ProviderUnavailable
            | Self::LocalModelUnavailable
            | Self::LocalQueueFull
            | Self::LocalLoadTimeout
            | Self::LocalResourceExhausted => ErrorCode::ResourceBusy,

            Self::SuccessorExists
            | Self::ClientUploadMismatch
            | Self::ControlReserveIncoherent
            | Self::AutomationInputInvalid
            | Self::UnknownValidator
            | Self::InvalidRuleParameters
            | Self::UnknownClaimPredicate
            | Self::SchemaLimitExceeded
            | Self::AmbiguousReference
            | Self::SourceNotCommitted
            | Self::InvalidModelConfig
            | Self::EffectsClassUnknown => ErrorCode::InvalidArgument,

            Self::StorageFull
            | Self::StorageUnavailable
            | Self::StorageCorrupt
            | Self::StorageBuildUnqualified
            | Self::InvariantViolation
            | Self::JournalUnavailable
            | Self::SecretStoreUnavailable => ErrorCode::InternalFailure,

            Self::EvidenceMissing
            | Self::MissingDependency
            | Self::BundleUnavailable
            | Self::SourceMissing
            | Self::ResourceMissingOrHidden
            | Self::HistoryUnavailable
            | Self::SecretRefNotFound => ErrorCode::NotFoundOrHidden,

            Self::AutomationProfileUnavailable
            | Self::IsolationBackendMissing
            | Self::IsolationObligationUnenforceable => ErrorCode::IsolationUnavailable,

            Self::GuardProtocolUnsupported => ErrorCode::ProtocolIncompatible,

            Self::CursorExpired => ErrorCode::ResyncRequired,
        }
    }

    /// Renvoie la politique de reprise par défaut ([`RetryClass`]) stipulée par le contrat canonique.
    #[expect(
        clippy::too_many_lines,
        reason = "Correspondance exhaustive des 156 ReasonCode canoniques enregistrés"
    )]
    #[must_use]
    pub const fn default_retry_class(&self) -> RetryClass {
        match self {
            Self::PublicationConflict
            | Self::ContractVersionConflict
            | Self::TargetMoved
            | Self::SourceChanged
            | Self::TargetAlreadyExists
            | Self::OwnerEpochChanged
            | Self::StaleEvidence
            | Self::EvidenceSnapshotMismatch
            | Self::ReceiptStale
            | Self::SnapshotInconsistent
            | Self::IndexStale
            | Self::SourceAdvanced
            | Self::PricingStale
            | Self::ConnectorAccessUnverified
            | Self::PredecessorNotTerminal
            | Self::ObjectDeleting
            | Self::CursorExpired => RetryClass::AfterRefresh,

            Self::StaleEpoch
            | Self::UnresolvedEffect
            | Self::GitOutcomeUnknown
            | Self::LaunchOutcomeUnknown
            | Self::TerminalInputUnknown
            | Self::ModelCallOutcomeUnknown
            | Self::IdentityUnverified
            | Self::ExternalStateChanged
            | Self::ReceiptValidationExpired
            | Self::DiscoveryHistoryUnknown
            | Self::TerminalUnavailable => RetryClass::AfterReconciliation,

            Self::Backpressure
            | Self::StorageBusy
            | Self::RecoveryInProgress
            | Self::AuthorityAlreadyActive
            | Self::StorageUnavailable
            | Self::ActionabilityTimeout
            | Self::AssertionTimeout
            | Self::PortInUse
            | Self::ProviderRateLimited
            | Self::ProviderUnavailable
            | Self::LocalModelUnavailable
            | Self::LocalQueueFull
            | Self::LocalLoadTimeout
            | Self::JournalUnavailable
            | Self::SecretStoreUnavailable
            | Self::GuardUnavailable => RetryClass::AfterBackoff,

            Self::DoneNotMet
            | Self::LoopExhausted
            | Self::ContractRejected
            | Self::ExpectedExecutionMissing
            | Self::EvidenceAmbiguous
            | Self::RestoreIncomplete
            | Self::AutomationBundleUnavailable
            | Self::AutomationTargetMissing
            | Self::AutomationTargetAmbiguous
            | Self::AutomationTargetDrift
            | Self::BusinessConstraintViolated
            | Self::BusinessPostconditionFailed
            | Self::AutomationAccountMismatch
            | Self::MergeConflict
            | Self::ValidationMissing
            | Self::ModelConfigurationConflict
            | Self::ContextLineageIncomplete
            | Self::DataPolicyConflict
            | Self::ProviderRefused
            | Self::OutputTruncated
            | Self::CapabilitySnapshotStale
            | Self::CapabilityUnavailable
            | Self::DecisionAuthorityRequired
            | Self::CapabilityMismatch
            | Self::ProfileUnavailable
            | Self::PolicyRefused
            | Self::GitVersionUnsupported
            | Self::ToolUnavailable
            | Self::DeclassificationAuthorityRequired
            | Self::InferenceLocalityUnknown
            | Self::ConnectorAuthRequired
            | Self::ConnectorAppNotAuthorized
            | Self::AccountIdentityMismatch
            | Self::SecretStoreLocked
            | Self::GuardApprovalRequired
            | Self::SubjectSuspended
            | Self::AccessRevoked
            | Self::BudgetExhausted
            | Self::SupervisionBudgetExhausted
            | Self::ContextBudgetExceeded
            | Self::PricingUnknown
            | Self::StorageLowSpace
            | Self::RestorationBlocked
            | Self::AccountingMismatch
            | Self::AutomationHumanStepRequired
            | Self::TargetCheckedOut
            | Self::PathOccupied
            | Self::BranchInUse
            | Self::DirtyWorktree
            | Self::ActiveProcess
            | Self::LocalResourceExhausted
            | Self::SourceNotCommitted
            | Self::StorageFull
            | Self::MissingDependency
            | Self::BundleUnavailable
            | Self::SourceMissing
            | Self::AutomationProfileUnavailable
            | Self::IsolationBackendMissing
            | Self::IsolationObligationUnenforceable => RetryClass::AfterHumanDecision,

            Self::StaleAuthorityGeneration
            | Self::BindingAlreadyEngaged
            | Self::TerminalImmutable
            | Self::IdempotencyWindowExpired
            | Self::AppliedDraftConflict
            | Self::AppliedDraftImmutable
            | Self::EvidenceExecutionMismatch
            | Self::EvidenceProjectionInvalid
            | Self::ClaimContradicted
            | Self::DiscoveryRequired
            | Self::DiscoveryEvidenceMissing
            | Self::EvidenceUnavailable
            | Self::AutomationModePolicyMismatch
            | Self::IncompatibleRole
            | Self::DependencyCycle
            | Self::UnsupportedFormat
            | Self::DefinitionRevoked
            | Self::ModelCallsForbidden
            | Self::OperationNotAuthorized
            | Self::PermissionExpansionDenied
            | Self::UnauthorizedAction
            | Self::SymbolicTargetUnsupported
            | Self::RemoteContextReferenceForbidden
            | Self::DataLocalityViolation
            | Self::ConnectorAccessRevoked
            | Self::SecretAccessDenied
            | Self::PolicyDenied
            | Self::GuardDenied
            | Self::GuardObligationUnsatisfied
            | Self::GuardApprovalUnsupported
            | Self::SubjectRevoked
            | Self::AccessDenied
            | Self::DiscoveryOriginMismatch
            | Self::DaemonAlreadyRunning
            | Self::SuccessorExists
            | Self::ClientUploadMismatch
            | Self::ControlReserveIncoherent
            | Self::AutomationInputInvalid
            | Self::UnknownValidator
            | Self::InvalidRuleParameters
            | Self::UnknownClaimPredicate
            | Self::SchemaLimitExceeded
            | Self::AmbiguousReference
            | Self::InvalidModelConfig
            | Self::EffectsClassUnknown
            | Self::StorageCorrupt
            | Self::StorageBuildUnqualified
            | Self::InvariantViolation
            | Self::EvidenceMissing
            | Self::ResourceMissingOrHidden
            | Self::HistoryUnavailable
            | Self::SecretRefNotFound
            | Self::GuardProtocolUnsupported => RetryClass::Never,
        }
    }

    /// Renvoie la chaîne textuelle wire canonique (`UPPER_SNAKE`).
    #[expect(
        clippy::too_many_lines,
        reason = "Correspondance exhaustive des 156 ReasonCode canoniques enregistrés"
    )]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PublicationConflict => "PUBLICATION_CONFLICT",
            Self::ContractVersionConflict => "CONTRACT_VERSION_CONFLICT",
            Self::TargetMoved => "TARGET_MOVED",
            Self::SourceChanged => "SOURCE_CHANGED",
            Self::TargetAlreadyExists => "TARGET_ALREADY_EXISTS",
            Self::StaleEpoch => "STALE_EPOCH",
            Self::OwnerEpochChanged => "OWNER_EPOCH_CHANGED",
            Self::StaleAuthorityGeneration => "STALE_AUTHORITY_GENERATION",
            Self::BindingAlreadyEngaged => "BINDING_ALREADY_ENGAGED",
            Self::StaleEvidence => "STALE_EVIDENCE",
            Self::DoneNotMet => "DONE_NOT_MET",
            Self::LoopExhausted => "LOOP_EXHAUSTED",
            Self::ContractRejected => "CONTRACT_REJECTED",
            Self::TerminalImmutable => "TERMINAL_IMMUTABLE",
            Self::IdempotencyWindowExpired => "IDEMPOTENCY_WINDOW_EXPIRED",
            Self::AppliedDraftConflict => "APPLIED_DRAFT_CONFLICT",
            Self::AppliedDraftImmutable => "APPLIED_DRAFT_IMMUTABLE",
            Self::ExpectedExecutionMissing => "EXPECTED_EXECUTION_MISSING",
            Self::EvidenceExecutionMismatch => "EVIDENCE_EXECUTION_MISMATCH",
            Self::EvidenceProjectionInvalid => "EVIDENCE_PROJECTION_INVALID",
            Self::EvidenceAmbiguous => "EVIDENCE_AMBIGUOUS",
            Self::EvidenceSnapshotMismatch => "EVIDENCE_SNAPSHOT_MISMATCH",
            Self::ClaimContradicted => "CLAIM_CONTRADICTED",
            Self::DiscoveryRequired => "DISCOVERY_REQUIRED",
            Self::DiscoveryEvidenceMissing => "DISCOVERY_EVIDENCE_MISSING",
            Self::RestoreIncomplete => "RESTORE_INCOMPLETE",
            Self::EvidenceUnavailable => "EVIDENCE_UNAVAILABLE",
            Self::AutomationModePolicyMismatch => "AUTOMATION_MODE_POLICY_MISMATCH",
            Self::AutomationBundleUnavailable => "AUTOMATION_BUNDLE_UNAVAILABLE",
            Self::AutomationTargetMissing => "AUTOMATION_TARGET_MISSING",
            Self::AutomationTargetAmbiguous => "AUTOMATION_TARGET_AMBIGUOUS",
            Self::AutomationTargetDrift => "AUTOMATION_TARGET_DRIFT",
            Self::BusinessConstraintViolated => "BUSINESS_CONSTRAINT_VIOLATED",
            Self::BusinessPostconditionFailed => "BUSINESS_POSTCONDITION_FAILED",
            Self::AutomationAccountMismatch => "AUTOMATION_ACCOUNT_MISMATCH",
            Self::ExternalStateChanged => "EXTERNAL_STATE_CHANGED",
            Self::ActionabilityTimeout => "ACTIONABILITY_TIMEOUT",
            Self::AssertionTimeout => "ASSERTION_TIMEOUT",
            Self::IncompatibleRole => "INCOMPATIBLE_ROLE",
            Self::ReceiptStale => "RECEIPT_STALE",
            Self::ReceiptValidationExpired => "RECEIPT_VALIDATION_EXPIRED",
            Self::DiscoveryHistoryUnknown => "DISCOVERY_HISTORY_UNKNOWN",
            Self::DependencyCycle => "DEPENDENCY_CYCLE",
            Self::MergeConflict => "MERGE_CONFLICT",
            Self::ValidationMissing => "VALIDATION_MISSING",
            Self::ModelConfigurationConflict => "MODEL_CONFIGURATION_CONFLICT",
            Self::ContextLineageIncomplete => "CONTEXT_LINEAGE_INCOMPLETE",
            Self::DataPolicyConflict => "DATA_POLICY_CONFLICT",
            Self::ProviderRefused => "PROVIDER_REFUSED",
            Self::OutputTruncated => "OUTPUT_TRUNCATED",
            Self::CapabilitySnapshotStale => "CAPABILITY_SNAPSHOT_STALE",
            Self::UnsupportedFormat => "UNSUPPORTED_FORMAT",
            Self::UnresolvedEffect => "UNRESOLVED_EFFECT",
            Self::GitOutcomeUnknown => "GIT_OUTCOME_UNKNOWN",
            Self::LaunchOutcomeUnknown => "LAUNCH_OUTCOME_UNKNOWN",
            Self::TerminalInputUnknown => "TERMINAL_INPUT_UNKNOWN",
            Self::ModelCallOutcomeUnknown => "MODEL_CALL_OUTCOME_UNKNOWN",
            Self::CapabilityUnavailable => "CAPABILITY_UNAVAILABLE",
            Self::DecisionAuthorityRequired => "DECISION_AUTHORITY_REQUIRED",
            Self::DefinitionRevoked => "DEFINITION_REVOKED",
            Self::ModelCallsForbidden => "MODEL_CALLS_FORBIDDEN",
            Self::OperationNotAuthorized => "OPERATION_NOT_AUTHORIZED",
            Self::IdentityUnverified => "IDENTITY_UNVERIFIED",
            Self::PermissionExpansionDenied => "PERMISSION_EXPANSION_DENIED",
            Self::CapabilityMismatch => "CAPABILITY_MISMATCH",
            Self::UnauthorizedAction => "UNAUTHORIZED_ACTION",
            Self::ProfileUnavailable => "PROFILE_UNAVAILABLE",
            Self::PolicyRefused => "POLICY_REFUSED",
            Self::SymbolicTargetUnsupported => "SYMBOLIC_TARGET_UNSUPPORTED",
            Self::GitVersionUnsupported => "GIT_VERSION_UNSUPPORTED",
            Self::ToolUnavailable => "TOOL_UNAVAILABLE",
            Self::DeclassificationAuthorityRequired => "DECLASSIFICATION_AUTHORITY_REQUIRED",
            Self::RemoteContextReferenceForbidden => "REMOTE_CONTEXT_REFERENCE_FORBIDDEN",
            Self::DataLocalityViolation => "DATA_LOCALITY_VIOLATION",
            Self::InferenceLocalityUnknown => "INFERENCE_LOCALITY_UNKNOWN",
            Self::ConnectorAuthRequired => "CONNECTOR_AUTH_REQUIRED",
            Self::ConnectorAccessRevoked => "CONNECTOR_ACCESS_REVOKED",
            Self::ConnectorAccessUnverified => "CONNECTOR_ACCESS_UNVERIFIED",
            Self::ConnectorAppNotAuthorized => "CONNECTOR_APP_NOT_AUTHORIZED",
            Self::AccountIdentityMismatch => "ACCOUNT_IDENTITY_MISMATCH",
            Self::SecretStoreLocked => "SECRET_STORE_LOCKED",
            Self::SecretAccessDenied => "SECRET_ACCESS_DENIED",
            Self::PolicyDenied => "POLICY_DENIED",
            Self::GuardDenied => "GUARD_DENIED",
            Self::GuardObligationUnsatisfied => "GUARD_OBLIGATION_UNSATISFIED",
            Self::GuardApprovalRequired => "GUARD_APPROVAL_REQUIRED",
            Self::GuardApprovalUnsupported => "GUARD_APPROVAL_UNSUPPORTED",
            Self::GuardUnavailable => "GUARD_UNAVAILABLE",
            Self::SubjectSuspended => "SUBJECT_SUSPENDED",
            Self::SubjectRevoked => "SUBJECT_REVOKED",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::DiscoveryOriginMismatch => "DISCOVERY_ORIGIN_MISMATCH",
            Self::AccessRevoked => "ACCESS_REVOKED",
            Self::BudgetExhausted => "BUDGET_EXHAUSTED",
            Self::SupervisionBudgetExhausted => "SUPERVISION_BUDGET_EXHAUSTED",
            Self::ContextBudgetExceeded => "CONTEXT_BUDGET_EXCEEDED",
            Self::PricingStale => "PRICING_STALE",
            Self::PricingUnknown => "PRICING_UNKNOWN",
            Self::ProviderRateLimited => "PROVIDER_RATE_LIMITED",
            Self::PredecessorNotTerminal => "PREDECESSOR_NOT_TERMINAL",
            Self::ObjectDeleting => "OBJECT_DELETING",
            Self::Backpressure => "BACKPRESSURE",
            Self::StorageBusy => "STORAGE_BUSY",
            Self::StorageLowSpace => "STORAGE_LOW_SPACE",
            Self::RecoveryInProgress => "RECOVERY_IN_PROGRESS",
            Self::AuthorityAlreadyActive => "AUTHORITY_ALREADY_ACTIVE",
            Self::DaemonAlreadyRunning => "DAEMON_ALREADY_RUNNING",
            Self::RestorationBlocked => "RESTORATION_BLOCKED",
            Self::AccountingMismatch => "ACCOUNTING_MISMATCH",
            Self::AutomationHumanStepRequired => "AUTOMATION_HUMAN_STEP_REQUIRED",
            Self::SnapshotInconsistent => "SNAPSHOT_INCONSISTENT",
            Self::IndexStale => "INDEX_STALE",
            Self::SourceAdvanced => "SOURCE_ADVANCED",
            Self::TargetCheckedOut => "TARGET_CHECKED_OUT",
            Self::PathOccupied => "PATH_OCCUPIED",
            Self::BranchInUse => "BRANCH_IN_USE",
            Self::DirtyWorktree => "DIRTY_WORKTREE",
            Self::ActiveProcess => "ACTIVE_PROCESS",
            Self::TerminalUnavailable => "TERMINAL_UNAVAILABLE",
            Self::PortInUse => "PORT_IN_USE",
            Self::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            Self::LocalModelUnavailable => "LOCAL_MODEL_UNAVAILABLE",
            Self::LocalQueueFull => "LOCAL_QUEUE_FULL",
            Self::LocalLoadTimeout => "LOCAL_LOAD_TIMEOUT",
            Self::LocalResourceExhausted => "LOCAL_RESOURCE_EXHAUSTED",
            Self::SuccessorExists => "SUCCESSOR_EXISTS",
            Self::ClientUploadMismatch => "CLIENT_UPLOAD_MISMATCH",
            Self::ControlReserveIncoherent => "CONTROL_RESERVE_INCOHERENT",
            Self::AutomationInputInvalid => "AUTOMATION_INPUT_INVALID",
            Self::UnknownValidator => "UNKNOWN_VALIDATOR",
            Self::InvalidRuleParameters => "INVALID_RULE_PARAMETERS",
            Self::UnknownClaimPredicate => "UNKNOWN_CLAIM_PREDICATE",
            Self::SchemaLimitExceeded => "SCHEMA_LIMIT_EXCEEDED",
            Self::AmbiguousReference => "AMBIGUOUS_REFERENCE",
            Self::SourceNotCommitted => "SOURCE_NOT_COMMITTED",
            Self::InvalidModelConfig => "INVALID_MODEL_CONFIG",
            Self::EffectsClassUnknown => "EFFECTS_CLASS_UNKNOWN",
            Self::StorageFull => "STORAGE_FULL",
            Self::StorageUnavailable => "STORAGE_UNAVAILABLE",
            Self::StorageCorrupt => "STORAGE_CORRUPT",
            Self::StorageBuildUnqualified => "STORAGE_BUILD_UNQUALIFIED",
            Self::InvariantViolation => "INVARIANT_VIOLATION",
            Self::JournalUnavailable => "JOURNAL_UNAVAILABLE",
            Self::SecretStoreUnavailable => "SECRET_STORE_UNAVAILABLE",
            Self::EvidenceMissing => "EVIDENCE_MISSING",
            Self::MissingDependency => "MISSING_DEPENDENCY",
            Self::BundleUnavailable => "BUNDLE_UNAVAILABLE",
            Self::SourceMissing => "SOURCE_MISSING",
            Self::ResourceMissingOrHidden => "RESOURCE_MISSING_OR_HIDDEN",
            Self::HistoryUnavailable => "HISTORY_UNAVAILABLE",
            Self::SecretRefNotFound => "SECRET_REF_NOT_FOUND",
            Self::AutomationProfileUnavailable => "AUTOMATION_PROFILE_UNAVAILABLE",
            Self::IsolationBackendMissing => "ISOLATION_BACKEND_MISSING",
            Self::IsolationObligationUnenforceable => "ISOLATION_OBLIGATION_UNENFORCEABLE",
            Self::GuardProtocolUnsupported => "GUARD_PROTOCOL_UNSUPPORTED",
            Self::CursorExpired => "CURSOR_EXPIRED",
        }
    }

    /// Renvoie la liste exhaustive des 156 raisons d'erreur canoniques enregistrées.
    #[expect(
        clippy::too_many_lines,
        reason = "Catalogue exhaustif des 156 ReasonCode canoniques enregistrés"
    )]
    #[must_use]
    pub const fn all_reasons() -> &'static [Self] {
        &[
            Self::PublicationConflict,
            Self::ContractVersionConflict,
            Self::TargetMoved,
            Self::SourceChanged,
            Self::TargetAlreadyExists,
            Self::StaleEpoch,
            Self::OwnerEpochChanged,
            Self::StaleAuthorityGeneration,
            Self::BindingAlreadyEngaged,
            Self::StaleEvidence,
            Self::DoneNotMet,
            Self::LoopExhausted,
            Self::ContractRejected,
            Self::TerminalImmutable,
            Self::IdempotencyWindowExpired,
            Self::AppliedDraftConflict,
            Self::AppliedDraftImmutable,
            Self::ExpectedExecutionMissing,
            Self::EvidenceExecutionMismatch,
            Self::EvidenceProjectionInvalid,
            Self::EvidenceAmbiguous,
            Self::EvidenceSnapshotMismatch,
            Self::ClaimContradicted,
            Self::DiscoveryRequired,
            Self::DiscoveryEvidenceMissing,
            Self::RestoreIncomplete,
            Self::EvidenceUnavailable,
            Self::AutomationModePolicyMismatch,
            Self::AutomationBundleUnavailable,
            Self::AutomationTargetMissing,
            Self::AutomationTargetAmbiguous,
            Self::AutomationTargetDrift,
            Self::BusinessConstraintViolated,
            Self::BusinessPostconditionFailed,
            Self::AutomationAccountMismatch,
            Self::ExternalStateChanged,
            Self::ActionabilityTimeout,
            Self::AssertionTimeout,
            Self::IncompatibleRole,
            Self::ReceiptStale,
            Self::ReceiptValidationExpired,
            Self::DiscoveryHistoryUnknown,
            Self::DependencyCycle,
            Self::MergeConflict,
            Self::ValidationMissing,
            Self::ModelConfigurationConflict,
            Self::ContextLineageIncomplete,
            Self::DataPolicyConflict,
            Self::ProviderRefused,
            Self::OutputTruncated,
            Self::CapabilitySnapshotStale,
            Self::UnsupportedFormat,
            Self::UnresolvedEffect,
            Self::GitOutcomeUnknown,
            Self::LaunchOutcomeUnknown,
            Self::TerminalInputUnknown,
            Self::ModelCallOutcomeUnknown,
            Self::CapabilityUnavailable,
            Self::DecisionAuthorityRequired,
            Self::DefinitionRevoked,
            Self::ModelCallsForbidden,
            Self::OperationNotAuthorized,
            Self::IdentityUnverified,
            Self::PermissionExpansionDenied,
            Self::CapabilityMismatch,
            Self::UnauthorizedAction,
            Self::ProfileUnavailable,
            Self::PolicyRefused,
            Self::SymbolicTargetUnsupported,
            Self::GitVersionUnsupported,
            Self::ToolUnavailable,
            Self::DeclassificationAuthorityRequired,
            Self::RemoteContextReferenceForbidden,
            Self::DataLocalityViolation,
            Self::InferenceLocalityUnknown,
            Self::ConnectorAuthRequired,
            Self::ConnectorAccessRevoked,
            Self::ConnectorAccessUnverified,
            Self::ConnectorAppNotAuthorized,
            Self::AccountIdentityMismatch,
            Self::SecretStoreLocked,
            Self::SecretAccessDenied,
            Self::PolicyDenied,
            Self::GuardDenied,
            Self::GuardObligationUnsatisfied,
            Self::GuardApprovalRequired,
            Self::GuardApprovalUnsupported,
            Self::GuardUnavailable,
            Self::SubjectSuspended,
            Self::SubjectRevoked,
            Self::AccessDenied,
            Self::DiscoveryOriginMismatch,
            Self::AccessRevoked,
            Self::BudgetExhausted,
            Self::SupervisionBudgetExhausted,
            Self::ContextBudgetExceeded,
            Self::PricingStale,
            Self::PricingUnknown,
            Self::ProviderRateLimited,
            Self::PredecessorNotTerminal,
            Self::ObjectDeleting,
            Self::Backpressure,
            Self::StorageBusy,
            Self::StorageLowSpace,
            Self::RecoveryInProgress,
            Self::AuthorityAlreadyActive,
            Self::DaemonAlreadyRunning,
            Self::RestorationBlocked,
            Self::AccountingMismatch,
            Self::AutomationHumanStepRequired,
            Self::SnapshotInconsistent,
            Self::IndexStale,
            Self::SourceAdvanced,
            Self::TargetCheckedOut,
            Self::PathOccupied,
            Self::BranchInUse,
            Self::DirtyWorktree,
            Self::ActiveProcess,
            Self::TerminalUnavailable,
            Self::PortInUse,
            Self::ProviderUnavailable,
            Self::LocalModelUnavailable,
            Self::LocalQueueFull,
            Self::LocalLoadTimeout,
            Self::LocalResourceExhausted,
            Self::SuccessorExists,
            Self::ClientUploadMismatch,
            Self::ControlReserveIncoherent,
            Self::AutomationInputInvalid,
            Self::UnknownValidator,
            Self::InvalidRuleParameters,
            Self::UnknownClaimPredicate,
            Self::SchemaLimitExceeded,
            Self::AmbiguousReference,
            Self::SourceNotCommitted,
            Self::InvalidModelConfig,
            Self::EffectsClassUnknown,
            Self::StorageFull,
            Self::StorageUnavailable,
            Self::StorageCorrupt,
            Self::StorageBuildUnqualified,
            Self::InvariantViolation,
            Self::JournalUnavailable,
            Self::SecretStoreUnavailable,
            Self::EvidenceMissing,
            Self::MissingDependency,
            Self::BundleUnavailable,
            Self::SourceMissing,
            Self::ResourceMissingOrHidden,
            Self::HistoryUnavailable,
            Self::SecretRefNotFound,
            Self::AutomationProfileUnavailable,
            Self::IsolationBackendMissing,
            Self::IsolationObligationUnenforceable,
            Self::GuardProtocolUnsupported,
            Self::CursorExpired,
        ]
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erreur lors de la résolution d'une raison d'erreur non reconnue.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Raison d'erreur inconnue : '{0}'")]
pub struct UnknownReasonCodeError(pub String);

impl FromStr for ReasonCode {
    type Err = UnknownReasonCodeError;

    #[expect(
        clippy::too_many_lines,
        reason = "Correspondance exhaustive des 156 ReasonCode canoniques enregistrés"
    )]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PUBLICATION_CONFLICT" => Ok(Self::PublicationConflict),
            "CONTRACT_VERSION_CONFLICT" => Ok(Self::ContractVersionConflict),
            "TARGET_MOVED" => Ok(Self::TargetMoved),
            "SOURCE_CHANGED" => Ok(Self::SourceChanged),
            "TARGET_ALREADY_EXISTS" => Ok(Self::TargetAlreadyExists),
            "STALE_EPOCH" => Ok(Self::StaleEpoch),
            "OWNER_EPOCH_CHANGED" => Ok(Self::OwnerEpochChanged),
            "STALE_AUTHORITY_GENERATION" => Ok(Self::StaleAuthorityGeneration),
            "BINDING_ALREADY_ENGAGED" => Ok(Self::BindingAlreadyEngaged),
            "STALE_EVIDENCE" => Ok(Self::StaleEvidence),
            "DONE_NOT_MET" => Ok(Self::DoneNotMet),
            "LOOP_EXHAUSTED" => Ok(Self::LoopExhausted),
            "CONTRACT_REJECTED" => Ok(Self::ContractRejected),
            "TERMINAL_IMMUTABLE" => Ok(Self::TerminalImmutable),
            "IDEMPOTENCY_WINDOW_EXPIRED" => Ok(Self::IdempotencyWindowExpired),
            "APPLIED_DRAFT_CONFLICT" => Ok(Self::AppliedDraftConflict),
            "APPLIED_DRAFT_IMMUTABLE" => Ok(Self::AppliedDraftImmutable),
            "EXPECTED_EXECUTION_MISSING" => Ok(Self::ExpectedExecutionMissing),
            "EVIDENCE_EXECUTION_MISMATCH" => Ok(Self::EvidenceExecutionMismatch),
            "EVIDENCE_PROJECTION_INVALID" => Ok(Self::EvidenceProjectionInvalid),
            "EVIDENCE_AMBIGUOUS" => Ok(Self::EvidenceAmbiguous),
            "EVIDENCE_SNAPSHOT_MISMATCH" => Ok(Self::EvidenceSnapshotMismatch),
            "CLAIM_CONTRADICTED" => Ok(Self::ClaimContradicted),
            "DISCOVERY_REQUIRED" => Ok(Self::DiscoveryRequired),
            "DISCOVERY_EVIDENCE_MISSING" => Ok(Self::DiscoveryEvidenceMissing),
            "RESTORE_INCOMPLETE" => Ok(Self::RestoreIncomplete),
            "EVIDENCE_UNAVAILABLE" => Ok(Self::EvidenceUnavailable),
            "AUTOMATION_MODE_POLICY_MISMATCH" => Ok(Self::AutomationModePolicyMismatch),
            "AUTOMATION_BUNDLE_UNAVAILABLE" => Ok(Self::AutomationBundleUnavailable),
            "AUTOMATION_TARGET_MISSING" => Ok(Self::AutomationTargetMissing),
            "AUTOMATION_TARGET_AMBIGUOUS" => Ok(Self::AutomationTargetAmbiguous),
            "AUTOMATION_TARGET_DRIFT" => Ok(Self::AutomationTargetDrift),
            "BUSINESS_CONSTRAINT_VIOLATED" => Ok(Self::BusinessConstraintViolated),
            "BUSINESS_POSTCONDITION_FAILED" => Ok(Self::BusinessPostconditionFailed),
            "AUTOMATION_ACCOUNT_MISMATCH" => Ok(Self::AutomationAccountMismatch),
            "EXTERNAL_STATE_CHANGED" => Ok(Self::ExternalStateChanged),
            "ACTIONABILITY_TIMEOUT" => Ok(Self::ActionabilityTimeout),
            "ASSERTION_TIMEOUT" => Ok(Self::AssertionTimeout),
            "INCOMPATIBLE_ROLE" => Ok(Self::IncompatibleRole),
            "RECEIPT_STALE" => Ok(Self::ReceiptStale),
            "RECEIPT_VALIDATION_EXPIRED" => Ok(Self::ReceiptValidationExpired),
            "DISCOVERY_HISTORY_UNKNOWN" => Ok(Self::DiscoveryHistoryUnknown),
            "DEPENDENCY_CYCLE" => Ok(Self::DependencyCycle),
            "MERGE_CONFLICT" => Ok(Self::MergeConflict),
            "VALIDATION_MISSING" => Ok(Self::ValidationMissing),
            "MODEL_CONFIGURATION_CONFLICT" => Ok(Self::ModelConfigurationConflict),
            "CONTEXT_LINEAGE_INCOMPLETE" => Ok(Self::ContextLineageIncomplete),
            "DATA_POLICY_CONFLICT" => Ok(Self::DataPolicyConflict),
            "PROVIDER_REFUSED" => Ok(Self::ProviderRefused),
            "OUTPUT_TRUNCATED" => Ok(Self::OutputTruncated),
            "CAPABILITY_SNAPSHOT_STALE" => Ok(Self::CapabilitySnapshotStale),
            "UNSUPPORTED_FORMAT" => Ok(Self::UnsupportedFormat),
            "UNRESOLVED_EFFECT" => Ok(Self::UnresolvedEffect),
            "GIT_OUTCOME_UNKNOWN" => Ok(Self::GitOutcomeUnknown),
            "LAUNCH_OUTCOME_UNKNOWN" => Ok(Self::LaunchOutcomeUnknown),
            "TERMINAL_INPUT_UNKNOWN" => Ok(Self::TerminalInputUnknown),
            "MODEL_CALL_OUTCOME_UNKNOWN" => Ok(Self::ModelCallOutcomeUnknown),
            "CAPABILITY_UNAVAILABLE" => Ok(Self::CapabilityUnavailable),
            "DECISION_AUTHORITY_REQUIRED" => Ok(Self::DecisionAuthorityRequired),
            "DEFINITION_REVOKED" => Ok(Self::DefinitionRevoked),
            "MODEL_CALLS_FORBIDDEN" => Ok(Self::ModelCallsForbidden),
            "OPERATION_NOT_AUTHORIZED" => Ok(Self::OperationNotAuthorized),
            "IDENTITY_UNVERIFIED" => Ok(Self::IdentityUnverified),
            "PERMISSION_EXPANSION_DENIED" => Ok(Self::PermissionExpansionDenied),
            "CAPABILITY_MISMATCH" => Ok(Self::CapabilityMismatch),
            "UNAUTHORIZED_ACTION" => Ok(Self::UnauthorizedAction),
            "PROFILE_UNAVAILABLE" => Ok(Self::ProfileUnavailable),
            "POLICY_REFUSED" => Ok(Self::PolicyRefused),
            "SYMBOLIC_TARGET_UNSUPPORTED" => Ok(Self::SymbolicTargetUnsupported),
            "GIT_VERSION_UNSUPPORTED" => Ok(Self::GitVersionUnsupported),
            "TOOL_UNAVAILABLE" => Ok(Self::ToolUnavailable),
            "DECLASSIFICATION_AUTHORITY_REQUIRED" => Ok(Self::DeclassificationAuthorityRequired),
            "REMOTE_CONTEXT_REFERENCE_FORBIDDEN" => Ok(Self::RemoteContextReferenceForbidden),
            "DATA_LOCALITY_VIOLATION" => Ok(Self::DataLocalityViolation),
            "INFERENCE_LOCALITY_UNKNOWN" => Ok(Self::InferenceLocalityUnknown),
            "CONNECTOR_AUTH_REQUIRED" => Ok(Self::ConnectorAuthRequired),
            "CONNECTOR_ACCESS_REVOKED" => Ok(Self::ConnectorAccessRevoked),
            "CONNECTOR_ACCESS_UNVERIFIED" => Ok(Self::ConnectorAccessUnverified),
            "CONNECTOR_APP_NOT_AUTHORIZED" => Ok(Self::ConnectorAppNotAuthorized),
            "ACCOUNT_IDENTITY_MISMATCH" => Ok(Self::AccountIdentityMismatch),
            "SECRET_STORE_LOCKED" => Ok(Self::SecretStoreLocked),
            "SECRET_ACCESS_DENIED" => Ok(Self::SecretAccessDenied),
            "POLICY_DENIED" => Ok(Self::PolicyDenied),
            "GUARD_DENIED" => Ok(Self::GuardDenied),
            "GUARD_OBLIGATION_UNSATISFIED" => Ok(Self::GuardObligationUnsatisfied),
            "GUARD_APPROVAL_REQUIRED" => Ok(Self::GuardApprovalRequired),
            "GUARD_APPROVAL_UNSUPPORTED" => Ok(Self::GuardApprovalUnsupported),
            "GUARD_UNAVAILABLE" => Ok(Self::GuardUnavailable),
            "SUBJECT_SUSPENDED" => Ok(Self::SubjectSuspended),
            "SUBJECT_REVOKED" => Ok(Self::SubjectRevoked),
            "ACCESS_DENIED" => Ok(Self::AccessDenied),
            "DISCOVERY_ORIGIN_MISMATCH" => Ok(Self::DiscoveryOriginMismatch),
            "ACCESS_REVOKED" => Ok(Self::AccessRevoked),
            "BUDGET_EXHAUSTED" => Ok(Self::BudgetExhausted),
            "SUPERVISION_BUDGET_EXHAUSTED" => Ok(Self::SupervisionBudgetExhausted),
            "CONTEXT_BUDGET_EXCEEDED" => Ok(Self::ContextBudgetExceeded),
            "PRICING_STALE" => Ok(Self::PricingStale),
            "PRICING_UNKNOWN" => Ok(Self::PricingUnknown),
            "PROVIDER_RATE_LIMITED" => Ok(Self::ProviderRateLimited),
            "PREDECESSOR_NOT_TERMINAL" => Ok(Self::PredecessorNotTerminal),
            "OBJECT_DELETING" => Ok(Self::ObjectDeleting),
            "BACKPRESSURE" => Ok(Self::Backpressure),
            "STORAGE_BUSY" => Ok(Self::StorageBusy),
            "STORAGE_LOW_SPACE" => Ok(Self::StorageLowSpace),
            "RECOVERY_IN_PROGRESS" => Ok(Self::RecoveryInProgress),
            "AUTHORITY_ALREADY_ACTIVE" => Ok(Self::AuthorityAlreadyActive),
            "DAEMON_ALREADY_RUNNING" => Ok(Self::DaemonAlreadyRunning),
            "RESTORATION_BLOCKED" => Ok(Self::RestorationBlocked),
            "ACCOUNTING_MISMATCH" => Ok(Self::AccountingMismatch),
            "AUTOMATION_HUMAN_STEP_REQUIRED" => Ok(Self::AutomationHumanStepRequired),
            "SNAPSHOT_INCONSISTENT" => Ok(Self::SnapshotInconsistent),
            "INDEX_STALE" => Ok(Self::IndexStale),
            "SOURCE_ADVANCED" => Ok(Self::SourceAdvanced),
            "TARGET_CHECKED_OUT" => Ok(Self::TargetCheckedOut),
            "PATH_OCCUPIED" => Ok(Self::PathOccupied),
            "BRANCH_IN_USE" => Ok(Self::BranchInUse),
            "DIRTY_WORKTREE" => Ok(Self::DirtyWorktree),
            "ACTIVE_PROCESS" => Ok(Self::ActiveProcess),
            "TERMINAL_UNAVAILABLE" => Ok(Self::TerminalUnavailable),
            "PORT_IN_USE" => Ok(Self::PortInUse),
            "PROVIDER_UNAVAILABLE" => Ok(Self::ProviderUnavailable),
            "LOCAL_MODEL_UNAVAILABLE" => Ok(Self::LocalModelUnavailable),
            "LOCAL_QUEUE_FULL" => Ok(Self::LocalQueueFull),
            "LOCAL_LOAD_TIMEOUT" => Ok(Self::LocalLoadTimeout),
            "LOCAL_RESOURCE_EXHAUSTED" => Ok(Self::LocalResourceExhausted),
            "SUCCESSOR_EXISTS" => Ok(Self::SuccessorExists),
            "CLIENT_UPLOAD_MISMATCH" => Ok(Self::ClientUploadMismatch),
            "CONTROL_RESERVE_INCOHERENT" => Ok(Self::ControlReserveIncoherent),
            "AUTOMATION_INPUT_INVALID" => Ok(Self::AutomationInputInvalid),
            "UNKNOWN_VALIDATOR" => Ok(Self::UnknownValidator),
            "INVALID_RULE_PARAMETERS" => Ok(Self::InvalidRuleParameters),
            "UNKNOWN_CLAIM_PREDICATE" => Ok(Self::UnknownClaimPredicate),
            "SCHEMA_LIMIT_EXCEEDED" => Ok(Self::SchemaLimitExceeded),
            "AMBIGUOUS_REFERENCE" => Ok(Self::AmbiguousReference),
            "SOURCE_NOT_COMMITTED" => Ok(Self::SourceNotCommitted),
            "INVALID_MODEL_CONFIG" => Ok(Self::InvalidModelConfig),
            "EFFECTS_CLASS_UNKNOWN" => Ok(Self::EffectsClassUnknown),
            "STORAGE_FULL" => Ok(Self::StorageFull),
            "STORAGE_UNAVAILABLE" => Ok(Self::StorageUnavailable),
            "STORAGE_CORRUPT" => Ok(Self::StorageCorrupt),
            "STORAGE_BUILD_UNQUALIFIED" => Ok(Self::StorageBuildUnqualified),
            "INVARIANT_VIOLATION" => Ok(Self::InvariantViolation),
            "JOURNAL_UNAVAILABLE" => Ok(Self::JournalUnavailable),
            "SECRET_STORE_UNAVAILABLE" => Ok(Self::SecretStoreUnavailable),
            "EVIDENCE_MISSING" => Ok(Self::EvidenceMissing),
            "MISSING_DEPENDENCY" => Ok(Self::MissingDependency),
            "BUNDLE_UNAVAILABLE" => Ok(Self::BundleUnavailable),
            "SOURCE_MISSING" => Ok(Self::SourceMissing),
            "RESOURCE_MISSING_OR_HIDDEN" => Ok(Self::ResourceMissingOrHidden),
            "HISTORY_UNAVAILABLE" => Ok(Self::HistoryUnavailable),
            "SECRET_REF_NOT_FOUND" => Ok(Self::SecretRefNotFound),
            "AUTOMATION_PROFILE_UNAVAILABLE" => Ok(Self::AutomationProfileUnavailable),
            "ISOLATION_BACKEND_MISSING" => Ok(Self::IsolationBackendMissing),
            "ISOLATION_OBLIGATION_UNENFORCEABLE" => Ok(Self::IsolationObligationUnenforceable),
            "GUARD_PROTOCOL_UNSUPPORTED" => Ok(Self::GuardProtocolUnsupported),
            "CURSOR_EXPIRED" => Ok(Self::CursorExpired),
            _ => Err(UnknownReasonCodeError(s.to_owned())),
        }
    }
}

impl TryFrom<&str> for ReasonCode {
    type Error = UnknownReasonCodeError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// DTO normatif de réponse d'erreur IPC (`ipc-error.schema.json`).
///
/// Invariants :
/// - `code`, `reason_code`, `message`, `retry_class`, `details_ref`, `command_id` sont obligatoires.
/// - `reason_code`, `details_ref` et `command_id` sont toujours sérialisés même s'ils valent `null`.
/// - `current_revision` est omis de la sérialisation si `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IpcError {
    /// Classe commune d'erreur parmi les 17 canoniques.
    pub code: ErrorCode,
    /// Raison typée ou null. Clé toujours présente.
    pub reason_code: Option<ReasonCode>,
    /// Message d'erreur explicatif (borné à 4096 caractères).
    pub message: String,
    /// Stratégie de reprise autorisée pour le client.
    pub retry_class: RetryClass,
    /// Référence opaque vers les détails sensibles protégés ou null. Clé toujours présente.
    pub details_ref: Option<String>,
    /// Identifiant de commande corrélé ou null. Clé toujours présente.
    #[ts(type = "string | null")]
    pub command_id: Option<CommandId>,
    /// Révision courante de l'agrégat (présente uniquement lors de conflits de révision).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_revision"
    )]
    #[ts(type = "number")]
    pub current_revision: Option<Revision>,
}

fn deserialize_present_revision<'de, D>(deserializer: D) -> Result<Option<Revision>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Revision::deserialize(deserializer).map(Some)
}

impl IpcError {
    /// Crée une nouvelle erreur IPC avec les champs obligatoires.
    #[must_use]
    pub fn new(
        code: ErrorCode,
        reason_code: Option<ReasonCode>,
        message: impl Into<String>,
        retry_class: RetryClass,
    ) -> Self {
        Self {
            code,
            reason_code,
            message: message.into(),
            retry_class,
            details_ref: None,
            command_id: None,
            current_revision: None,
        }
    }

    /// Crée une erreur standard à partir d'un [`ReasonCode`] typé en dérivant
    /// automatiquement son [`ErrorCode`] et sa [`RetryClass`] par défaut.
    #[must_use]
    pub fn from_reason(reason: ReasonCode, message: impl Into<String>) -> Self {
        Self::new(
            reason.error_code(),
            Some(reason),
            message,
            reason.default_retry_class(),
        )
    }

    /// Associe un identifiant de commande à l'erreur.
    #[must_use]
    pub fn with_command_id(mut self, command_id: CommandId) -> Self {
        self.command_id = Some(command_id);
        self
    }

    /// Associe une référence de détails protégés à l'erreur.
    #[must_use]
    pub fn with_details_ref(mut self, details_ref: impl Into<String>) -> Self {
        self.details_ref = Some(details_ref.into());
        self
    }

    /// Associe la révision courante lors d'un conflit de révision.
    #[must_use]
    pub fn with_current_revision(mut self, revision: Revision) -> Self {
        self.current_revision = Some(revision);
        self
    }
}
