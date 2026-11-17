//! Moteur expérimental de connaissance multilangage et de capture Git/worktree
//! (Scénarios 1 & 2 du spike M0 n°4, `REQ-KNOW-001`, `REQ-KNOW-002`, `REQ-KNOW-003`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use unleash_domain::{ClockPort, IdGeneratorPort, StreamId};
use unleash_protocol::ReasonCode;

use crate::errors::SpikeError;
use crate::types::{
    CaptureConsistency, ContentDigest, DiscoveryCause, DiscoveryCauseKind, DiscoveryDecision,
    DiscoveryOrigin, DiscoveryPurpose, DiscoveryReceipt, DiscoveryStrategy, EventCursorRef,
    Freshness, HexSha256, SnapshotKind, SnapshotRef, TemporalRelation, TypedSourceRef,
    ValidationBasis, timestamp_ms_to_utc,
};

/// Langages couverts en V1 (`shared-knowledge.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LanguageProfile {
    /// Rust (`rust-analyzer`).
    Rust,
    /// Python (`pyright`).
    Python,
    /// TypeScript (`typescript-language-server`).
    TypeScript,
    /// Go (`gopls`).
    Go,
    /// Java (`jdtls`).
    Java,
}

impl LanguageProfile {
    /// Nom lisible du profil de langage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Go => "go",
            Self::Java => "java",
        }
    }

    /// Nom de l'exécutable analyseur V1 attendu selon `shared-knowledge.md`.
    #[must_use]
    pub const fn expected_binary(self) -> &'static str {
        match self {
            Self::Rust => "rust-analyzer",
            Self::Python => "pyright",
            Self::TypeScript => "typescript-language-server",
            Self::Go => "gopls",
            Self::Java => "jdtls",
        }
    }

    /// Déduit le langage à partir de l'extension de fichier.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        match ext {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "ts" | "tsx" | "mts" | "cts" => Some(Self::TypeScript),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            _ => None,
        }
    }
}

/// État de qualification d'un profil d'analyse (`shared-knowledge.md` R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileAvailability {
    /// Binaire détecté, checksum calculé et handshake LSP + fixture qualifiés.
    Ready,
    /// Binaire présent mais qualification LSP incomplète ou capacité réduite.
    Degraded,
    /// Analyseur absent, non exécutable ou non qualifié (`profile_unavailable`).
    Unavailable,
}

/// Rapport de détection et de qualification d'un analyseur externe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzerQualification {
    /// Langage ciblé.
    pub language: LanguageProfile,
    /// Nom du binaire attendu.
    pub binary_name: String,
    /// État de disponibilité (`Ready`, `Degraded`, `Unavailable`).
    pub availability: ProfileAvailability,
    /// Motif normatif en cas d'indisponibilité (`ReasonCode::ProfileUnavailable`).
    pub reason_code: Option<ReasonCode>,
    /// Chemin canonique résolu s'il existe.
    pub canonical_path: Option<PathBuf>,
    /// Source de détection (`explicit_override`, `search_paths`, `process_path`, `absent`).
    pub detection_source: String,
    /// Sortie de version observée (`--version`).
    pub declared_version: Option<String>,
    /// Empreinte SHA-256 du binaire exécutable.
    pub binary_sha256: Option<HexSha256>,
    /// Limites explicites annoncées sur les résultats.
    pub limitations: Vec<String>,
}

/// Configuration de détection des analyseurs externes.
#[derive(Debug, Clone, Default)]
pub struct AnalyzerProbeConfig {
    /// Chemins explicites par langage (prioritaires).
    pub explicit_paths: BTreeMap<LanguageProfile, PathBuf>,
    /// Répertoires de recherche supplémentaires.
    pub search_dirs: Vec<PathBuf>,
    /// Si `true`, autorise le repli sur le `PATH` du processus courant.
    pub allow_process_path: bool,
    /// Profils forcés à `Ready` uniquement si un fournisseur sémantique réel est connecté en test.
    pub prequalified_lsp_languages: Vec<LanguageProfile>,
}

impl AnalyzerProbeConfig {
    /// Configuration déterministe de test sans consultation du `PATH` système.
    #[must_use]
    pub fn isolated() -> Self {
        Self::default()
    }

