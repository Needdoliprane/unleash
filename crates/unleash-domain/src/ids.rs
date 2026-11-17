//! Identifiants newtypes fortement typés pour le domaine Unleash.
//!
//! Tous les identifiants sont encapsulés dans des newtypes spécifiques pour interdire
//! tout échange accidentel entre entités distinctes. Les formats sont validés à la construction.

use core::fmt;
use core::str::FromStr;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::errors::IdValidationError;

/// Macro pour déclarer un newtype UUID classique supportant l'ordre total.
macro_rules! define_uuid_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[repr(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Construit l'identifiant à partir d'un UUID brut existant.
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Construit l'identifiant de manière déterministe à partir d'un entier `u128`.
            #[must_use]
            pub const fn from_u128(v: u128) -> Self {
                Self(Uuid::from_u128(v))
            }

            /// Renvoie l'UUID sous-jacent.
            #[must_use]
            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<Uuid> for $name {
            fn from(u: Uuid) -> Self {
                Self(u)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl FromStr for $name {
            type Err = IdValidationError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s)
                    .map(Self)
                    .map_err(|e| IdValidationError::InvalidUuid(e.to_string()))
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdValidationError;

            fn try_from(s: &str) -> Result<Self, Self::Error> {
                s.parse()
            }
        }
    };
}

define_uuid_id!(
    CommandId,
    "Identifiant unique et stable d'une commande client (clé d'idempotence avec `actor_id`)."
);
define_uuid_id!(
    RequestId,
    "Identifiant de transport éphémère d'une requête client-serveur."
);
define_uuid_id!(
    ReceiptId,
    "Identifiant global unique du reçu durable d'une commande (causation ID)."
);
define_uuid_id!(
    DaemonInstanceId,
    "Identifiant d'une incarnation/instance active du daemon."
);
define_uuid_id!(
    StreamId,
    "Identifiant de la séquence d'événements durables d'une autorité."
);
define_uuid_id!(ProjectId, "Identifiant d'un projet supervisé.");
define_uuid_id!(GoalId, "Identifiant d'un objectif de haut niveau.");
define_uuid_id!(MissionId, "Identifiant d'une mission métier.");
define_uuid_id!(RunId, "Identifiant d'une exécution de workflow.");
define_uuid_id!(
    IterationId,
    "Identifiant d'une itération de boucle interne de workflow."
);
define_uuid_id!(
    StepId,
    "Identifiant d'une instance ou activation d'étape de workflow."
);
define_uuid_id!(TaskId, "Identifiant d'une tâche attribuée à un agent.");
define_uuid_id!(AgentId, "Identifiant d'un agent logique durable.");
define_uuid_id!(SessionId, "Identifiant d'une session durable d'agent.");
define_uuid_id!(
    InvocationId,
    "Identifiant d'une invocation bornée d'un agent."
);
define_uuid_id!(
    AttemptId,
    "Identifiant d'une tentative physique d'exécution d'une invocation."
);
define_uuid_id!(
    ActionId,
    "Identifiant d'une action durable engagée auprès du broker."
);
define_uuid_id!(
    DispatchId,
    "Identifiant d'un dispatch individuel d'une action."
);
define_uuid_id!(
    ModelCallId,
    "Identifiant d'un appel logique à un modèle LLM."
);
define_uuid_id!(
    ManifestId,
    "Identifiant d'un manifeste immuable (contexte ou résultat modèle)."
);
define_uuid_id!(ProcessId, "Identifiant logique d'un processus supervisé.");
define_uuid_id!(
    ProcessRunUuid,
    "UUID unique d'une exécution physique de processus."
);
define_uuid_id!(HostId, "Identifiant d'un `process-host` hôte.");
define_uuid_id!(TerminalId, "Identifiant d'une session de terminal PTY.");
define_uuid_id!(WorktreeId, "Identifiant d'un worktree alloué.");
define_uuid_id!(RepoId, "Identifiant d'un dépôt Git local.");
define_uuid_id!(LeaseId, "Identifiant d'une lease de ressource.");
define_uuid_id!(
    ReservationId,
    "Identifiant d'une réservation de capacité ou de budget."
);
define_uuid_id!(PoolId, "Identifiant d'un pool de capacité.");
define_uuid_id!(AccountId, "Identifiant d'un compte de budget.");
define_uuid_id!(
    ReplayWorkerId,
    "Identifiant d'un worker de replay sans modèle."
);
define_uuid_id!(
    ProviderProbeId,
    "Identifiant d'une sonde de validation fournisseur."
);
define_uuid_id!(
    DraftId,
    "Identifiant d'un brouillon de configuration ou de procédure."
);
define_uuid_id!(
    RegistryVersionId,
    "Identifiant de version dans un registre immuable."
);
define_uuid_id!(
    DiscoveryReceiptId,
    "Identifiant d'un reçu de découverte ou de qualification."
);
define_uuid_id!(EvidenceId, "Identifiant d'une pièce de preuve immuable.");
define_uuid_id!(BackupId, "Identifiant d'un travail de sauvegarde `SQLite`.");
define_uuid_id!(
    RestoreId,
    "Identifiant d'une demande de restauration contrôlée."
);
define_uuid_id!(
    OperationId,
    "Identifiant d'une opération asynchrone durable."
);
define_uuid_id!(
    ApprovalScopeId,
    "Identifiant d'un périmètre d'approbation humaine."
);

/// Identifiant d'une lignée ou génération d'autorité (`AuthorityGeneration`).
///
/// Contrairement aux autres UUID, `AuthorityGeneration` n'implémente volontairement
/// NI `PartialOrd` NI `Ord` afin de garantir qu'aucune comparaison d'ordre direct
/// entre deux lignées d'autorité ne soit possible au niveau du typage (INV-007, Q-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct AuthorityGeneration(Uuid);

impl AuthorityGeneration {
    /// Construit la génération à partir d'un UUID existant.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Construit la génération de manière déterministe à partir d'un entier `u128`.
    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Self(Uuid::from_u128(v))
    }

    /// Renvoie l'UUID sous-jacent.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for AuthorityGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for AuthorityGeneration {
    fn from(u: Uuid) -> Self {
        Self(u)
    }
}

