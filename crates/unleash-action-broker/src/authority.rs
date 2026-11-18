// SPDX-License-Identifier: Apache-2.0
//! Calcul d'autorité effective par intersection stricte, interdiction d'auto-élévation,
//! validation anti-traversée/symlinks/TOCTOU des chemins et blocage des sources externes
//! (`REQ-SEC-001`, `REQ-SEC-005`, `REQ-SEC-006`).

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use unleash_domain::{DataLocality, ModelPolicy, ProjectId, Revision, TimestampMs};
use unleash_platform::assert_not_under_claude;

use crate::effects::{
    ActionIntent, ActionProvenance, EffectsClass, NormalizedResourceRef, TrustLevel,
};
use crate::error::ActionBrokerError;
use crate::isolation::{IsolationMode, IsolationRequirements};

/// Portée hiérarchique maximale d'un rôle ou mandat (`repository < project < global`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    /// Portée limitée à un seul dépôt/worktree.
    Repository,
    /// Portée limitée à un projet.
    Project,
    /// Portée globale utilisateur.
    Global,
}

/// Couche individuelle de politique ou mandat entrant dans l'intersection d'autorité.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityLayer {
    /// Nom identifiant cette couche (ex. `"project_policy"`, `"composed_role_v1"`).
    pub layer_name: String,
    /// Effets autorisés par cette couche (`None` = n'ajoute pas de restriction d'allowlist).
    pub allowed_effects: Option<BTreeSet<EffectsClass>>,
    /// Effets explicitement interdits par cette couche (l'interdiction domine toujours).
    pub denied_effects: BTreeSet<EffectsClass>,
    /// Verbes autorisés par cette couche (`None` = pas de restriction explicite).
    pub allowed_verbs: Option<BTreeSet<String>>,
    /// Verbes explicitement interdits par cette couche.
    pub denied_verbs: BTreeSet<String>,
    /// Racines filesystem autorisées par cette couche (`None` = n'ajoute pas de restriction).
    pub allowed_roots: Option<Vec<PathBuf>>,
    /// Hôtes réseau autorisés (`None` = n'ajoute pas de restriction, `Some(empty)` = aucun).
    pub allowed_network_hosts: Option<BTreeSet<String>>,
    /// Références opaques de secrets accessibles (`None` = n'ajoute pas de restriction).
    pub allowed_secret_refs: Option<BTreeSet<String>>,
    /// Politique de modèle imposée (`Forbidden` domine `AllowedBounded`).
    pub model_policy: ModelPolicy,
    /// Contrainte de localité des données (`LocalOnly` domine `ApprovedRemote`).
    pub data_locality: DataLocality,
    /// Mode d'isolation (`Strict` domine `Standard`).
    pub isolation_mode: IsolationMode,
    /// Exige un confinement OS explicite par action même si le projet est `Standard`.
    pub require_action_isolation: bool,
    /// Portée maximale permise (l'intersection prend le minimum).
    pub max_scope_kind: ScopeKind,
    /// Autorisation de délégation (`&&`).
    pub allow_delegation: bool,
    /// Autorisation inter-dépôts (`&&`).
    pub allow_cross_repository: bool,
}

/// Trace explicite expliquant quelle couche a restreint une capacité lors de l'intersection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitingAuthorityTrace {
    /// Couche responsable de la limitation.
    pub layer_name: String,
    /// Capacité ou attribut restreint.
    pub capability: String,
    /// Motif de la restriction.
    pub reason: String,
}