    /// Configuration inspectant les emplacements usuels + `PATH` courant (sans écriture).
    #[must_use]
    pub fn host_toolchain() -> Self {
        let mut search_dirs = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            let home_path = PathBuf::from(home);
            search_dirs.push(home_path.join(".cargo/bin"));
            search_dirs.push(home_path.join("go/bin"));
            search_dirs.push(home_path.join(".local/bin"));
        }
        search_dirs.push(PathBuf::from("/opt/homebrew/bin"));
        search_dirs.push(PathBuf::from("/usr/local/bin"));
        Self {
            explicit_paths: BTreeMap::new(),
            search_dirs,
            allow_process_path: true,
            prequalified_lsp_languages: Vec::new(),
        }
    }
}

/// Détecte et qualifie un analyseur externe pour `language` sans jamais télécharger ni installer.
///
/// # Errors
/// Ne renvoie une erreur que sur échec système imprévu ; une absence de binaire produit un
/// rapport [`ProfileAvailability::Unavailable`] avec `ReasonCode::ProfileUnavailable`.
pub fn probe_analyzer(
    language: LanguageProfile,
    config: &AnalyzerProbeConfig,
) -> Result<AnalyzerQualification, SpikeError> {
    let bin_name = language.expected_binary();
    let (found, source) = locate_binary(language, bin_name, config);

    let Some(candidate_path) = found else {
        return Ok(AnalyzerQualification {
            language,
            binary_name: bin_name.to_owned(),
            availability: ProfileAvailability::Unavailable,
            reason_code: Some(ReasonCode::ProfileUnavailable),
            canonical_path: None,
            detection_source: String::from("absent"),
            declared_version: None,
            binary_sha256: None,
            limitations: vec![format!(
                "Analyseur '{bin_name}' absent : mode lexical seul autorisé, aucune relation semantic_resolved émise"
            )],
        });
    };

    let canonical_path = fs::canonicalize(&candidate_path).unwrap_or(candidate_path);
    let binary_bytes = match fs::read(&canonical_path) {
        Ok(b) => b,
        Err(err) => {
            return Ok(AnalyzerQualification {
                language,
                binary_name: bin_name.to_owned(),
                availability: ProfileAvailability::Unavailable,
                reason_code: Some(ReasonCode::ProfileUnavailable),
                canonical_path: Some(canonical_path),
                detection_source: source,
                declared_version: None,
                binary_sha256: None,
                limitations: vec![format!("Binaire illisible ({err}) : profile_unavailable")],
            });
        }
    };
    let binary_sha256 = Some(HexSha256::of_bytes(&binary_bytes));

    let version_output = Command::new(&canonical_path).arg("--version").output();
    match version_output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            let declared_version = if stdout.is_empty() {
                None
            } else {
                Some(stdout)
            };
            let is_lsp_qualified = config.prequalified_lsp_languages.contains(&language);
            let (availability, limitations) = if is_lsp_qualified {
                (ProfileAvailability::Ready, Vec::new())
            } else {
                (
                    ProfileAvailability::Degraded,
                    vec![String::from(
                        "Binaire détecté mais handshake LSP initialize + requête fixture non qualifiés dans ce spike : dégradation lexicale explicite",
                    )],
                )
            };
            Ok(AnalyzerQualification {
                language,
                binary_name: bin_name.to_owned(),
                availability,
                reason_code: None,
                canonical_path: Some(canonical_path),
                detection_source: source,
                declared_version,
                binary_sha256,
                limitations,
            })
        }
        Ok(out) => Ok(AnalyzerQualification {
            language,
            binary_name: bin_name.to_owned(),
            availability: ProfileAvailability::Unavailable,
            reason_code: Some(ReasonCode::ProfileUnavailable),
            canonical_path: Some(canonical_path),
            detection_source: source,
            declared_version: None,
            binary_sha256,
            limitations: vec![format!(
                "Échec de --version (code={:?}) : profile_unavailable",
                out.status.code()
            )],
        }),
        Err(err) => Ok(AnalyzerQualification {
            language,
            binary_name: bin_name.to_owned(),
            availability: ProfileAvailability::Unavailable,
            reason_code: Some(ReasonCode::ProfileUnavailable),
            canonical_path: Some(canonical_path),
            detection_source: source,
            declared_version: None,
            binary_sha256,
            limitations: vec![format!(
                "Exécution impossible de '{bin_name}' ({err}) : profile_unavailable"
            )],
        }),
    }
}

