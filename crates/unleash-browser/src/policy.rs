//! Politiques d'exécution (`mediated` vs `project_script`), niveau d'isolation et
//! garde anti-modèle sous `model_policy = forbidden`.
//!
//! Réalise `REQ-AUTO-006` et `REQ-AUTO-014` : empêche tout appel modèle (direct,
//! gestionnaire, embedding, réparation ou script généré) sous `ModelPolicy::Forbidden`,
//! et refuse toute attestation sans modèle d'un `project_script` non confiné avec
//! [`BrowserError::IsolationUnavailable`].

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use unleash_domain::{ModelPolicy, RunId, Sha256Digest};

use crate::error::BrowserError;

/// Mode d'exécution d'une procédure navigateur (`browser-execution.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Procédure fermée pilotée action par action via `BrowserPort` et `fantoccini`.
    Mediated,
    /// Script externe (ex. Playwright Test TypeScript) exécuté sous supervision `F011`.
    ProjectScript,
}

/// Origine déclarée d'un script navigateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptOrigin {
    /// Script interne figé et versionné appartenant exclusivement à l'adaptateur `unleash-browser`.
    AdapterVersioned,
    /// Script proposé ou généré par un modèle (strictement rejeté en V1).
    ModelSupplied,
}

/// Niveau d'isolation exigé pour le contexte navigateur (`Q-05` / `security-boundaries.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    /// Mode standard : contexte dédié `--user-data-dir` et proxy d'egress configuré au lancement.
    Standard,
    /// Mode strict : exige un confinement OS prouvé couvrant réseau, processus et descendants.
    Strict,
}

/// Preuve de confinement fournie par la couche OS (`unleash-platform` / `unleash-process-host`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfinementProof {
    /// Aucun backend de confinement OS vérifié n'est actif pour ce contexte.
    Unconfined,
    /// Confinement OS actif et vérifié (ex. Seatbelt sur macOS ou bubblewrap/seccomp sur Linux).
    VerifiedOsSandbox {
        /// Nom du backend d'isolation effectif.
        backend: String,
        /// Empreinte du profil de confinement appliqué.
        proof_digest: Sha256Digest,
    },
}

/// Nature d'une tentative d'invocation modèle interceptée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAttemptKind {
    /// Appel modèle de génération de texte ou de complétion.
    TextGeneration,
    /// Appel modèle multimodal / vision sur capture d'écran.
    Vision,
    /// Calcul d'embedding ou reclassement sémantique.
    Embedding,
    /// Réveil d'un manager ou rédaction de rapport par LLM.
    ManagerSummary,
    /// Tentative de guérison/réparation autonome de sélecteur par modèle.
    SelectorRepair,
    /// Requête de `sampling` émise par un composant externe.
    Sampling,
    /// Tentative d'injection d'`execute_script` fourni par un modèle.
    ModelSuppliedScript,
}

/// Journal auditable d'une tentative d'appel modèle refusée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeniedModelAttempt {
    /// Identifiant du run d'origine auquel la tentative est attribuée.
    pub origin_run_id: RunId,
    /// Nature de la tentative bloquée.
    pub kind: ModelAttemptKind,
    /// Détail diagnostique non sensible.
    pub detail: String,
}

/// Preuve instrumentée d'exécution sans appel modèle pour un run navigateur (`REQ-AUTO-006`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZeroModelProofReport {
    /// Identifiant du run couvert.
    pub run_id: RunId,
    /// Mode d'exécution utilisé (`mediated` ou `project_script`).
    pub execution_mode: ExecutionMode,
    /// Politique modèle effective.
    pub model_policy: ModelPolicy,
    /// Nombre total d'appels modèle autorisés et expédiés durant le run (doit être `0` sous `Forbidden`).
    pub model_dispatch_count: u64,
    /// Nombre de tentatives modèle explicitement refusées avant dispatch.
    pub model_denied_attempt_count: u64,
    /// Nombre de requêtes bloquées par le proxy d'egress.
    pub egress_blocked_count: u64,
    /// Indique si le mode exigeait un confinement strict vérifié.
    pub confinement_verified: bool,
    /// Détail des tentatives refusées capturées.
    pub denied_attempts: Vec<DeniedModelAttempt>,
}

/// Vérifie la compatibilité entre le mode d'exécution, la politique modèle et la preuve de confinement.
///
/// # Errors
/// - [`BrowserError::IsolationUnavailable`] si un `project_script` revendique l'exécution
///   sans modèle (`ModelPolicy::Forbidden`) ou si `IsolationLevel::Strict` est exigé sans
///   [`ConfinementProof::VerifiedOsSandbox`].
pub fn check_execution_isolation(
    mode: ExecutionMode,
    model_policy: ModelPolicy,
    isolation: IsolationLevel,
    proof: &ConfinementProof,
) -> Result<(), BrowserError> {
    let is_confined = matches!(proof, ConfinementProof::VerifiedOsSandbox { .. });
    if mode == ExecutionMode::ProjectScript
        && model_policy == ModelPolicy::Forbidden
        && !is_confined
    {
        return Err(BrowserError::IsolationUnavailable {
            reason: "Un script project_script ne peut pas recevoir l'attestation sans modèle (model_policy=forbidden) sans confinement OS/réseau/descendants vérifié".into(),
        });
    }
    if isolation == IsolationLevel::Strict && !is_confined {
        return Err(BrowserError::IsolationUnavailable {
            reason: "Le mode d'isolation strict exige un backend de sandbox OS vérifié".into(),
        });
    }
    Ok(())
}