/// Autorité effective calculée par intersection stricte des couches hiérarchiques.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveAuthority {
    /// Projet associé à ce mandat si applicable.
    pub project_id: Option<ProjectId>,
    /// Révision de politique prise en compte.
    pub policy_revision: Revision,
    /// Révision du scope/mandat prise en compte.
    pub scope_revision: Revision,
    /// Ensemble final des classes d'effets autorisées.
    pub allowed_effects: BTreeSet<EffectsClass>,
    /// Ensemble final des verbes autorisés (si restreint à une allowlist).
    pub allowed_verbs: Option<BTreeSet<String>>,
    /// Ensemble final des verbes explicitement interdits.
    pub denied_verbs: BTreeSet<String>,
    /// Racines filesystem autorisées.
    pub allowed_roots: Vec<PathBuf>,
    /// Hôtes réseau autorisés.
    pub allowed_network_hosts: BTreeSet<String>,
    /// Références de secrets autorisées.
    pub allowed_secret_refs: BTreeSet<String>,
    /// Politique effective de modèle.
    pub model_policy: ModelPolicy,
    /// Politique effective de localité des données.
    pub data_locality: DataLocality,
    /// Mode d'isolation effectif.
    pub isolation_mode: IsolationMode,
    /// Indique si le confinement par action est exigé.
    pub require_action_isolation: bool,
    /// Portée maximale effective.
    pub max_scope_kind: ScopeKind,
    /// Délégation permise.
    pub allow_delegation: bool,
    /// Accès inter-dépôts permis.
    pub allow_cross_repository: bool,
    /// Traces des limitations appliquées par les couches supérieures ou concurrentes.
    pub limiting_traces: Vec<LimitingAuthorityTrace>,
}

impl EffectiveAuthority {
    /// Indique si une classe d'effet est permise par l'autorité effective.
    #[must_use]
    pub fn allows_effect(&self, effect: EffectsClass) -> bool {
        self.allowed_effects.contains(&effect)
    }

    /// Indique si un verbe donné est permis par l'autorité effective.
    #[must_use]
    pub fn allows_verb(&self, verb: &str) -> bool {
        if self.denied_verbs.contains(verb) {
            return false;
        }
        match &self.allowed_verbs {
            Some(allowed) => allowed.contains(verb),
            None => true,
        }
    }

    /// Retourne les noms des autorités limitantes associées.
    #[must_use]
    pub fn limiting_layer_names(&self) -> Vec<String> {
        self.limiting_traces
            .iter()
            .map(|t| t.layer_name.clone())
            .collect()
    }

    /// Contrôle une [`ActionIntent`] contre cette autorité effective avant appel `GuardPort`
    /// et à nouveau avant dispatch (`REQ-SEC-001`, `REQ-SEC-005`, `REQ-SEC-006`, `REQ-SEC-007`).
    ///
    /// Retourne les chemins canoniques résolus des cibles `NormalizedResourceRef::Path` et `GitRef`
    /// pour permettre la vérification TOCTOU au moment du dispatch.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::PolicyDenied`], [`ActionBrokerError::ScopeDenied`],
    /// [`ActionBrokerError::ModelCallsForbidden`], [`ActionBrokerError::DataLocalityViolation`]
    /// ou [`ActionBrokerError::PermissionExpansionDenied`].
    pub fn check_intent(
        &self,
        intent: &ActionIntent,
        now_ms: TimestampMs,
    ) -> Result<Vec<(PathBuf, PathBuf)>, ActionBrokerError> {
        if let ActionProvenance::ExternalSource {
            trust_level: TrustLevel::UntrustedExternal,
            source_label,
            ..
        } = &intent.provenance
            && (intent.policy_revision > self.policy_revision
                || intent.scope_revision > self.scope_revision)
        {
            return Err(ActionBrokerError::PermissionExpansionDenied {
                reason: format!(
                    "Source externe '{source_label}' ne peut ni modifier ni élever la révision de politique/scope"
                ),
                limiting_authority: String::from("effective_authority"),
            });
        }

        if now_ms > intent.action_deadline {
            return Err(ActionBrokerError::PolicyDenied {
                reason: format!(
                    "Date limite de l'action {} dépassée ({} > {})",
                    intent.action_id,
                    now_ms.as_i64(),
                    intent.action_deadline.as_i64()
                ),
                limiting_authorities: self.limiting_layer_names(),
            });
        }

        if intent.policy_revision != self.policy_revision
            || intent.scope_revision != self.scope_revision
        {
            return Err(ActionBrokerError::PolicyDenied {
                reason: "Révision du mandat ou de la politique différente de l'autorité effective"
                    .into(),
                limiting_authorities: self.limiting_layer_names(),
            });
        }

        if let Some(expected_proj) = self.project_id
            && intent.project_id != Some(expected_proj)
        {
            return Err(ActionBrokerError::ScopeDenied {
                reason: format!(
                    "Projet d'action {:?} hors du mandat projet {expected_proj}",
                    intent.project_id
                ),
            });
        }

        if intent.effects_class == EffectsClass::ModelDispatch
            && self.model_policy == ModelPolicy::Forbidden
        {
            return Err(ActionBrokerError::ModelCallsForbidden {
                reason: String::from(
                    "Dispatch modèle interdit sous model_policy=forbidden (INV-018)",
                ),
            });
        }

        if !self.allows_effect(intent.effects_class) {
            return Err(ActionBrokerError::PolicyDenied {
                reason: format!(
                    "Classe d'effet '{}' interdite par l'autorité effective",
                    intent.effects_class
                ),
                limiting_authorities: self.limiting_layer_names(),
            });
        }

        if !self.allows_verb(&intent.verb) {
            return Err(ActionBrokerError::PolicyDenied {
                reason: format!(
                    "Verbe '{}' interdit ou absent de l'intersection des mandats",
                    intent.verb
                ),
                limiting_authorities: self.limiting_layer_names(),
            });
        }

        let required = IsolationRequirements::from_authority(self);
        if (required.mode == IsolationMode::Strict
            && intent.isolation_requirements.mode != IsolationMode::Strict)
            || (required.require_process_confinement
                && !intent.isolation_requirements.require_process_confinement)
            || !required
                .required_capabilities
                .is_subset(&intent.isolation_requirements.required_capabilities)
        {
            return Err(ActionBrokerError::PolicyDenied {
                reason: String::from(
                    "L'action demande le mode standard alors que l'autorité exige le mode strict",
                ),
                limiting_authorities: self.limiting_layer_names(),
            });
        }

        for sref in &intent.secret_refs {
            if !self.allowed_secret_refs.contains(sref.as_opaque_str()) {
                return Err(ActionBrokerError::SecretAccessDenied {
                    reason: String::from(
                        "Référence de secret non autorisée par le mandat effectif",
                    ),
                });
            }
        }

        self.check_resources(intent)
    }