fn locate_binary(
    language: LanguageProfile,
    bin_name: &str,
    config: &AnalyzerProbeConfig,
) -> (Option<PathBuf>, String) {
    if let Some(explicit) = config.explicit_paths.get(&language) {
        if explicit.is_file() {
            return (Some(explicit.clone()), String::from("explicit_override"));
        }
        return (None, String::from("explicit_override_missing"));
    }

    for dir in &config.search_dirs {
        let candidate = dir.join(bin_name);
        if candidate.is_file() {
            return (Some(candidate), String::from("search_dirs"));
        }
    }

    if config.allow_process_path
        && let Ok(path_var) = std::env::var("PATH")
    {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return (Some(candidate), String::from("process_path"));
            }
        }
    }

    (None, String::from("absent"))
}

/// Niveau de confiance catégoriel d'une relation ou découverte (`shared-knowledge.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationConfidence {
    /// Résolu sémantiquement par un serveur LSP qualifié.
    SemanticResolved,
    /// Analysé syntaxiquement (tree-sitter).
    Syntactic,
    /// Extrait par balayage lexical uniquement.
    Lexical,
    /// Déclaré manuellement / contrat explicite.
    Declared,
    /// Inconnu.
    Unknown,
}

/// Symbole découvert dans un fichier du dépôt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSymbol {
    /// Identifiant candidat (`symbol:<lang>:<name>`).
    pub candidate_id: String,
    /// Langage d'origine.
    pub language: LanguageProfile,
    /// Chemin relatif dans le dépôt.
    pub file_path: String,
    /// Numéro de ligne (1-based).
    pub line: usize,
    /// Nom du symbole.
    pub name: String,
    /// Méthode d'extraction.
    pub method: String,
    /// Niveau de confiance catégoriel.
    pub confidence: RelationConfidence,
}

/// Relation entre symboles ou fichiers avec provenance explicite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredRelation {
    /// Langage source.
    pub language: LanguageProfile,
    /// Symbole ou fichier source.
    pub from: String,
    /// Symbole cible mentionné.
    pub to: String,
    /// Niveau de confiance catégoriel (`SemanticResolved` interdit sans LSP qualifié).
    pub confidence: RelationConfidence,
    /// Profil ayant produit la relation.
    pub profile: String,
}

/// Résultat complet d'une découverte multilangage sur un dépôt local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultilanguageDiscoveryOutput {
    /// Snapshot source effectivement observé.
    pub snapshot: CapturedSnapshot,
    /// Qualification de chaque langage présent dans le dépôt.
    pub qualifications: BTreeMap<LanguageProfile, AnalyzerQualification>,
    /// Symboles découverts.
    pub symbols: Vec<DiscoveredSymbol>,
    /// Relations produites (sans aucune relation `SemanticResolved` fabriquée).
    pub relations: Vec<DiscoveredRelation>,
    /// Reçu durable `DiscoveryReceipt V1` prêt pour validation JSON Schema.
    pub receipt: DiscoveryReceipt,
}

/// État Git du code (`shared-knowledge.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodeState {
    /// Arbre de travail propre aligné sur un commit.
    Committed,
    /// Modifications locales non committées ou fichiers non suivis.
    Dirty,
    /// Dépôt sans commit initial (`unborn`).
    Unborn,
}

/// Entrée individuelle d'un manifeste canonique d'overlay (`REQ-KNOW-001` / `REQ-KNOW-002`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayEntry {
    /// Chemin relatif UTF-8 (lossy pour affichage) + hex des octets bruts du chemin.
    pub path_bytes_hex: String,
    /// Code d'état porcelain Git (`"  "` suivi propre, `"M "`, `" M"`, `"??"`, `"D "`, etc.).
    pub git_status: String,
    /// Mode POSIX / type (`file`, `exec`, `symlink`, `deleted`).
    pub entry_kind: String,
    /// OID de l'index Git (`git ls-files --stage`) si présent.
    pub index_oid: Option<String>,
    /// Empreinte SHA-256 des octets actuels sur disque (ou cible de symlink), `None` si supprimé.
    pub worktree_sha256: Option<HexSha256>,
}