impl From<AuthorityGeneration> for Uuid {
    fn from(id: AuthorityGeneration) -> Self {
        id.0
    }
}

impl FromStr for AuthorityGeneration {
    type Err = IdValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|e| IdValidationError::InvalidUuid(e.to_string()))
    }
}

impl TryFrom<&str> for AuthorityGeneration {
    type Error = IdValidationError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// Identifiant authentifié de l'acteur émettant une commande (`ActorId`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct ActorId(String);

impl ActorId {
    /// Crée un nouvel `ActorId` après validation de non-vacuité.
    ///
    /// # Errors
    /// Retourne `IdValidationError::EmptyIdentifier` si la chaîne est vide ou uniquement composée d'espaces.
    pub fn new(s: impl Into<String>) -> Result<Self, IdValidationError> {
        let val = s.into();
        if val.trim().is_empty() {
            return Err(IdValidationError::EmptyIdentifier("ActorId"));
        }
        Ok(Self(val))
    }

    /// Renvoie la référence vers la chaîne sous-jacente.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ActorId {
    type Err = IdValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Référence opaque vers un secret géré par le daemon ou le trousseau de clés OS.
///
/// La représentation en mémoire est protégée et ne révèle jamais la référence ni aucun contenu
/// dans les implémentations `Debug` et `Display` (qui affichent toujours `SecretRef([REDACTED])`).
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct SecretRef(String);

impl SecretRef {
    /// Construit une nouvelle référence opaque vers un secret.
    ///
    /// # Errors
    /// Retourne `IdValidationError::EmptyIdentifier` si la référence est vide.
    pub fn new(reference: impl Into<String>) -> Result<Self, IdValidationError> {
        let val = reference.into();
        if val.trim().is_empty() {
            return Err(IdValidationError::EmptyIdentifier("SecretRef"));
        }
        Ok(Self(val))
    }

