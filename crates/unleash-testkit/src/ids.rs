//! Générateur déterministe d'identifiants implémentant [`unleash_domain::IdGeneratorPort`].
//!
//! Produit une séquence reproductible et sans collision d'UUID v4 à partir d'une graine `u64`
//! via un mélangeur `SplitMix64` bijectif. Imprime automatiquement la graine sur `stderr` lorsqu'un
//! test échoue (panic) afin de permettre la reproduction exacte.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;
use unleash_domain::IdGeneratorPort;
use uuid::{Builder, Uuid};

/// Nom de la variable d'environnement permettant d'imposer la graine lors d'un rejeu de test.
pub const TEST_SEED_ENV_VAR: &str = "UNLEASH_TEST_SEED";

/// Erreur de lecture ou d'analyse d'une graine depuis l'environnement.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdGeneratorError {
    /// La variable `UNLEASH_TEST_SEED` ne contient pas un entier `u64` décimal ou hexadécimal (`0x...`) valide.
    #[error("Graine '{raw}' invalide dans {TEST_SEED_ENV_VAR} : entier u64 attendu")]
    InvalidSeed {
        /// Valeur brute observée.
        raw: String,
    },
}

/// Garde RAII légère imprimant la graine de test sur `stderr` si le thread panique pendant sa portée.
#[derive(Debug)]
pub struct SeedDiagnosticGuard {
    seed: u64,
}

impl SeedDiagnosticGuard {
    /// Construit une garde de diagnostic pour la graine fournie.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Renvoie la graine surveillée par cette garde.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }
}

impl Drop for SeedDiagnosticGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            #[expect(
                clippy::print_stderr,
                reason = "Diagnostic testkit : affiche la graine reproductible lors d'un échec de test."
            )]
            {
                eprintln!(
                    "[unleash-testkit] test failure seed={} (0x{:016x})",
                    self.seed, self.seed
                );
            }
        }
    }
}

/// Générateur déterministe d'UUID et d'identifiants du domaine à partir d'une graine `u64`.
///
/// La séquence générée est strictement identique d'une exécution à l'autre pour une même graine
/// et sans collision sur l'espace complet du compteur `u64`.
pub struct DeterministicIdGenerator {
    seed: u64,
    counter: AtomicU64,
}

impl fmt::Debug for DeterministicIdGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeterministicIdGenerator")
            .field("seed", &self.seed)
            .field("seed_hex", &format_args!("0x{:016x}", self.seed))
            .field("generated_count", &self.generated_count())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for DeterministicIdGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DeterministicIdGenerator(seed={} / 0x{:016x}, count={})",
            self.seed,
            self.seed,
            self.generated_count()
        )
    }
}

impl DeterministicIdGenerator {
    /// Crée un générateur déterministe initialisé avec `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            seed,
            counter: AtomicU64::new(0),
        }
    }

    /// Construit un générateur en lisant `UNLEASH_TEST_SEED` (décimal ou préfixé `0x`),
    /// ou en utilisant `default_seed` si la variable est absente.
    ///
    /// # Errors
    /// - [`IdGeneratorError::InvalidSeed`] si `UNLEASH_TEST_SEED` est présente mais invalide.
    pub fn from_env(default_seed: u64) -> Result<Self, IdGeneratorError> {
        match std::env::var(TEST_SEED_ENV_VAR) {
            Ok(raw) => {
                let seed = parse_seed_str(&raw)?;
                Ok(Self::new(seed))
            }
            Err(std::env::VarError::NotPresent) => Ok(Self::new(default_seed)),
            Err(std::env::VarError::NotUnicode(_)) => Err(IdGeneratorError::InvalidSeed {
                raw: String::from("<non-utf8>"),
            }),
        }
    }

    /// Renvoie la graine `u64` d'initialisation.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Renvoie le nombre d'identifiants déjà émis depuis la création ou le dernier [`Self::reset`].
    #[must_use]
    pub fn generated_count(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }

    /// Réinitialise le compteur interne à zéro pour rejouer exactement la séquence depuis le début.
    pub fn reset(&self) {
        self.counter.store(0, Ordering::SeqCst);
    }

    /// Génère n'importe quel identifiant newtype du domaine implémentant `From<Uuid>`.
    #[must_use]
    pub fn next_id<T: From<Uuid>>(&self) -> T {
        T::from(self.new_uuid())
    }

    /// Crée une garde RAII qui affichera la graine sur `stderr` si le thread courant panique.
    #[must_use]
    pub const fn failure_guard(&self) -> SeedDiagnosticGuard {
        SeedDiagnosticGuard::new(self.seed)
    }
}

impl Drop for DeterministicIdGenerator {
    fn drop(&mut self) {
        if std::thread::panicking() {
            #[expect(
                clippy::print_stderr,
                reason = "Diagnostic testkit : affiche la graine reproductible lors d'un échec de test."
            )]
            {
                eprintln!(
                    "[unleash-testkit] DeterministicIdGenerator failure seed={} (0x{:016x}) generated_count={}",
                    self.seed,
                    self.seed,
                    self.counter.load(Ordering::Relaxed)
                );
            }
        }
    }
}

impl IdGeneratorPort for DeterministicIdGenerator {
    fn new_uuid(&self) -> Uuid {
        let index = self.counter.fetch_add(1, Ordering::SeqCst);
        let base = splitmix64(self.seed);
        let hi = splitmix64(base ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let lo = splitmix64(
            base.rotate_left(32)
                ^ index.wrapping_mul(0xBF58_476D_1CE4_E5B9)
                ^ 0x517C_C1B7_2722_0A95,
        );

        let mut bytes = [0u8; 16];
        let hi_bytes = hi.to_be_bytes();
        let lo_bytes = lo.to_be_bytes();
        bytes[0..8].copy_from_slice(&hi_bytes);
        bytes[8..16].copy_from_slice(&lo_bytes);
        Builder::from_random_bytes(bytes).into_uuid()
    }
}

const fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn parse_seed_str(raw: &str) -> Result<u64, IdGeneratorError> {
    let trimmed = raw.trim();
    let parsed = if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else {
        trimmed.parse::<u64>()
    };
    parsed.map_err(|_| IdGeneratorError::InvalidSeed {
        raw: raw.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn test_deterministic_ids_reproducible_and_unique() {
        let gen_a = DeterministicIdGenerator::new(0xDEAD_BEEF_CAFE_BABE);
        let gen_b = DeterministicIdGenerator::new(0xDEAD_BEEF_CAFE_BABE);

        let mut seen = HashSet::new();
        for _ in 0..1_000 {
            let ua = gen_a.new_uuid();
            let ub = gen_b.new_uuid();
            assert_eq!(ua, ub);
            assert!(seen.insert(ua));
        }

        let gen_other = DeterministicIdGenerator::new(0xDEAD_BEEF_CAFE_BABF);
        assert_ne!(
            gen_other.new_uuid(),
            DeterministicIdGenerator::new(0xDEAD_BEEF_CAFE_BABE).new_uuid()
        );
    }

    #[test]
    fn test_parse_seed_formats() {
        assert_eq!(parse_seed_str("42").unwrap(), 42);
        assert_eq!(parse_seed_str("0x2a").unwrap(), 42);
        assert_eq!(parse_seed_str("0XFF").unwrap(), 255);
        assert!(parse_seed_str("not-a-seed").is_err());
    }
}