/// Manifeste canonique du contenu capturé (RFC 8785 + SHA-256).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayManifest {
    /// Version de format du manifeste.
    pub format_version: u64,
    /// Commit de base (`None` si `unborn`).
    pub base_commit_oid: Option<String>,
    /// Tree de base (`None` si `unborn`).
    pub base_tree_oid: Option<String>,
    /// Entrées triées de manière déterministe.
    pub entries: Vec<OverlayEntry>,
}

/// Snapshot capturé depuis un dépôt Git local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedSnapshot {
    /// État Git observé.
    pub code_state: CodeState,
    /// Commit `HEAD` (`None` si `unborn`).
    pub base_commit_oid: Option<String>,
    /// Tree `HEAD^{tree}` (`None` si `unborn`).
    pub base_tree_oid: Option<String>,
    /// Niveau de cohérence prouvé.
    pub capture_consistency: CaptureConsistency,
    /// Raison typée en cas d'incohérence ou dégradation.
    pub diagnostic_reason: Option<ReasonCode>,
    /// Manifeste initial capturé.
    pub manifest: OverlayManifest,
    /// Digest canonique initial.
    pub manifest_sha256: HexSha256,
    /// Digest canonique observé lors de la passe de vérification (différent si mutation concurrente).
    pub verification_sha256: HexSha256,
    /// Référence typée `TypedSourceRef` pour `DiscoveryReceipt`.
    pub typed_source_ref: TypedSourceRef,
    /// Référence `SnapshotRef` pour `SubjectRef`.
    pub snapshot_ref: SnapshotRef,
}

/// Capture un snapshot Git et détecte toute mutation concurrente via un hook déterministe.
///
/// - Si le worktree est propre et committé (`git_status` vide) et reste inchangé après le hook,
///   la capture obtient `CaptureConsistency::GitTreeCoherent` (`SnapshotKind::GitTree`).
/// - Si le worktree est dirty et qu'une mutation est détectée entre les deux passes, la capture
///   passe à `CaptureConsistency::Inconsistent` (`ReasonCode::SnapshotInconsistent`).
/// - Si le worktree est dirty et identique entre deux scans séquentiels sans snapshot OS atomique,
///   la capture reste explicitement `CaptureConsistency::Unverified` (`SnapshotKind::SourceSnapshot`).
///
/// # Errors
/// Renvoie [`SpikeError::GitCommandFailed`], [`SpikeError::Io`] ou [`SpikeError::Protocol`].
pub fn capture_git_snapshot_with_hook<F>(
    repo_dir: &Path,
    repo_id: &str,
    worktree_id: Option<&str>,
    source_id: &str,
    generation: u64,
    clock: &dyn ClockPort,
    mut between_passes_hook: F,
) -> Result<CapturedSnapshot, SpikeError>
where
    F: FnMut(&Path) -> Result<(), SpikeError>,
{
    let head_oid = run_git_optional(repo_dir, &["rev-parse", "--verify", "HEAD"])?;
    let tree_oid = match head_oid {
        Some(_) => run_git_optional(repo_dir, &["rev-parse", "--verify", "HEAD^{tree}"])?,
        None => None,
    };

    let manifest_before = build_overlay_manifest(repo_dir, head_oid.clone(), tree_oid.clone())?;
    let sha_before = HexSha256::canonical_json(&manifest_before)?;

    between_passes_hook(repo_dir)?;

    let head_after = run_git_optional(repo_dir, &["rev-parse", "--verify", "HEAD"])?;
    let tree_after = match head_after {
        Some(_) => run_git_optional(repo_dir, &["rev-parse", "--verify", "HEAD^{tree}"])?,
        None => None,
    };
    let manifest_after = build_overlay_manifest(repo_dir, head_after.clone(), tree_after.clone())?;
    let sha_after = HexSha256::canonical_json(&manifest_after)?;

    let is_dirty_before = manifest_before.entries.iter().any(|e| e.git_status != "  ");
    let is_dirty_after = manifest_after.entries.iter().any(|e| e.git_status != "  ");

    let code_state = if head_oid.is_none() || head_after.is_none() {
        CodeState::Unborn
    } else if is_dirty_before || is_dirty_after {
        CodeState::Dirty
    } else {
        CodeState::Committed
    };

    let (capture_consistency, diagnostic_reason, snapshot_kind) = if sha_before != sha_after {
        (
            CaptureConsistency::Inconsistent,
            Some(ReasonCode::SnapshotInconsistent),
            SnapshotKind::SourceSnapshot,
        )
    } else if code_state == CodeState::Committed {
        (
            CaptureConsistency::GitTreeCoherent,
            None,
            SnapshotKind::GitTree,
        )
    } else {
        (
            CaptureConsistency::Unverified,
            None,
            SnapshotKind::SourceSnapshot,
        )
    };

    let observed_at = timestamp_ms_to_utc(clock.now_ms())?;
    let sha_hex = sha_before.as_hex();
    let short_sha = sha_hex.get(..12).unwrap_or(&sha_hex);
    let version_id = format!("git-snap-{short_sha}-g{generation}");

    let typed_source_ref = TypedSourceRef::GitSnapshot {
        source_id: source_id.to_owned(),
        version_id: version_id.clone(),
        content_digest: ContentDigest::sha256(sha_before),
        generation,
        access_policy_ref: String::from("policy-repo-local-v1"),
        acl_revision: 1,
        observed_at,
        capture_consistency,
        repo_id: repo_id.to_owned(),
        worktree_id: worktree_id.map(str::to_owned),
    };

    let snapshot_ref = SnapshotRef {
        kind: snapshot_kind,
        id: version_id,
        sha256: sha_before,
    };

    Ok(CapturedSnapshot {
        code_state,
        base_commit_oid: head_oid,
        base_tree_oid: tree_oid,
        capture_consistency,
        diagnostic_reason,
        manifest: manifest_before,
        manifest_sha256: sha_before,
        verification_sha256: sha_after,
        typed_source_ref,
        snapshot_ref,
    })
}