    /// Accès restreint à la référence opaque sous-jacente pour l'adaptateur de secrets.
    #[must_use]
    pub fn as_opaque_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretRef([REDACTED])")
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretRef([REDACTED])")
    }
}

/// Empreinte cryptographique normalisée SHA-256 (`Sha256Digest`).
///
/// Valide strictement que la chaîne hexadécimale est composée d'exactement 64 caractères hexadécimaux
/// minuscules (`0-9`, `a-f`). Normalise les représentations `<hex64>`, `sha256:<hex64>` et
/// `{ "algorithm": "sha256", "hex": "<hex64>" }` vers la forme canonique `sha256:<hex64>`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Calcule le digest SHA-256 à partir d'un flux d'octets brut.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let result = hasher.finalize();
        Self(result.into())
    }

    /// Construit un digest à partir des 32 octets bruts.
    #[must_use]
    pub const fn from_raw_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Renvoie les 32 octets bruts du digest.
    #[must_use]
    pub const fn as_raw_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Valide une chaîne de 64 caractères hex minuscules et construit le digest.
    ///
    /// # Errors
    /// Retourne `IdValidationError::InvalidSha256` si la chaîne ne respecte pas le format strict.
    pub fn parse_hex(hex_str: &str) -> Result<Self, IdValidationError> {
        if hex_str.len() != 64 {
            return Err(IdValidationError::InvalidSha256(format!(
                "Longueur incorrecte: {} caractères (64 attendus)",
                hex_str.len()
            )));
        }

        // Vérification stricte minuscules et hex
        for b in hex_str.bytes() {
            if !matches!(b, b'0'..=b'9' | b'a'..=b'f') {
                return Err(IdValidationError::InvalidSha256(format!(
                    "Caractère non autorisé '{char}' (seuls 0-9 et a-f minuscules sont admis)",
                    char = b as char
                )));
            }
        }

        let mut bytes = [0u8; 32];
        hex::decode_to_slice(hex_str, &mut bytes)
            .map_err(|e| IdValidationError::InvalidSha256(format!("Échec de décodage hex: {e}")))?;

        Ok(Self(bytes))
    }

    /// Renvoie la chaîne hexadécimale de 64 caractères minuscules (sans préfixe).
    #[must_use]
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Renvoie la représentation canonique avec préfixe `sha256:<hex64>`.
    #[must_use]
    pub fn as_prefixed_str(&self) -> String {
        format!("sha256:{}", self.hex())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Digest(\"{}\")", self.as_prefixed_str())
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", hex::encode(self.0))
    }
}

impl FromStr for Sha256Digest {
    type Err = IdValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(stripped) = s.strip_prefix("sha256:") {
            Self::parse_hex(stripped)
        } else {
            Self::parse_hex(s)
        }
    }
}

impl TryFrom<&str> for Sha256Digest {
    type Error = IdValidationError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_prefixed_str())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Sha256Visitor;

        impl<'de> Visitor<'de> for Sha256Visitor {
            type Value = Sha256Digest;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(
                    "une chaîne '<hex64>', 'sha256:<hex64>' ou un objet '{ \"algorithm\": \"sha256\", \"hex\": \"<hex64>\" }'",
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                value.parse().map_err(de::Error::custom)
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut algorithm: Option<String> = None;
                let mut hex_val: Option<String> = None;

                while let Some(key) = access.next_key::<String>()? {
                    match key.as_str() {
                        "algorithm" => {
                            if algorithm.is_some() {
                                return Err(de::Error::duplicate_field("algorithm"));
                            }
                            algorithm = Some(access.next_value()?);
                        }
                        "hex" => {
                            if hex_val.is_some() {
                                return Err(de::Error::duplicate_field("hex"));
                            }
                            hex_val = Some(access.next_value()?);
                        }
                        _ => {
                            let _: de::IgnoredAny = access.next_value()?;
                        }
                    }
                }

                let algorithm = algorithm.ok_or_else(|| de::Error::missing_field("algorithm"))?;
                let hex_val = hex_val.ok_or_else(|| de::Error::missing_field("hex"))?;

                if algorithm != "sha256" {
                    return Err(de::Error::custom(format!(
                        "Algorithme inconnu '{algorithm}' (seul 'sha256' est supporté)"
                    )));
                }

                Sha256Digest::parse_hex(&hex_val).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_any(Sha256Visitor)
    }
}
