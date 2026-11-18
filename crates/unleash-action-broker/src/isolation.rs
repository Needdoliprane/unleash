// SPDX-License-Identifier: Apache-2.0
//! Modes et garanties d'isolation OS (`Q-05`, `security-boundaries.md`, `REQ-SEC-005`).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::ActionBrokerError;

/// Mode d'isolation d'un projet ou d'une exécution (`Q-05`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationMode {
    /// Mode par défaut : outils typés bornés mais exécutions de processus non confinées OS.
    #[default]
    Standard,
    /// Mode strict : exige un confinement OS réel (`sandbox-exec`/`bwrap`/proxy) et échoue fermé sinon.
    Strict,
}

impl IsolationMode {
    /// Chaîne wire canonique (`"standard"` ou `"strict"`).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Strict => "strict",
        }
    }

    /// Libellé exact exposé aux clients UI (`security-boundaries.md` : jamais « isolé » en `standard`).
    #[must_use]
    pub const fn confinement_label(&self) -> &'static str {
        match self {
            Self::Standard => "non confiné",
            Self::Strict => "confinement OS strict",
        }
    }
}

/// Capacité atomique requise auprès du backend d'isolation OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationCapability {
    /// Restriction des écritures aux seules racines assignées.
    FilesystemWriteScoping,
    /// Interdiction en lecture de `data_root`, socket et trousseaux.
    ForbiddenPathsReadDeny,
    /// Sortie réseau contrainte au proxy d'egress local.
    NetworkEgressProxyOnly,
    /// Environnement épuré sans secrets généraux du parent.
    CleanEnvironment,
    /// Héritage du confinement par tous les processus descendants.
    SubprocessInheritance,
}

/// Exigences d'isolation dérivées de l'autorité effective pour une action donnée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsolationRequirements {
    /// Mode d'isolation effectif (`Standard` ou `Strict`).
    pub mode: IsolationMode,
    /// Indique si le confinement OS du processus est strictement exigé avant lancement.
    pub require_process_confinement: bool,
    /// Ensemble des capacités de confinement nécessaires.
    pub required_capabilities: BTreeSet<IsolationCapability>,
}

impl IsolationRequirements {
    /// Construit des exigences standard non confinées (adaptées aux outils typés sans confinement OS strict).
    #[must_use]
    pub fn standard_unconfined() -> Self {
        Self {
            mode: IsolationMode::Standard,
            require_process_confinement: false,
            required_capabilities: BTreeSet::new(),
        }
    }

    /// Construit des exigences strictes avec l'ensemble complet des garanties OS.
    #[must_use]
    pub fn strict_default() -> Self {
        Self {
            mode: IsolationMode::Strict,
            require_process_confinement: true,
            required_capabilities: BTreeSet::from([
                IsolationCapability::FilesystemWriteScoping,
                IsolationCapability::ForbiddenPathsReadDeny,
                IsolationCapability::NetworkEgressProxyOnly,
                IsolationCapability::CleanEnvironment,
                IsolationCapability::SubprocessInheritance,
            ]),
        }
    }

    /// Dérive les exigences d'isolation directement depuis une [`crate::EffectiveAuthority`].
    #[must_use]
    pub fn from_authority(authority: &crate::authority::EffectiveAuthority) -> Self {
        if authority.isolation_mode == IsolationMode::Strict || authority.require_action_isolation {
            let mut req = Self::strict_default();
            req.mode = authority.isolation_mode;
            req.require_process_confinement = true;
            req
        } else {
            Self::standard_unconfined()
        }
    }
}

/// Résultat du contrôle d'un backend d'isolation OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsolationBackendStatus {
    /// Le backend est présent et supporte toutes les capacités exigées.
    Available {
        /// Nom du backend (`macos-seatbelt`, `linux-bubblewrap`, etc.).
        backend_name: String,
        /// Capacités effectivement supportées.
        supported_capabilities: BTreeSet<IsolationCapability>,
        /// Version ou ABI observée.
        abi_version: String,
    },
    /// Le backend d'isolation requis est absent sur cette machine.
    MissingBackend {
        /// Nom du backend attendu.
        backend_name: String,
        /// Remédiation affichée à l'utilisateur.
        remediation: String,
    },
    /// Le backend est présent mais incapable d'imposer certaines obligations requises.
    Unenforceable {
        /// Nom du backend concerné.
        backend_name: String,
        /// Capacités manquantes.
        missing_capabilities: BTreeSet<IsolationCapability>,
        /// Raison technique.
        reason: String,
        /// Remédiation explicite.
        remediation: String,
    },
}

/// Port d'inspection des capacités du backend d'isolation OS.
pub trait IsolationProbePort: Send + Sync {
    /// Sonde la disponibilité réelle du backend d'isolation pour `req`.
    fn probe_isolation(&self, req: &IsolationRequirements) -> IsolationBackendStatus;
}