/// Exécute une découverte multilangage (Rust + Python + TypeScript) sur un dépôt Git sous `/tmp`
/// et émet un [`DiscoveryReceipt`] complet conforme à `discovery-receipt.schema.json`.
///
/// # Errors
/// Renvoie [`SpikeError`] en cas d'erreur Git, d'I/O ou de canonisation.
pub fn discover_multilanguage_repo(
    repo_dir: &Path,
    repo_id: &str,
    worktree_id: Option<&str>,
    query_symbol: &str,
    probe_config: &AnalyzerProbeConfig,
    clock: &dyn ClockPort,
    ids: &dyn IdGeneratorPort,
) -> Result<MultilanguageDiscoveryOutput, SpikeError> {
    let snapshot = capture_git_snapshot_with_hook(
        repo_dir,
        repo_id,
        worktree_id,
        "source-multilang-repo",
        1,
        clock,
        |_| Ok(()),
    )?;

    let mut languages_present = Vec::new();
    let mut symbols = Vec::new();
    let mut relations = Vec::new();

    for entry in &snapshot.manifest.entries {
        if entry.entry_kind == "deleted" {
            continue;
        }
        let rel_path_bytes = hex::decode(&entry.path_bytes_hex)
            .map_err(|e| SpikeError::ContractInvariant(format!("Chemin hex invalide : {e}")))?;
        let rel_path_str = String::from_utf8_lossy(&rel_path_bytes).into_owned();
        let full_path = repo_dir.join(&rel_path_str);
        let Some(lang) = LanguageProfile::from_path(&full_path) else {
            continue;
        };
        if !languages_present.contains(&lang) {
            languages_present.push(lang);
        }
        if let Ok(content) = fs::read_to_string(&full_path) {
            scan_file_lexical(&rel_path_str, lang, &content, &mut symbols, &mut relations);
        }
    }

    let mut qualifications = BTreeMap::new();
    for lang in languages_present {
        let qual = probe_analyzer(lang, probe_config)?;
        qualifications.insert(lang, qual);
    }

    let candidate_ids: Vec<String> = symbols
        .iter()
        .filter(|sym| {
            query_symbol.is_empty()
                || sym
                    .name
                    .to_lowercase()
                    .contains(&query_symbol.to_lowercase())
        })
        .map(|sym| sym.candidate_id.clone())
        .collect();

    let receipt = build_discovery_receipt(&snapshot, &qualifications, candidate_ids, clock, ids)?;

    Ok(MultilanguageDiscoveryOutput {
        snapshot,
        qualifications,
        symbols,
        relations,
        receipt,
    })
}

