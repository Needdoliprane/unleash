// SPDX-License-Identifier: Apache-2.0
//! Abstraction du coffre de secrets OS (`KeyringPort`) et adaptateur local sécurisé de test
//! utilisant `secrecy` et `zeroize` (`REQ-SEC-003`, `F017`, `security-boundaries.md`).

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use secrecy::SecretString;
use thiserror::Error;
use unleash_domain::SecretRef;
use zeroize::Zeroize;

/// Nature du backend de coffre de secrets actif.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyringBackendKind {
    /// Trousseau natif macOS (Keychain).
    MacosKeychain,
    /// Service de secrets Linux (`Secret Service` / keyring).
    LinuxSecretService,
    /// Adaptateur local sécurisé de test (mémoire protégée `secrecy`/`zeroize`, sans écriture claire).
    LocalSecureTest,
    /// Aucun backend OS disponible ou configuré (refus fail-closed sans repli clair).
    Unavailable,
}

/// Statut observable d'une référence de secret sans en lire ni exposer la valeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SecretStatus {
    /// Un secret existe pour cette référence.
    Present,
    /// Aucun secret enregistré pour cette référence.
    Missing,
    /// Le coffre de secrets est actuellement verrouillé.
    Locked,
}

/// Erreur typée émise par [`KeyringPort`], alignée sur le catalogue F017 sans fuite de secret.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum KeyringError {
    /// Coffre de secrets verrouillé (`CapabilityDenied/SECRET_STORE_LOCKED`).
    #[error("Coffre de secrets verrouillé (SECRET_STORE_LOCKED)")]
    Locked,

    /// Coffre de secrets indisponible (`InternalFailure/SECRET_STORE_UNAVAILABLE`).
    #[error("Coffre de secrets indisponible : {reason}")]
    Unavailable {
        /// Motif d'indisponibilité (sans valeur secrète).
        reason: String,
    },

    /// Accès au secret refusé (`CapabilityDenied/SECRET_ACCESS_DENIED`).
    #[error("Accès au coffre de secrets refusé : {reason}")]
    AccessDenied {
        /// Motif du refus (sans valeur secrète).
        reason: String,
    },

    /// Référence de secret introuvable (`NotFoundOrHidden/SECRET_REF_NOT_FOUND`).
    #[error("Référence de secret introuvable (SECRET_REF_NOT_FOUND)")]
    NotFound,

    /// Référence de secret malformée.
    #[error("Référence de secret invalide : {0}")]
    InvalidReference(String),

    /// Erreur d'I/O sous-jacente du backend.
    #[error("Erreur I/O du coffre de secrets : {0}")]
    Io(#[from] std::io::Error),
}

/// Port abstrait d'accès au coffre de secrets OS (`F017`).
///
/// L'implémentation ne doit jamais écrire de valeur secrète en clair dans les logs, erreurs ou
/// fichiers ordinaires (`security-boundaries.md` : « aucun fallback clair »).
pub trait KeyringPort: Send + Sync {
    /// Indique le backend implémenté par cet adaptateur.
    fn backend_kind(&self) -> KeyringBackendKind;

    /// Enregistre ou remplace le secret associé à `reference`.
    ///
    /// # Errors
    /// Retourne [`KeyringError::Locked`], [`KeyringError::Unavailable`],
    /// [`KeyringError::AccessDenied`] ou [`KeyringError::InvalidReference`].
    fn store(&self, reference: &SecretRef, secret: SecretString) -> Result<(), KeyringError>;

    /// Résout le secret associé à `reference` à l'usage strictement autorisé du transport.
    ///
    /// # Errors
    /// Retourne [`KeyringError::NotFound`] si absent, ou [`KeyringError::Locked`] /
    /// [`KeyringError::AccessDenied`] / [`KeyringError::Unavailable`].
    fn load(&self, reference: &SecretRef) -> Result<SecretString, KeyringError>;

    /// Supprime le secret associé à `reference` en zéroïsant sa mémoire.
    ///
    /// # Errors
    /// Retourne [`KeyringError::NotFound`] si absent, ou [`KeyringError::Locked`] /
    /// [`KeyringError::AccessDenied`] / [`KeyringError::Unavailable`].
    fn delete(&self, reference: &SecretRef) -> Result<(), KeyringError>;

    /// Interroge la disponibilité d'une référence sans jamais lire ni retourner la valeur secrète.
    ///
    /// # Errors
    /// Retourne [`KeyringError::Unavailable`] ou [`KeyringError::AccessDenied`].
    fn status(&self, reference: &SecretRef) -> Result<SecretStatus, KeyringError>;
}

#[derive(Default)]
struct LocalSecureState {
    entries: BTreeMap<String, SecretString>,
    locked: bool,
    unavailable: bool,
    denied_refs: BTreeSet<String>,
}

impl Drop for LocalSecureState {
    fn drop(&mut self) {
        for (_, mut secret) in std::mem::take(&mut self.entries) {
            secret.zeroize();
        }
    }
}

/// Adaptateur local sécurisé de test pour [`KeyringPort`].
///
/// Conserve les secrets exclusivement en mémoire protégée via [`secrecy::SecretString`] et
/// [`zeroize::Zeroize`] (aucun repli en clair sur disque) et permet aux tests F017 de simuler
/// coffre verrouillé, indisponible ou accès refusé.
#[derive(Default)]
pub struct LocalSecureKeyring {
    state: Mutex<LocalSecureState>,
}