/// Sonde d'isolation par défaut sur l'hôte OS local.
///
/// En mode `Standard` sans confinement exigé, rapporte l'état standard ; en mode `Strict`,
/// vérifie la présence d'un binaire potentiel (`/usr/bin/sandbox-exec` sur macOS ou `bwrap`
/// sur Linux) mais refuse le confinement tant que son application et son proxy ne sont pas
/// qualifiés par un adaptateur d'exécution dédié.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostIsolationProbe;

impl IsolationProbePort for HostIsolationProbe {
    fn probe_isolation(&self, req: &IsolationRequirements) -> IsolationBackendStatus {
        if !req.require_process_confinement && req.mode == IsolationMode::Standard {
            return IsolationBackendStatus::Available {
                backend_name: String::from("none_standard"),
                supported_capabilities: BTreeSet::new(),
                abi_version: String::from("standard-unconfined"),
            };
        }

        #[cfg(target_os = "macos")]
        {
            let path = std::path::Path::new("/usr/bin/sandbox-exec");
            if path.is_file() {
                IsolationBackendStatus::Unenforceable {
                    backend_name: String::from("macos-seatbelt"),
                    missing_capabilities: BTreeSet::from([
                        IsolationCapability::FilesystemWriteScoping,
                        IsolationCapability::ForbiddenPathsReadDeny,
                        IsolationCapability::NetworkEgressProxyOnly,
                        IsolationCapability::CleanEnvironment,
                        IsolationCapability::SubprocessInheritance,
                    ]),
                    reason: String::from(
                        "La présence de sandbox-exec ne prouve ni confinement appliqué ni proxy d'egress qualifié",
                    ),
                    remediation: String::from(
                        "Raccorder un backend qualifié à unleash-process-host avant une exécution stricte",
                    ),
                }
            } else {
                IsolationBackendStatus::MissingBackend {
                    backend_name: String::from("macos-seatbelt"),
                    remediation: String::from(
                        "Restaurer /usr/bin/sandbox-exec ou repasser explicitement le projet en standard",
                    ),
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            let path = std::path::Path::new("/usr/bin/bwrap");
            if path.is_file() {
                IsolationBackendStatus::Unenforceable {
                    backend_name: String::from("linux-bubblewrap"),
                    missing_capabilities: BTreeSet::from([
                        IsolationCapability::NetworkEgressProxyOnly,
                    ]),
                    reason: String::from(
                        "Montage du proxy d'egress et seccomp bwrap non qualifiés dans cet adaptateur",
                    ),
                    remediation: String::from(
                        "Qualifier le profil bwrap/seccomp ou repasser explicitement en mode standard",
                    ),
                }
            } else {
                IsolationBackendStatus::MissingBackend {
                    backend_name: String::from("linux-bubblewrap"),
                    remediation: String::from(
                        "Installer bubblewrap (bwrap) ou repasser explicitement le projet en standard",
                    ),
                }
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            IsolationBackendStatus::MissingBackend {
                backend_name: String::from("unsupported-os"),
                remediation: String::from(
                    "Confinement strict indisponible sur cet OS ; repasser explicitement en standard",
                ),
            }
        }
    }
}

/// Valide les exigences d'isolation auprès de `probe` sans jamais effectuer de repli vers `Standard`.
///
/// # Errors
/// Retourne [`ActionBrokerError::IsolationBackendMissing`] ou
/// [`ActionBrokerError::IsolationObligationUnenforceable`] si le confinement requis manque.
pub fn verify_isolation_requirements(
    req: &IsolationRequirements,
    probe: &dyn IsolationProbePort,
) -> Result<String, ActionBrokerError> {
    if req.mode == IsolationMode::Standard && !req.require_process_confinement {
        return Ok(String::from("none_standard"));
    }

    match probe.probe_isolation(req) {
        IsolationBackendStatus::Available {
            backend_name,
            supported_capabilities,
            ..
        } => {
            let missing: Vec<_> = req
                .required_capabilities
                .difference(&supported_capabilities)
                .collect();
            if missing.is_empty() {
                Ok(backend_name)
            } else {
                Err(ActionBrokerError::IsolationObligationUnenforceable {
                    backend: backend_name,
                    reason: format!("Capacités d'isolation manquantes : {missing:?}"),
                    remediation: String::from(
                        "Fournir un backend complet ou repasser explicitement le projet en mode standard",
                    ),
                })
            }
        }
        IsolationBackendStatus::MissingBackend {
            backend_name,
            remediation,
        } => Err(ActionBrokerError::IsolationBackendMissing {
            backend: backend_name,
            remediation,
        }),
        IsolationBackendStatus::Unenforceable {
            backend_name,
            reason,
            remediation,
            ..
        } => Err(ActionBrokerError::IsolationObligationUnenforceable {
            backend: backend_name,
            reason,
            remediation,
        }),
    }
}