fn build_discovery_receipt(
    snapshot: &CapturedSnapshot,
    qualifications: &BTreeMap<LanguageProfile, AnalyzerQualification>,
    candidate_ids: Vec<String>,
    clock: &dyn ClockPort,
    ids: &dyn IdGeneratorPort,
) -> Result<DiscoveryReceipt, SpikeError> {
    let (decision, strategy, justification) = if candidate_ids.is_empty() {
        (
            DiscoveryDecision::Create,
            DiscoveryStrategy::New,
            build_justification(qualifications, 0),
        )
    } else {
        (
            DiscoveryDecision::Create,
            DiscoveryStrategy::Adapt,
            build_justification(qualifications, candidate_ids.len()),
        )
    };

    let now = timestamp_ms_to_utc(clock.now_ms())?;
    let max_age_ms: u64 = 300_000;
    let valid_until_ms = clock
        .now_ms()
        .as_i64()
        .saturating_add(i64::try_from(max_age_ms).unwrap_or(300_000));
    let valid_until = chrono::DateTime::from_timestamp_millis(valid_until_ms)
        .ok_or(SpikeError::InvalidTimestamp(valid_until_ms))?;

    let stream_id = StreamId::from_uuid(ids.new_uuid());
    let receipt_id = format!("discovery-{}", ids.new_uuid());
    Ok(DiscoveryReceipt {
        schema_version: String::from("unleash.discovery.v1"),
        receipt_id,
        intent_id: format!("intent-{}", ids.new_uuid()),
        origin: DiscoveryOrigin::Invocation {
            principal_id: String::from("principal-spike-worker-01"),
            cause: DiscoveryCause {
                kind: DiscoveryCauseKind::CommandReceipt,
                id: format!("cmd-receipt-{}", ids.new_uuid()),
            },
            invocation_id: ids.new_invocation_id().to_string(),
            attempt_id: format!("attempt-{}", ids.new_uuid()),
            run_id: Some(ids.new_run_id().to_string()),
            step_id: Some(ids.new_step_id().to_string()),
        },
        query_ids: vec![format!("query-{}", ids.new_uuid())],
        catalog_revision: 1,
        purpose: DiscoveryPurpose::PreCreation,
        decision,
        strategy,
        candidate_ids,
        target_resource_id: None,
        target_version_ref: None,
        justification,
        source_refs: vec![snapshot.typed_source_ref.clone()],
        freshness: if snapshot.capture_consistency == CaptureConsistency::Inconsistent {
            Freshness::Unknown
        } else {
            Freshness::Valid
        },
        temporal_relation: TemporalRelation::BeforeCreation,
        validation_basis: ValidationBasis {
            intent_revision: 1,
            scope_ref: String::from("scope-spike-multilang"),
            scope_revision: 1,
            policy_version_ref: String::from("discovery-policy-v1"),
            validator_version: String::from("discovery-validator-v1"),
            event_cursor: EventCursorRef {
                stream_id,
                sequence: 1,
            },
            checked_at: now,
            valid_until,
            max_age_ms,
        },
        recorded_at: now,
        supersedes_receipt_id: None,
        creation_use_id: None,
        coverage_ref: format!("coverage-{}", ids.new_uuid()),
    })
}