    fn check_resources(
        &self,
        intent: &ActionIntent,
    ) -> Result<Vec<(PathBuf, PathBuf)>, ActionBrokerError> {
        let mut verified_paths = Vec::new();
        for res in &intent.resource_refs {
            match res {
                NormalizedResourceRef::Path(path) => {
                    let canon = validate_path_within_roots(path, &self.allowed_roots)?;
                    verified_paths.push((path.clone(), canon));
                }
                NormalizedResourceRef::GitRef { repo, .. } => {
                    let canon = validate_path_within_roots(repo, &self.allowed_roots)?;
                    verified_paths.push((repo.clone(), canon));
                }
                NormalizedResourceRef::NetworkEndpoint { host, .. } => {
                    if self.data_locality == DataLocality::LocalOnly && !is_loopback_host(host) {
                        return Err(ActionBrokerError::DataLocalityViolation {
                            reason: format!(
                                "Destination réseau '{host}' refusée sous data_locality=local_only"
                            ),
                        });
                    }
                    if !self.allowed_network_hosts.contains(host) {
                        return Err(ActionBrokerError::PolicyDenied {
                            reason: format!(
                                "Hôte réseau '{host}' absent de la liste d'hôtes du mandat"
                            ),
                            limiting_authorities: self.limiting_layer_names(),
                        });
                    }
                }
                NormalizedResourceRef::ModelTarget { locality, .. } => {
                    if self.data_locality == DataLocality::LocalOnly
                        && *locality == DataLocality::ApprovedRemote
                    {
                        return Err(ActionBrokerError::DataLocalityViolation {
                            reason: String::from(
                                "Modèle distant refusé sous data_locality=local_only",
                            ),
                        });
                    }
                }
                NormalizedResourceRef::ConnectorResource { .. }
                | NormalizedResourceRef::Terminal(_) => {}
            }
        }

        Ok(verified_paths)
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Calcule l'intersection stricte d'une pile ordonnée de couches d'autorité (`REQ-SEC-001`).
///
/// Aucune couche inférieure ou concurrente ne peut élargir les droits accordés par une autre couche ;
/// toute interdiction l'emporte systématiquement, `Forbidden` domine `AllowedBounded`,
/// `LocalOnly` domine `ApprovedRemote` et `Strict` domine `Standard`.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "One ordered pass intersects every independent security dimension without widening grants"
)]
pub fn compute_effective_authority(
    project_id: Option<ProjectId>,
    policy_revision: Revision,
    scope_revision: Revision,
    layers: &[AuthorityLayer],
) -> EffectiveAuthority {
    let mut allowed_effects: BTreeSet<EffectsClass> = EffectsClass::ALL.into_iter().collect();
    let mut allowed_verbs: Option<BTreeSet<String>> = None;
    let mut denied_verbs: BTreeSet<String> = BTreeSet::new();
    let mut allowed_roots: Option<Vec<PathBuf>> = None;
    let mut allowed_network_hosts: Option<BTreeSet<String>> = None;
    let mut allowed_secret_refs: Option<BTreeSet<String>> = None;
    let mut model_policy = ModelPolicy::AllowedBounded;
    let mut data_locality = DataLocality::ApprovedRemote;
    let mut isolation_mode = IsolationMode::Standard;
    let mut require_action_isolation = false;
    let mut max_scope_kind = ScopeKind::Global;
    let mut allow_delegation = true;
    let mut allow_cross_repository = true;
    let mut limiting_traces = Vec::new();

    for layer in layers {
        if let Some(layer_effects) = &layer.allowed_effects {
            let removed: Vec<_> = allowed_effects.difference(layer_effects).copied().collect();
            for eff in removed {
                limiting_traces.push(LimitingAuthorityTrace {
                    layer_name: layer.layer_name.clone(),
                    capability: format!("effect:{eff}"),
                    reason: String::from("absent de la liste d'effets autorisés de la couche"),
                });
            }
            allowed_effects = allowed_effects
                .intersection(layer_effects)
                .copied()
                .collect();
        }

        for denied in &layer.denied_effects {
            if allowed_effects.remove(denied) {
                limiting_traces.push(LimitingAuthorityTrace {
                    layer_name: layer.layer_name.clone(),
                    capability: format!("effect:{denied}"),
                    reason: String::from("explicitement interdit par la couche"),
                });
            }
        }

        if let Some(layer_verbs) = &layer.allowed_verbs {
            allowed_verbs = Some(match allowed_verbs {
                None => layer_verbs.clone(),
                Some(prev) => {
                    for dropped in prev.difference(layer_verbs) {
                        limiting_traces.push(LimitingAuthorityTrace {
                            layer_name: layer.layer_name.clone(),
                            capability: format!("verb:{dropped}"),
                            reason: String::from("exclu par l'intersection des verbes"),
                        });
                    }
                    prev.intersection(layer_verbs).cloned().collect()
                }
            });
        }

        for dv in &layer.denied_verbs {
            denied_verbs.insert(dv.clone());
            limiting_traces.push(LimitingAuthorityTrace {
                layer_name: layer.layer_name.clone(),
                capability: format!("verb:{dv}"),
                reason: String::from("verbe explicitement refusé"),
            });
        }

        if let Some(layer_roots) = &layer.allowed_roots {
            allowed_roots = Some(match allowed_roots {
                None => layer_roots.clone(),
                Some(prev) => intersect_roots(&prev, layer_roots),
            });
        }

        if let Some(layer_hosts) = &layer.allowed_network_hosts {
            allowed_network_hosts = Some(match allowed_network_hosts {
                None => layer_hosts.clone(),
                Some(prev) => {
                    for dropped in prev.difference(layer_hosts) {
                        limiting_traces.push(LimitingAuthorityTrace {
                            layer_name: layer.layer_name.clone(),
                            capability: format!("host:{dropped}"),
                            reason: String::from("hôte réseau non admis par cette couche"),
                        });
                    }
                    prev.intersection(layer_hosts).cloned().collect()
                }
            });
        }

        if let Some(layer_secrets) = &layer.allowed_secret_refs {
            allowed_secret_refs = Some(match allowed_secret_refs {
                None => layer_secrets.clone(),
                Some(prev) => prev.intersection(layer_secrets).cloned().collect(),
            });
        }

        if layer.model_policy == ModelPolicy::Forbidden && model_policy != ModelPolicy::Forbidden {
            model_policy = ModelPolicy::Forbidden;
            limiting_traces.push(LimitingAuthorityTrace {
                layer_name: layer.layer_name.clone(),
                capability: String::from("model_policy"),
                reason: String::from("model_policy=forbidden domine allowed_bounded"),
            });
        }

        if layer.data_locality == DataLocality::LocalOnly
            && data_locality != DataLocality::LocalOnly
        {
            data_locality = DataLocality::LocalOnly;
            limiting_traces.push(LimitingAuthorityTrace {
                layer_name: layer.layer_name.clone(),
                capability: String::from("data_locality"),
                reason: String::from("data_locality=local_only domine approved_remote"),
            });
        }

        if layer.isolation_mode == IsolationMode::Strict && isolation_mode != IsolationMode::Strict
        {
            isolation_mode = IsolationMode::Strict;
            limiting_traces.push(LimitingAuthorityTrace {
                layer_name: layer.layer_name.clone(),
                capability: String::from("isolation_mode"),
                reason: String::from("isolation_mode=strict domine standard"),
            });
        }

        require_action_isolation |= layer.require_action_isolation;

        if layer.max_scope_kind < max_scope_kind {
            max_scope_kind = layer.max_scope_kind;
            limiting_traces.push(LimitingAuthorityTrace {
                layer_name: layer.layer_name.clone(),
                capability: String::from("max_scope_kind"),
                reason: format!("plafonnement de portée à {:?}", layer.max_scope_kind),
            });
        }

        allow_delegation &= layer.allow_delegation;
        allow_cross_repository &= layer.allow_cross_repository;
    }

    if let Some(ref mut verbs) = allowed_verbs {
        verbs.retain(|v| !denied_verbs.contains(v));
    }

    EffectiveAuthority {
        project_id,
        policy_revision,
        scope_revision,
        allowed_effects,
        allowed_verbs,
        denied_verbs,
        allowed_roots: allowed_roots.unwrap_or_default(),
        allowed_network_hosts: allowed_network_hosts.unwrap_or_default(),
        allowed_secret_refs: allowed_secret_refs.unwrap_or_default(),
        model_policy,
        data_locality,
        isolation_mode,
        require_action_isolation,
        max_scope_kind,
        allow_delegation,
        allow_cross_repository,
        limiting_traces,
    }
}

