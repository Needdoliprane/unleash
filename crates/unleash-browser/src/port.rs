//! Port interne abstrait [`BrowserPort`] définissant la frontière d'exécution navigateur.
//!
//! Voir `.claude/architecture/browser-execution.md` §4.

use std::future::Future;
use std::pin::Pin;

use crate::error::BrowserError;
use crate::locator::{ResolvedTarget, TargetSpec};
use crate::runtime::{PinnedBrowserRuntime, RuntimeInspection};
use crate::types::{
    ActionOutcome, AssertionEvaluationReport, BrowserActionIntent, BrowserCheckpoint,
    BrowserContextHandle, BusinessAssertion, CheckpointInput, CloseContextResult, CloseReason,
    ObserveScope, OpenContextRequest, PageObservation,
};

/// Future typée boîte et `Send` utilisée par le port objet-safe [`BrowserPort`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Contrat interne abstrait implémenté par l'adaptateur `WebDriver` Rust (`fantoccini`).
pub trait BrowserPort: Send + Sync {
    /// Inspecte passivement un runtime navigateur épinglé sans lancer de processus ni télécharger.
    fn inspect_runtime<'a>(
        &'a self,
        runtime_ref: &'a PinnedBrowserRuntime,
    ) -> BoxFuture<'a, Result<RuntimeInspection, BrowserError>>;

    /// Ouvre un contexte navigateur isolé sous mandat, fence et politique d'egress.
    fn open_context(
        &self,
        request: OpenContextRequest,
    ) -> BoxFuture<'_, Result<BrowserContextHandle, BrowserError>>;

    /// Produit une observation horodatée et bornée de la page courante.
    fn observe<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        scope: ObserveScope,
    ) -> BoxFuture<'a, Result<PageObservation, BrowserError>>;

    /// Résout un locator déclaratif V1 avec contrôle strict de cardinalité et d'identité métier.
    fn resolve_target<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        target: &'a TargetSpec,
        observation: &'a PageObservation,
    ) -> BoxFuture<'a, Result<ResolvedTarget, BrowserError>>;

    /// Exécute une action V1 cataloguée après pré-dispatch durable et vérification d'autorité.
    fn perform_action<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        intent: &'a BrowserActionIntent,
        target: Option<&'a ResolvedTarget>,
    ) -> BoxFuture<'a, Result<ActionOutcome, BrowserError>>;

    /// Évalue déterministement les assertions métier publiées contre l'état courant observé.
    fn evaluate_assertions<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        assertions: &'a [BusinessAssertion],
        observation: &'a PageObservation,
    ) -> BoxFuture<'a, Result<AssertionEvaluationReport, BrowserError>>;

    /// Produit un checkpoint durable de l'état rapproché et des effets connus/inconnus.
    fn checkpoint<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        input: CheckpointInput,
    ) -> BoxFuture<'a, Result<BrowserCheckpoint, BrowserError>>;

    /// Ferme le contexte `WebDriver` et retourne le statut observé de fermeture.
    fn close_context(
        &self,
        handle: BrowserContextHandle,
        reason: CloseReason,
    ) -> BoxFuture<'_, Result<CloseContextResult, BrowserError>>;
}