fn build_justification(
    qualifications: &BTreeMap<LanguageProfile, AnalyzerQualification>,
    candidate_count: usize,
) -> String {
    let mut parts = Vec::new();
    for (lang, q) in qualifications {
        let state = match q.availability {
            ProfileAvailability::Ready => "ready",
            ProfileAvailability::Degraded => "degraded_lexical",
            ProfileAvailability::Unavailable => "profile_unavailable",
        };
        parts.push(format!("{}:{state}", lang.as_str()));
    }
    format!(
        "Découverte multilangage ({candidate_count} candidat(s)) avec états de profils [{}].",
        parts.join(", ")
    )
}

fn scan_file_lexical(
    rel_path: &str,
    lang: LanguageProfile,
    content: &str,
    symbols: &mut Vec<DiscoveredSymbol>,
    relations: &mut Vec<DiscoveredRelation>,
) {
    let mut defined_names: Vec<String> = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx.saturating_add(1);
        let trimmed = line.trim();
        if let Some(sym_name) = extract_lexical_symbol(lang, trimmed) {
            defined_names.push(sym_name.clone());
            symbols.push(DiscoveredSymbol {
                candidate_id: format!("symbol:{}:{sym_name}", lang.as_str()),
                language: lang,
                file_path: rel_path.to_owned(),
                line: line_no,
                name: sym_name,
                method: String::from("lexical_scan_v1"),
                confidence: RelationConfidence::Lexical,
            });
        }
    }

    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(target) = extract_lexical_call(lang, trimmed, &defined_names) {
            relations.push(DiscoveredRelation {
                language: lang,
                from: format!("{rel_path}:{}", idx.saturating_add(1)),
                to: target,
                confidence: RelationConfidence::Lexical,
                profile: format!("lexical/{}", lang.as_str()),
            });
        }
    }
}

fn extract_lexical_symbol(lang: LanguageProfile, trimmed: &str) -> Option<String> {
    let prefix_rest = match lang {
        LanguageProfile::Rust => trimmed
            .strip_prefix("pub fn ")
            .or_else(|| trimmed.strip_prefix("fn ")),
        LanguageProfile::Python => trimmed
            .strip_prefix("def ")
            .or_else(|| trimmed.strip_prefix("class ")),
        LanguageProfile::TypeScript => trimmed
            .strip_prefix("export function ")
            .or_else(|| trimmed.strip_prefix("function ")),
        LanguageProfile::Go => trimmed.strip_prefix("func "),
        LanguageProfile::Java => trimmed.strip_prefix("public class "),
    }?;
    let ident: String = prefix_rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if ident.is_empty() { None } else { Some(ident) }
}

fn extract_lexical_call(
    lang: LanguageProfile,
    trimmed: &str,
    defined_names: &[String],
) -> Option<String> {
    // Ignore la ligne de définition elle-même.
    if extract_lexical_symbol(lang, trimmed).is_some() {
        return None;
    }
    for name in defined_names {
        let call_pattern = format!("{name}(");
        if trimmed.contains(&call_pattern) {
            return Some(format!("symbol:{}:{name}", lang.as_str()));
        }
    }
    None
}