fn intersect_roots(a: &[PathBuf], b: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for pa in a {
        for pb in b {
            let na = normalize_lexical(pa);
            let nb = normalize_lexical(pb);
            if na.starts_with(&nb) {
                if !out.contains(&na) {
                    out.push(na);
                }
            } else if nb.starts_with(&na) && !out.contains(&nb) {
                out.push(nb);
            }
        }
    }
    out
}

/// Vérifie qu'un rôle ou skill ne tente pas d'étendre explicitement les permissions au-delà du plafond
/// supérieur (`CapabilityDenied/PERMISSION_EXPANSION_DENIED`).
///
/// # Errors
/// Retourne [`ActionBrokerError::PermissionExpansionDenied`] si un droit demandé dépasse `ceiling`.
pub fn validate_no_permission_expansion(
    requested_effects: &BTreeSet<EffectsClass>,
    requested_verbs: &BTreeSet<String>,
    requested_hosts: &BTreeSet<String>,
    requested_model_policy: Option<ModelPolicy>,
    requested_locality: Option<DataLocality>,
    ceiling: &EffectiveAuthority,
) -> Result<(), ActionBrokerError> {
    for eff in requested_effects {
        if !ceiling.allows_effect(*eff) {
            return Err(ActionBrokerError::PermissionExpansionDenied {
                reason: format!("Effet demandé '{eff}' interdit par le plafond supérieur"),
                limiting_authority: ceiling
                    .limiting_layer_names()
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| String::from("policy_ceiling")),
            });
        }
    }

    for verb in requested_verbs {
        if !ceiling.allows_verb(verb) {
            return Err(ActionBrokerError::PermissionExpansionDenied {
                reason: format!("Verbe demandé '{verb}' interdit par le plafond supérieur"),
                limiting_authority: ceiling
                    .limiting_layer_names()
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| String::from("policy_ceiling")),
            });
        }
    }

    for host in requested_hosts {
        if !ceiling.allowed_network_hosts.contains(host) {
            return Err(ActionBrokerError::PermissionExpansionDenied {
                reason: format!("Hôte réseau demandé '{host}' hors du plafond réseau"),
                limiting_authority: ceiling
                    .limiting_layer_names()
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| String::from("policy_ceiling")),
            });
        }
    }

    if requested_model_policy == Some(ModelPolicy::AllowedBounded)
        && ceiling.model_policy == ModelPolicy::Forbidden
    {
        return Err(ActionBrokerError::PermissionExpansionDenied {
            reason: String::from("Tentative d'activer un modèle sous model_policy=forbidden"),
            limiting_authority: String::from("model_policy_ceiling"),
        });
    }

    if requested_locality == Some(DataLocality::ApprovedRemote)
        && ceiling.data_locality == DataLocality::LocalOnly
    {
        return Err(ActionBrokerError::PermissionExpansionDenied {
            reason: String::from(
                "Tentative d'activer une sortie distante sous data_locality=local_only",
            ),
            limiting_authority: String::from("data_locality_ceiling"),
        });
    }

    Ok(())
}