/// Garde d'exécution contrôlant et comptabilisant toute tentative d'appel modèle (`REQ-AUTO-006`).
#[derive(Debug, Default)]
pub struct ModelPolicyEnforcer {
    dispatch_count: AtomicU64,
    denied_attempt_count: AtomicU64,
    denied_attempts: Mutex<Vec<DeniedModelAttempt>>,
}

impl ModelPolicyEnforcer {
    /// Crée un nouveau garde avec compteurs initialisés à zéro.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            dispatch_count: AtomicU64::new(0),
            denied_attempt_count: AtomicU64::new(0),
            denied_attempts: Mutex::new(Vec::new()),
        }
    }

    /// Vérifie si une invocation modèle est permise par `model_policy` avant tout dispatch.
    ///
    /// Sous [`ModelPolicy::Forbidden`], incrémente `model_denied_attempt_count`, journalise
    /// le refus et retourne [`BrowserError::ModelUseForbidden`].
    ///
    /// # Errors
    /// - [`BrowserError::ModelUseForbidden`] si `model_policy` vaut [`ModelPolicy::Forbidden`].
    pub fn guard_model_call(
        &self,
        run_id: RunId,
        model_policy: ModelPolicy,
        kind: ModelAttemptKind,
        detail: &str,
    ) -> Result<(), BrowserError> {
        if model_policy == ModelPolicy::Forbidden {
            self.denied_attempt_count.fetch_add(1, Ordering::SeqCst);
            tracing::warn!(
                %run_id,
                ?kind,
                detail,
                "Tentative d'appel modèle refusée sous model_policy=forbidden"
            );
            if let Ok(mut guard) = self.denied_attempts.lock() {
                guard.push(DeniedModelAttempt {
                    origin_run_id: run_id,
                    kind,
                    detail: detail.to_owned(),
                });
            }
            return Err(BrowserError::ModelUseForbidden {
                reason: format!("{kind:?}: {detail}"),
            });
        }
        Ok(())
    }

    /// Enregistre un dispatch modèle autorisé (interdit sous [`ModelPolicy::Forbidden`]).
    ///
    /// # Errors
    /// - [`BrowserError::ModelUseForbidden`] si `model_policy` vaut [`ModelPolicy::Forbidden`].
    pub fn record_authorized_dispatch(
        &self,
        run_id: RunId,
        model_policy: ModelPolicy,
        detail: &str,
    ) -> Result<(), BrowserError> {
        self.guard_model_call(
            run_id,
            model_policy,
            ModelAttemptKind::TextGeneration,
            detail,
        )?;
        self.dispatch_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Retourne le nombre d'appels modèle effectivement expédiés.
    #[must_use]
    pub fn dispatch_count(&self) -> u64 {
        self.dispatch_count.load(Ordering::SeqCst)
    }

    /// Retourne le nombre de tentatives modèle refusées.
    #[must_use]
    pub fn denied_attempt_count(&self) -> u64 {
        self.denied_attempt_count.load(Ordering::SeqCst)
    }

    /// Retourne la liste des tentatives refusées capturées.
    #[must_use]
    pub fn denied_attempts(&self) -> Vec<DeniedModelAttempt> {
        self.denied_attempts
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Construit le rapport de preuve d'absence d'appel modèle.
    #[must_use]
    pub fn build_zero_model_proof(
        &self,
        run_id: RunId,
        execution_mode: ExecutionMode,
        model_policy: ModelPolicy,
        confinement_proof: &ConfinementProof,
        egress_blocked_count: u64,
    ) -> ZeroModelProofReport {
        ZeroModelProofReport {
            run_id,
            execution_mode,
            model_policy,
            model_dispatch_count: self.dispatch_count(),
            model_denied_attempt_count: self.denied_attempt_count(),
            egress_blocked_count,
            confinement_verified: matches!(
                confinement_proof,
                ConfinementProof::VerifiedOsSandbox { .. }
            ),
            denied_attempts: self.denied_attempts(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_script_forbidden_without_confinement_is_rejected() {
        let res = check_execution_isolation(
            ExecutionMode::ProjectScript,
            ModelPolicy::Forbidden,
            IsolationLevel::Standard,
            &ConfinementProof::Unconfined,
        );
        assert!(matches!(
            res,
            Err(BrowserError::IsolationUnavailable { .. })
        ));
    }

    #[test]
    fn mediated_strict_without_confinement_is_rejected() {
        let res = check_execution_isolation(
            ExecutionMode::Mediated,
            ModelPolicy::Forbidden,
            IsolationLevel::Strict,
            &ConfinementProof::Unconfined,
        );
        assert!(matches!(
            res,
            Err(BrowserError::IsolationUnavailable { .. })
        ));
    }

    #[test]
    fn mediated_standard_forbidden_is_admitted_and_blocks_model_calls() {
        let res = check_execution_isolation(
            ExecutionMode::Mediated,
            ModelPolicy::Forbidden,
            IsolationLevel::Standard,
            &ConfinementProof::Unconfined,
        );
        assert!(res.is_ok());

        let enforcer = ModelPolicyEnforcer::new();
        let run_id = RunId::from_u128(42);
        let denied = enforcer.guard_model_call(
            run_id,
            ModelPolicy::Forbidden,
            ModelAttemptKind::Embedding,
            "local embed attempt",
        );
        assert!(matches!(
            denied,
            Err(BrowserError::ModelUseForbidden { .. })
        ));
        assert_eq!(enforcer.dispatch_count(), 0);
        assert_eq!(enforcer.denied_attempt_count(), 1);
    }
}