fn build_overlay_manifest(
    repo_dir: &Path,
    base_commit_oid: Option<String>,
    base_tree_oid: Option<String>,
) -> Result<OverlayManifest, SpikeError> {
    let stage_out = run_git_bytes(repo_dir, &["ls-files", "--stage", "-z"])?;
    let mut index_oids: BTreeMap<Vec<u8>, String> = BTreeMap::new();
    for rec in stage_out.split(|b| *b == 0) {
        if rec.is_empty() {
            continue;
        }
        // Format: "<mode> <oid> <stage>\t<path>"
        if let Some(tab_pos) = rec.iter().position(|b| *b == b'\t')
            && let Some(meta) = rec.get(..tab_pos)
            && let Some(path_bytes) = rec.get(tab_pos.saturating_add(1)..)
        {
            let meta_str = String::from_utf8_lossy(meta);
            if let Some(oid) = meta_str.split_whitespace().nth(1) {
                index_oids.insert(path_bytes.to_vec(), oid.to_owned());
            }
        }
    }

    let status_out = run_git_bytes(repo_dir, &["status", "--porcelain=v1", "-z", "-uall"])?;
    let mut status_map: BTreeMap<Vec<u8>, String> = BTreeMap::new();
    let mut tokens = status_out.split(|b| *b == 0);
    while let Some(rec) = tokens.next() {
        if rec.is_empty() {
            continue;
        }
        if rec.len() >= 4
            && let Some(xy_bytes) = rec.get(0..2)
            && let Some(path_bytes) = rec.get(3..)
        {
            let xy = String::from_utf8_lossy(xy_bytes).into_owned();
            if xy.starts_with('R') || xy.starts_with('C') {
                // Consume the old path entry in `-z` format.
                let _ = tokens.next();
            }
            status_map.insert(path_bytes.to_vec(), xy);
        }
    }

    let mut all_paths: BTreeMap<Vec<u8>, String> = BTreeMap::new();
    for path_bytes in index_oids.keys() {
        let st = status_map
            .get(path_bytes)
            .cloned()
            .unwrap_or_else(|| String::from("  "));
        all_paths.insert(path_bytes.clone(), st);
    }
    for (path_bytes, st) in status_map {
        all_paths.insert(path_bytes, st);
    }

    let mut entries = Vec::with_capacity(all_paths.len());
    for (path_bytes, git_status) in all_paths {
        let rel_str = String::from_utf8_lossy(&path_bytes).into_owned();
        let full_path = repo_dir.join(&rel_str);
        let index_oid = index_oids.get(&path_bytes).cloned();

        let (entry_kind, worktree_sha256) = match fs::symlink_metadata(&full_path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    let target = fs::read_link(&full_path)?;
                    let digest = sha256_bytes(target.to_string_lossy().as_bytes());
                    (String::from("symlink"), Some(digest))
                } else if meta.is_file() {
                    let bytes = fs::read(&full_path)?;
                    let digest = sha256_bytes(&bytes);
                    #[cfg(unix)]
                    let is_exec = {
                        use std::os::unix::fs::PermissionsExt;
                        meta.permissions().mode() & 0o111 != 0
                    };
                    #[cfg(not(unix))]
                    let is_exec = false;
                    let kind = if is_exec { "exec" } else { "file" };
                    (String::from(kind), Some(digest))
                } else {
                    (String::from("other"), None)
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                (String::from("deleted"), None)
            }
            Err(err) => return Err(SpikeError::Io(err)),
        };

        entries.push(OverlayEntry {
            path_bytes_hex: hex::encode(&path_bytes),
            git_status,
            entry_kind,
            index_oid,
            worktree_sha256,
        });
    }

    Ok(OverlayManifest {
        format_version: 1,
        base_commit_oid,
        base_tree_oid,
        entries,
    })
}

fn sha256_bytes(bytes: &[u8]) -> HexSha256 {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let raw: [u8; 32] = hasher.finalize().into();
    let hex_str = hex::encode(raw);
    let digest = unleash_domain::Sha256Digest::from_raw_bytes(raw);
    debug_assert_eq!(digest.hex(), hex_str);
    HexSha256(digest)
}

fn run_git_bytes(repo_dir: &Path, args: &[&str]) -> Result<Vec<u8>, SpikeError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(SpikeError::GitCommandFailed {
            args: args.join(" "),
            status: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        })
    }
}

fn run_git_optional(repo_dir: &Path, args: &[&str]) -> Result<Option<String>, SpikeError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if s.is_empty() { Ok(None) } else { Ok(Some(s)) }
    } else {
        Ok(None)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use unleash_testkit::{ManualClock, local_test_dir};

    #[test]
    fn unborn_git_repo_yields_none_oid_and_unverified_consistency() {
        let dir = local_test_dir("m0-unborn").unwrap();
        let init = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-b", "main"])
            .output()
            .unwrap();
        assert!(init.status.success());

        fs::write(dir.path().join("README.md"), "draft").unwrap();
        let clock = ManualClock::new(1_763_301_600_000);
        let snap = capture_git_snapshot_with_hook(
            dir.path(),
            "repo-unborn",
            Some("wt-main"),
            "src-unborn",
            1,
            &clock,
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(snap.code_state, CodeState::Unborn);
        assert_eq!(snap.base_commit_oid, None);
        assert_eq!(snap.base_tree_oid, None);
        assert_eq!(snap.capture_consistency, CaptureConsistency::Unverified);
    }
}