/// Valide qu'un chemin reste strictement confiné dans l'une des racines `allowed_roots`,
/// en refusant `..`, `.claude/` et tout lien symbolique pointant hors racine (`REQ-SEC-005`).
///
/// # Errors
/// Retourne [`ActionBrokerError::ScopeDenied`] si le chemin est invalide, échappe aux racines ou traverse un symlink sortant.
pub fn validate_path_within_roots(
    candidate: &Path,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, ActionBrokerError> {
    if candidate.as_os_str().is_empty() {
        return Err(ActionBrokerError::ScopeDenied {
            reason: String::from("Chemin de ressource vide"),
        });
    }

    if allowed_roots.is_empty() {
        return Err(ActionBrokerError::ScopeDenied {
            reason: String::from("Aucune racine filesystem autorisée dans le mandat"),
        });
    }

    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(ActionBrokerError::ScopeDenied {
            reason: format!(
                "Traversée parent ('..') strictement refusée : '{}'",
                candidate.display()
            ),
        });
    }

    assert_not_under_claude(candidate)?;

    let resolved_candidate = resolve_with_symlink_boundary(candidate)?;
    assert_not_under_claude(&resolved_candidate)?;

    for root in allowed_roots {
        let resolved_root = resolve_with_symlink_boundary(root)?;
        if resolved_candidate.starts_with(&resolved_root) {
            return Ok(resolved_candidate);
        }
    }

    Err(ActionBrokerError::ScopeDenied {
        reason: format!(
            "Chemin '{}' (résolu '{}') hors des racines autorisées",
            candidate.display(),
            resolved_candidate.display()
        ),
    })
}

