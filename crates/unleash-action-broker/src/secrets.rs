// SPDX-License-Identifier: Apache-2.0
//! Port de résolution sécurisée des secrets ([`SecretStore`]) sans exposition en `Debug`/logs/erreurs
//! (`REQ-SEC-003`, `security-boundaries.md`).

use core::fmt;
use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};
use unleash_domain::{ActionId, SecretRef};
use unleash_platform::KeyringPort;
pub use unleash_platform::SecretStatus as KeyringStatus;

use crate::error::SecretStoreError;
use crate::guard_port::GuardDecisionRef;

/// Secret résolu en mémoire protégée pour un transport autorisé uniquement.
///
/// - `Debug` masque toujours la valeur (`ResolvedSecret([REDACTED])`).
/// - Ne implémente volontairement ni `Display` ni `Serialize`.
/// - Libération avec zéroïsation automatique via [`secrecy::SecretString`].
#[derive(Clone)]
pub struct ResolvedSecret(SecretString);

impl ResolvedSecret {
    /// Enveloppe un secret protégé pour livraison au transport autorisé.
    #[must_use]
    pub const fn new(secret: SecretString) -> Self {
        Self(secret)
    }

    /// Expose temporairement la valeur brute au seul composant de transport autorisé.
    #[must_use]
    pub fn expose_for_transport(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for ResolvedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ResolvedSecret([REDACTED])")
    }
}

/// Jeton de preuve d'autorisation exigé pour résoudre un secret avant dispatch.
///
/// Ne peut être construit que dans la crate après validation complète des politiques,
/// obligations et revalidations de fence.
///
/// ```compile_fail
/// use unleash_action_broker::AuthorizedSecretAccess;
/// let forged = AuthorizedSecretAccess { action_id: todo!(), decision_ref: todo!() };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedSecretAccess {
    action_id: ActionId,
    decision_ref: GuardDecisionRef,
    allowed_refs: Vec<SecretRef>,
}

impl AuthorizedSecretAccess {
    pub(crate) fn new(
        action_id: ActionId,
        decision_ref: GuardDecisionRef,
        allowed_refs: Vec<SecretRef>,
    ) -> Self {
        Self {
            action_id,
            decision_ref,
            allowed_refs,
        }
    }

    /// Identité de l'action qui a obtenu cette autorisation.
    #[must_use]
    pub const fn action_id(&self) -> ActionId {
        self.action_id
    }

    /// Décision corrélée au marqueur de dispatch.
    #[must_use]
    pub fn decision_ref(&self) -> &GuardDecisionRef {
        &self.decision_ref
    }

    /// Vérifie que cette référence appartient exactement à l'intention autorisée.
    #[must_use]
    pub fn allows_reference(&self, reference: &SecretRef) -> bool {
        self.allowed_refs.contains(reference)
    }
}

/// Port abstrait d'accès au coffre de secrets pour l'`ActionBroker` (`REQ-SEC-003`).
pub trait SecretStore: Send + Sync {
    /// Résout le secret référencé exclusivement pour un transport déjà autorisé.
    ///
    /// # Errors
    /// Retourne [`SecretStoreError`] (`Locked`, `Unavailable`, `AccessDenied`, `NotFound`).
    fn resolve_for_authorized_transport(
        &self,
        reference: &SecretRef,
        grant: &AuthorizedSecretAccess,
    ) -> Result<ResolvedSecret, SecretStoreError>;

    /// Interroge la présence/verrouillage d'une référence sans jamais lire ni retourner de valeur.
    ///
    /// # Errors
    /// Retourne [`SecretStoreError::Unavailable`] ou [`SecretStoreError::AccessDenied`].
    fn status(&self, reference: &SecretRef) -> Result<KeyringStatus, SecretStoreError>;

    /// Enregistre ou met à jour un secret derrière sa référence opaque.
    ///
    /// # Errors
    /// Retourne [`SecretStoreError`].
    fn store(&self, reference: &SecretRef, secret: SecretString) -> Result<(), SecretStoreError>;

    /// Révoque/supprime un secret du coffre local.
    ///
    /// # Errors
    /// Retourne [`SecretStoreError`].
    fn revoke(&self, reference: &SecretRef) -> Result<(), SecretStoreError>;
}

/// Adaptateur reliant [`SecretStore`] à [`unleash_platform::KeyringPort`].
#[derive(Debug)]
pub struct PlatformKeyringSecretStore<K: KeyringPort + ?Sized> {
    keyring: Arc<K>,
}

impl<K: KeyringPort + ?Sized> Clone for PlatformKeyringSecretStore<K> {
    fn clone(&self) -> Self {
        Self {
            keyring: Arc::clone(&self.keyring),
        }
    }
}

impl<K: KeyringPort + ?Sized> PlatformKeyringSecretStore<K> {
    /// Crée un adaptateur [`SecretStore`] s'appuyant sur le coffre OS fourni.
    #[must_use]
    pub const fn new(keyring: Arc<K>) -> Self {
        Self { keyring }
    }
}

impl<K: KeyringPort + ?Sized> SecretStore for PlatformKeyringSecretStore<K> {
    fn resolve_for_authorized_transport(
        &self,
        reference: &SecretRef,
        grant: &AuthorizedSecretAccess,
    ) -> Result<ResolvedSecret, SecretStoreError> {
        if !grant.allows_reference(reference) {
            return Err(SecretStoreError::AccessDenied {
                reason: "Référence hors de l'intention autorisée".into(),
            });
        }
        let secret = self.keyring.load(reference)?;
        Ok(ResolvedSecret::new(secret))
    }

    fn status(&self, reference: &SecretRef) -> Result<KeyringStatus, SecretStoreError> {
        self.keyring
            .status(reference)
            .map_err(SecretStoreError::from)
    }

    fn store(&self, reference: &SecretRef, secret: SecretString) -> Result<(), SecretStoreError> {
        self.keyring
            .store(reference, secret)
            .map_err(SecretStoreError::from)
    }

    fn revoke(&self, reference: &SecretRef) -> Result<(), SecretStoreError> {
        self.keyring
            .delete(reference)
            .map_err(SecretStoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_secret_redacts_debug() {
        let sec = ResolvedSecret::new(SecretString::from("canary-hidden-xyz".to_owned()));
        let dbg = format!("{sec:?}");
        assert_eq!(dbg, "ResolvedSecret([REDACTED])");
        assert!(!dbg.contains("canary-hidden-xyz"));
    }
}
