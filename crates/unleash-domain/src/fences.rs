//! Types et compteurs ordonnés de fences, générations et époques.
//!
//! Garantit qu'aucun ordre direct n'est établi entre des lignées d'autorité distinctes (`AuthorityGeneration`),
//! et que les compteurs d'époque (`DaemonEpoch`) et de fence (`Fence`) sont strictement monotones
//! à l'intérieur d'une même lignée (INV-007, failure-recovery.md).

use core::cmp::Ordering;
use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::FenceError;
use crate::ids::AuthorityGeneration;

/// Numéro de révision d'un agrégat (incrémenté à chaque modification réussie).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Revision(u64);

impl Revision {
    /// Révision initiale d'un agrégat nouvellement créé (0).
    pub const INITIAL: Self = Self(0);

    /// Construit une révision à partir d'une valeur entière.
    #[must_use]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Renvoie la valeur numérique de la révision.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Calcule la révision suivante (+1).
    #[must_use]
    pub const fn next(&self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Époque d'incarnation du daemon au sein d'une même lignée d'autorité.
///
/// Incrémentée à chaque redémarrage normal sans restauration (sans rollback de base).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct DaemonEpoch(u64);

impl DaemonEpoch {
    /// Époque initiale (0 ou 1 selon initialisation).
    pub const INITIAL: Self = Self(1);

    /// Construit une époque à partir d'une valeur entière.
    #[must_use]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Renvoie la valeur numérique de l'époque.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Calcule l'époque suivante.
    #[must_use]
    pub const fn next(&self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for DaemonEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Compteur de fence monotone protégeant une ressource, un bail ou une tentative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Fence(u64);

impl Fence {
    /// Fence initial (0).
    pub const ZERO: Self = Self(0);

    /// Construit un fence à partir d'une valeur entière.
    #[must_use]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Renvoie la valeur numérique du fence.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Calcule le fence suivant.
    #[must_use]
    pub const fn next(&self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Fence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Numéro de séquence d'un événement dans un flux (`stream_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Sequence(u64);

impl Sequence {
    /// Séquence initiale (1).
    pub const INITIAL: Self = Self(1);

    /// Construit un numéro de séquence.
    #[must_use]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Renvoie la valeur numérique de la séquence.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Calcule la séquence suivante.
    #[must_use]
    pub const fn next(&self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Sequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Horodatage en millisecondes UTC depuis UNIX epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct TimestampMs(i64);

impl TimestampMs {
    /// Construit un horodatage à partir de millisecondes entières.
    #[must_use]
    pub const fn new(ms: i64) -> Self {
        Self(ms)
    }

    /// Renvoie la valeur en millisecondes.
    #[must_use]
    pub const fn as_i64(&self) -> i64 {
        self.0
    }
}

impl fmt::Display for TimestampMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

/// Montant financier en micro-unités d'une devise explicite (entier non flottant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct MicrosAmount(u64);

impl MicrosAmount {
    /// Montant nul.
    pub const ZERO: Self = Self(0);

    /// Construit un montant en micro-unités.
    #[must_use]
    pub const fn new(micros: u64) -> Self {
        Self(micros)
    }

    /// Renvoie le montant brut en micro-unités.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Addition saturante de deux montants.
    #[must_use]
    pub const fn saturating_add(&self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Soustraction vérifiée.
    #[must_use]
    pub const fn checked_sub(&self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

impl fmt::Display for MicrosAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}µ", self.0)
    }
}

/// Jeton attestant de l'appartenance à une lignée d'autorité (`AuthorityGeneration`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineageToken {
    generation: AuthorityGeneration,
}

impl LineageToken {
    /// Émet un nouveau jeton de lignée pour une autorité donnée.
    #[must_use]
    pub const fn new(generation: AuthorityGeneration) -> Self {
        Self { generation }
    }

    /// Renvoie la génération d'autorité représentée.
    #[must_use]
    pub const fn generation(&self) -> AuthorityGeneration {
        self.generation
    }
}

/// Barrière d'autorité combinant lignée (`AuthorityGeneration`), époque (`DaemonEpoch`) et clôture (`Fence`).
///
/// **Invariant critique (INV-007, failure-recovery.md) :**
/// Deux `LineageFence` portant des `AuthorityGeneration` distinctes sont STRICTEMENT INCOMPARABLES
/// (leur comparaison `partial_cmp` retourne systématiquement `None`).
/// Aucun ordre total (`Ord`) n'est implémenté sur ce type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineageFence {
    /// Lignée d'autorité (UUID).
    pub authority_generation: AuthorityGeneration,
    /// Époque d'incarnation au sein de cette lignée.
    pub epoch: DaemonEpoch,
    /// Fence local au sein de cette époque et lignée.
    pub fence: Fence,
}

impl LineageFence {
    /// Construit une barrière d'autorité complète.
    #[must_use]
    pub const fn new(
        authority_generation: AuthorityGeneration,
        epoch: DaemonEpoch,
        fence: Fence,
    ) -> Self {
        Self {
            authority_generation,
            epoch,
            fence,
        }
    }

    /// Vérifie qu'un candidat est un successeur monotone valide dans la MÊME lignée d'autorité.
    ///
    /// # Errors
    /// - `FenceError::LineageMismatch` si les générations d'autorité diffèrent.
    /// - `FenceError::StaleFence` si l'époque ou le fence du candidat est inférieur ou égal selon le contrat.
    pub fn check_monotone_successor(&self, candidate: &Self) -> Result<(), FenceError> {
        if self.authority_generation != candidate.authority_generation {
            return Err(FenceError::LineageMismatch {
                expected: self.authority_generation.to_string(),
                actual: candidate.authority_generation.to_string(),
            });
        }

        // Dans la même lignée : soit epoch strictement supérieur, soit même epoch et fence >= self.fence
        if candidate.epoch > self.epoch
            || (candidate.epoch == self.epoch && candidate.fence >= self.fence)
        {
            Ok(())
        } else {
            Err(FenceError::StaleFence {
                expected_min: self.fence.as_u64(),
                actual: candidate.fence.as_u64(),
            })
        }
    }
}

impl PartialOrd for LineageFence {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Deux lignées distinctes ne sont JAMAIS comparables par ordre.
        if self.authority_generation == other.authority_generation {
            match self.epoch.cmp(&other.epoch) {
                Ordering::Equal => Some(self.fence.cmp(&other.fence)),
                ord => Some(ord),
            }
        } else {
            None
        }
    }
}