/// Revalide juste avant exécution qu'aucun remplacement TOCTOU (ex. swap de symlink) n'a modifié
/// la cible autorisée (`REQ-SEC-005`).
///
/// # Errors
/// Retourne [`ActionBrokerError::ScopeDenied`] si la cible a changé ou sort des racines autorisées.
pub fn verify_path_toctou(
    authorized_canonical: &Path,
    original_candidate: &Path,
    allowed_roots: &[PathBuf],
) -> Result<(), ActionBrokerError> {
    let revalidated = validate_path_within_roots(original_candidate, allowed_roots)?;
    if revalidated != authorized_canonical {
        return Err(ActionBrokerError::ScopeDenied {
            reason: format!(
                "Substitution TOCTOU détectée sur '{}' : cible initiale '{}', cible actuelle '{}'",
                original_candidate.display(),
                authorized_canonical.display(),
                revalidated.display()
            ),
        });
    }
    Ok(())
}

/// Vérifie qu'une demande de mutation de politique/fournisseur/budget ne provient pas d'une source
/// externe non fiable (`REQ-SEC-006`).
///
/// # Errors
/// Retourne [`ActionBrokerError::PermissionExpansionDenied`] si la source est non fiable.
pub fn validate_authority_mutation_provenance(
    provenance: &ActionProvenance,
) -> Result<(), ActionBrokerError> {
    if let ActionProvenance::ExternalSource {
        source_kind,
        source_label,
        trust_level: TrustLevel::UntrustedExternal,
    } = provenance
    {
        return Err(ActionBrokerError::PermissionExpansionDenied {
            reason: format!(
                "Une source externe non fiable ({source_kind:?}: '{source_label}') ne peut pas modifier politique, fournisseur, budget ou permissions"
            ),
            limiting_authority: String::from("control_authority_boundary"),
        });
    }
    Ok(())
}