impl fmt::Debug for LocalSecureKeyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f.debug_struct("LocalSecureKeyring")
            .field("backend_kind", &KeyringBackendKind::LocalSecureTest)
            .field("entry_count", &guard.entries.len())
            .field("locked", &guard.locked)
            .field("unavailable", &guard.unavailable)
            .field("entries", &"[REDACTED]")
            .finish()
    }
}

impl LocalSecureKeyring {
    /// Crée un nouvel adaptateur local sécurisé vide et déverrouillé.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Simule le verrouillage du trousseau OS (`SECRET_STORE_LOCKED`).
    pub fn lock(&self) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.locked = true;
    }

    /// Simule le déverrouillage du trousseau OS.
    pub fn unlock(&self) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.locked = false;
    }

    /// Simule l'indisponibilité du service de secrets (`SECRET_STORE_UNAVAILABLE`).
    pub fn set_unavailable(&self, unavailable: bool) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.unavailable = unavailable;
    }

    /// Simule un refus ACL OS ciblé pour `reference` (`SECRET_ACCESS_DENIED`).
    pub fn deny_ref(&self, reference: &SecretRef) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .denied_refs
            .insert(reference.as_opaque_str().to_owned());
    }

    /// Lève le refus ACL simulé pour `reference`.
    pub fn allow_ref(&self, reference: &SecretRef) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.denied_refs.remove(reference.as_opaque_str());
    }

    /// Vide toutes les entrées en zéroïsant chaque secret.
    pub fn clear(&self) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (_, mut secret) in std::mem::take(&mut guard.entries) {
            secret.zeroize();
        }
    }
}

fn check_ref_syntax(reference: &SecretRef) -> Result<&str, KeyringError> {
    let key = reference.as_opaque_str();
    if key.trim().is_empty() {
        return Err(KeyringError::InvalidReference(String::from(
            "Référence vide",
        )));
    }
    Ok(key)
}

fn check_access(state: &LocalSecureState, key: &str) -> Result<(), KeyringError> {
    if state.unavailable {
        return Err(KeyringError::Unavailable {
            reason: String::from("Service de secrets local marqué indisponible"),
        });
    }
    if state.denied_refs.contains(key) {
        return Err(KeyringError::AccessDenied {
            reason: String::from("Accès refusé par la politique du coffre"),
        });
    }
    if state.locked {
        return Err(KeyringError::Locked);
    }
    Ok(())
}

impl KeyringPort for LocalSecureKeyring {
    fn backend_kind(&self) -> KeyringBackendKind {
        KeyringBackendKind::LocalSecureTest
    }

    fn store(&self, reference: &SecretRef, secret: SecretString) -> Result<(), KeyringError> {
        let key = check_ref_syntax(reference)?;
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        check_access(&guard, key)?;
        if let Some(mut previous) = guard.entries.insert(key.to_owned(), secret) {
            previous.zeroize();
        }
        Ok(())
    }

    fn load(&self, reference: &SecretRef) -> Result<SecretString, KeyringError> {
        let key = check_ref_syntax(reference)?;
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        check_access(&guard, key)?;
        guard
            .entries
            .get(key)
            .cloned()
            .ok_or(KeyringError::NotFound)
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), KeyringError> {
        let key = check_ref_syntax(reference)?;
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        check_access(&guard, key)?;
        let Some(mut removed) = guard.entries.remove(key) else {
            return Err(KeyringError::NotFound);
        };
        removed.zeroize();
        Ok(())
    }

    fn status(&self, reference: &SecretRef) -> Result<SecretStatus, KeyringError> {
        let key = check_ref_syntax(reference)?;
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.unavailable {
            return Err(KeyringError::Unavailable {
                reason: String::from("Service de secrets local marqué indisponible"),
            });
        }
        if guard.denied_refs.contains(key) {
            return Err(KeyringError::AccessDenied {
                reason: String::from("Accès refusé par la politique du coffre"),
            });
        }
        if guard.locked {
            return Ok(SecretStatus::Locked);
        }
        if guard.entries.contains_key(key) {
            Ok(SecretStatus::Present)
        } else {
            Ok(SecretStatus::Missing)
        }
    }
}

/// Adaptateur OS fail-closed utilisé lorsque aucun backend de trousseau OS signé (`EXT-DIST`)
/// n'est encore configuré ; n'effectue jamais de repli en clair sur disque.
#[derive(Debug, Clone)]
pub struct UnavailableOsKeyring {
    reason: String,
}

impl UnavailableOsKeyring {
    /// Construit un adaptateur fail-closed avec un motif diagnostic sans secret.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl KeyringPort for UnavailableOsKeyring {
    fn backend_kind(&self) -> KeyringBackendKind {
        KeyringBackendKind::Unavailable
    }

    fn store(&self, _reference: &SecretRef, mut secret: SecretString) -> Result<(), KeyringError> {
        secret.zeroize();
        Err(KeyringError::Unavailable {
            reason: self.reason.clone(),
        })
    }

    fn load(&self, _reference: &SecretRef) -> Result<SecretString, KeyringError> {
        Err(KeyringError::Unavailable {
            reason: self.reason.clone(),
        })
    }

    fn delete(&self, _reference: &SecretRef) -> Result<(), KeyringError> {
        Err(KeyringError::Unavailable {
            reason: self.reason.clone(),
        })
    }

    fn status(&self, _reference: &SecretRef) -> Result<SecretStatus, KeyringError> {
        Err(KeyringError::Unavailable {
            reason: self.reason.clone(),
        })
    }
}