fn resolve_with_symlink_boundary(path: &Path) -> Result<PathBuf, ActionBrokerError> {
    let lexical = normalize_lexical(path);
    if let Ok(canon) = std::fs::canonicalize(&lexical) {
        return Ok(canon);
    }

    // Remonter jusqu'au plus long préfixe existant afin de résoudre d'éventuels symlinks
    // sur les répertoires parents même lorsque le fichier feuille n'existe pas encore.
    let mut existing = lexical.as_path();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() && std::fs::symlink_metadata(existing).is_err() {
        let Some(name) = existing.file_name() else {
            break;
        };
        tail.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            break;
        };
        if parent.as_os_str().is_empty() {
            break;
        }
        existing = parent;
    }

    let mut base = if existing.exists() {
        std::fs::canonicalize(existing).map_err(|e| ActionBrokerError::ScopeDenied {
            reason: format!(
                "Impossible de canoniser le préfixe '{}' : {e}",
                existing.display()
            ),
        })?
    } else if std::fs::symlink_metadata(existing).is_ok() {
        // Lien symbolique cassé -> ne jamais résoudre silencieusement hors périmètre.
        return Err(ActionBrokerError::ScopeDenied {
            reason: format!(
                "Lien symbolique cassé ou cible inaccessible : '{}'",
                existing.display()
            ),
        });
    } else {
        existing.to_path_buf()
    };

    for part in tail.into_iter().rev() {
        base.push(part);
    }
    Ok(normalize_lexical(&base))
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_never_widens_grants() {
        let upper = AuthorityLayer {
            layer_name: String::from("upper"),
            allowed_effects: Some(BTreeSet::from([
                EffectsClass::LocalRead,
                EffectsClass::LocalWrite,
            ])),
            denied_effects: BTreeSet::from([EffectsClass::LocalWrite]),
            allowed_verbs: Some(BTreeSet::from([
                String::from("fs.read"),
                String::from("fs.write"),
            ])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: None,
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Strict,
            require_action_isolation: true,
            max_scope_kind: ScopeKind::Repository,
            allow_delegation: false,
            allow_cross_repository: false,
        };
        let lower = AuthorityLayer {
            layer_name: String::from("lower"),
            allowed_effects: Some(BTreeSet::from([
                EffectsClass::LocalRead,
                EffectsClass::LocalWrite,
                EffectsClass::GitRemote,
            ])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([
                String::from("fs.read"),
                String::from("git.push"),
            ])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: None,
            allowed_network_hosts: Some(BTreeSet::from([String::from("github.com")])),
            allowed_secret_refs: Some(BTreeSet::from([String::from("sec:1")])),
            model_policy: ModelPolicy::AllowedBounded,
            data_locality: DataLocality::ApprovedRemote,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: ScopeKind::Global,
            allow_delegation: true,
            allow_cross_repository: true,
        };

        let eff =
            compute_effective_authority(None, Revision::new(1), Revision::new(1), &[upper, lower]);
        assert_eq!(
            eff.allowed_effects,
            BTreeSet::from([EffectsClass::LocalRead])
        );
        assert_eq!(eff.model_policy, ModelPolicy::Forbidden);
        assert_eq!(eff.data_locality, DataLocality::LocalOnly);
        assert_eq!(eff.isolation_mode, IsolationMode::Strict);
        assert!(eff.allowed_network_hosts.is_empty());
        assert!(eff.allowed_secret_refs.is_empty());
        assert_eq!(eff.max_scope_kind, ScopeKind::Repository);
        assert!(!eff.allow_delegation);
        assert!(!eff.allow_cross_repository);
    }
}
