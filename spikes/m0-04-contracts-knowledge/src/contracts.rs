//! Moteur expérimental de contrats, parseurs et réconciliation de claims
//! (Scénarios 2 & 3 du spike M0 n°4, `REQ-CONTRACT-004`, `REQ-CONTRACT-005`, `REQ-CONTRACT-006`, `REQ-CONTRACT-007`).

use std::collections::BTreeMap;

use unleash_domain::StreamId;
use unleash_protocol::ReasonCode;

use crate::errors::SpikeError;
use crate::types::{
    ActivationRef, CaptureConsistency, ClaimProposition, CommandSpec, ContractClaim, ExactRef,
    ExecutionResult, ExpectedExecutionBinding, ExpectedTestExecution, ParseState, ProcessState,
    SubjectRef, TestCasesSummary, TestExecutionObservation, ToolOrParserRef,
};

/// Résultat structuré d'un parseur de tests V1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTestReport {
    /// Identifiant et version du parseur appliqué.
    pub parser_ref: ToolOrParserRef,
    /// État de complétude du parsing.
    pub parse_state: ParseState,
    /// Compteurs normalisés si `Complete`, `None` sinon.
    pub cases: Option<TestCasesSummary>,
    /// Raison lisible si le parsing reste incomplet ou indisponible.
    pub diagnostic: Option<String>,
}

/// Analyse un rapport `JUnit` XML (`parser.junit-xml` v1) sans jamais inventer de cas.
///
/// Retourne `ParseState::Incomplete` (`cases: None`) si le XML est vide, tronqué,
/// dépourvu de balise `<testsuite>`/`<testsuites>` valide ou possède des compteurs incohérents.
#[must_use]
pub fn parse_junit_xml(xml: &str) -> ParsedTestReport {
    let parser_ref = ToolOrParserRef {
        id: String::from("parser.junit-xml"),
        version: 1,
    };

    let trimmed = xml.trim();
    if trimmed.is_empty() || !trimmed.starts_with('<') {
        return ParsedTestReport {
            parser_ref,
            parse_state: ParseState::Incomplete,
            cases: None,
            diagnostic: Some(String::from("Contenu XML vide ou non XML")),
        };
    }

    let has_closing = trimmed.contains("</testsuite>")
        || trimmed.contains("</testsuites>")
        || trimmed.contains("/>");
    if !has_closing {
        return ParsedTestReport {
            parser_ref,
            parse_state: ParseState::Incomplete,
            cases: None,
            diagnostic: Some(String::from("Document JUnit XML tronqué")),
        };
    }

    let suite_headers = extract_opening_tags(trimmed, "testsuite");
    let headers_to_sum = if suite_headers.is_empty() {
        extract_opening_tags(trimmed, "testsuites")
    } else {
        suite_headers
    };

    if headers_to_sum.is_empty() {
        return ParsedTestReport {
            parser_ref,
            parse_state: ParseState::Incomplete,
            cases: None,
            diagnostic: Some(String::from("Aucune balise <testsuite> trouvée")),
        };
    }

    let mut total_sum: u32 = 0;
    let mut failed_sum: u32 = 0;
    let mut skipped_sum: u32 = 0;

    for tag in headers_to_sum {
        let Some(tests) = parse_xml_u32_attr(&tag, "tests") else {
            return ParsedTestReport {
                parser_ref,
                parse_state: ParseState::Incomplete,
                cases: None,
                diagnostic: Some(String::from("Attribut 'tests' manquant ou invalide")),
            };
        };
        let failures = parse_xml_u32_attr(&tag, "failures").unwrap_or(0);
        let errors = parse_xml_u32_attr(&tag, "errors").unwrap_or(0);
        let skipped = parse_xml_u32_attr(&tag, "skipped").unwrap_or(0);

        let suite_failed = failures.saturating_add(errors);
        if suite_failed.saturating_add(skipped) > tests {
            return ParsedTestReport {
                parser_ref,
                parse_state: ParseState::Incomplete,
                cases: None,
                diagnostic: Some(String::from(
                    "Compteurs JUnit incohérents (failed + skipped > tests)",
                )),
            };
        }

        total_sum = total_sum.saturating_add(tests);
        failed_sum = failed_sum.saturating_add(suite_failed);
        skipped_sum = skipped_sum.saturating_add(skipped);
    }

    let passed = total_sum.saturating_sub(failed_sum.saturating_add(skipped_sum));

    ParsedTestReport {
        parser_ref,
        parse_state: ParseState::Complete,
        cases: Some(TestCasesSummary {
            total: total_sum,
            passed,
            failed: failed_sum,
            skipped: skipped_sum,
        }),
        diagnostic: None,
    }
}

/// Classe explicitement la sortie texte brute de `cargo test` comme `Incomplete` (`unknown`),
/// conformément à `contract-enforcement.md` (R7).
#[must_use]
pub fn parse_cargo_test_text(stdout: &str) -> ParsedTestReport {
    let detail = if stdout.trim().is_empty() {
        "Sortie texte cargo test vide : parsing incomplete"
    } else {
        "Sortie texte cargo test sans JUnit XML (cargo-nextest requis) : parsing incomplete"
    };
    ParsedTestReport {
        parser_ref: ToolOrParserRef {
            id: String::from("parser.cargo-test-text"),
            version: 1,
        },
        parse_state: ParseState::Incomplete,
        cases: None,
        diagnostic: Some(String::from(detail)),
    }
}

fn extract_opening_tags(xml: &str, tag_name: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let needle = format!("<{tag_name}");
    let mut search_from = 0usize;
    while let Some(slice) = xml.get(search_from..) {
        let Some(rel_idx) = slice.find(&needle) else {
            break;
        };
        let abs_start = search_from.saturating_add(rel_idx);
        let after_name = abs_start.saturating_add(needle.len());
        let Some(next_char) = xml.get(after_name..).and_then(|s| s.chars().next()) else {
            break;
        };
        if next_char.is_ascii_whitespace() || next_char == '>' || next_char == '/' {
            if let Some(rest) = xml.get(abs_start..)
                && let Some(end_rel) = rest.find('>')
                && let Some(tag) = rest.get(..=end_rel)
            {
                tags.push(tag.to_owned());
                search_from = abs_start.saturating_add(end_rel.saturating_add(1));
                continue;
            }
            break;
        }
        search_from = after_name;
    }
    tags
}

fn parse_xml_u32_attr(tag: &str, attr_name: &str) -> Option<u32> {
    for quote in ['"', '\''] {
        let pattern = format!("{attr_name}={quote}");
        if let Some(pos) = tag.find(&pattern) {
            let val_start = pos.saturating_add(pattern.len());
            if let Some(rest) = tag.get(val_start..)
                && let Some(end_pos) = rest.find(quote)
                && let Some(raw_num) = rest.get(..end_pos)
            {
                return raw_num.trim().parse::<u32>().ok();
            }
        }
    }
    None
}

/// Enregistrement autoritaire d'une observation (complète ou incomplète) rattachée à une action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationRecord {
    /// Projection V1 complète collectée par un adaptateur authentifié.
    Projected(Box<TestExecutionObservation>),
    /// Collecte incomplète : aucune projection V1 n'est fabriquée depuis l'attendu.
    IncompleteCollection {
        /// Identifiant d'action lié.
        action_id: String,
        /// Identifiant du binding lié.
        execution_binding_id: String,
        /// Composante manquante.
        missing_component: String,
    },
}

impl ObservationRecord {
    /// Construit un enregistrement de projection V1 authentifiée.
    #[must_use]
    pub fn projected(obs: TestExecutionObservation) -> Self {
        Self::Projected(Box::new(obs))
    }
}

/// Paramètres de scellement préalable d'une exécution attendue ([`SealedActionJournal::seal_binding`]).
#[derive(Debug, Clone)]
pub struct SealBindingRequest {
    /// Identifiant unique du binding.
    pub binding_id: String,
    /// Référence exacte du contrat.
    pub contract_ref: ExactRef,
    /// Règles partageant ce slot.
    pub rule_ids: Vec<String>,
    /// Slot littéral d'exécution.
    pub expectation_slot: String,
    /// Attendu autoritaire immuable.
    pub expectation: ExpectedTestExecution,
    /// Activation et révision.
    pub activation_ref: ActivationRef,
    /// Action réservée avant dispatch.
    pub action_id: String,
    /// Flux durable d'événements.
    pub stream_id: StreamId,
}

/// Journal autoritaire des attendus scellés avant effet et des observations collectées.
#[derive(Debug, Default)]
pub struct SealedActionJournal {
    expectations: BTreeMap<String, ExpectedTestExecution>,
    bindings: BTreeMap<String, ExpectedExecutionBinding>,
    observations_by_action: BTreeMap<String, Vec<ObservationRecord>>,
    next_sequence: u64,
}

impl SealedActionJournal {
    /// Crée un journal vide avec compteur de séquence initialisé à 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            expectations: BTreeMap::new(),
            bindings: BTreeMap::new(),
            observations_by_action: BTreeMap::new(),
            next_sequence: 1,
        }
    }

    /// Scelle un [`ExpectedTestExecution`] et émet son [`ExpectedExecutionBinding`] avant effet.
    ///
    /// # Errors
    /// Renvoie [`SpikeError::ContractInvariant`] si `rule_ids` est vide ou si un binding existe déjà,
    /// ou [`SpikeError::Protocol`] si la canonisation JCS échoue.
    pub fn seal_binding(
        &mut self,
        req: SealBindingRequest,
    ) -> Result<ExpectedExecutionBinding, SpikeError> {
        if req.rule_ids.is_empty() {
            return Err(SpikeError::ContractInvariant(String::from(
                "Un binding scellé exige au moins une règle",
            )));
        }
        if self.bindings.contains_key(&req.binding_id) {
            return Err(SpikeError::ContractInvariant(format!(
                "Binding '{}' déjà scellé : remplacement interdit",
                req.binding_id
            )));
        }

        let expectation_ref = req.expectation.exact_ref()?;
        let seq = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);

        let binding = ExpectedExecutionBinding {
            schema_version: String::from("unleash.expected-execution-binding.v1"),
            binding_id: req.binding_id.clone(),
            contract_ref: req.contract_ref,
            rule_ids: req.rule_ids,
            expectation_slot: req.expectation_slot,
            expectation_ref,
            subject: req.expectation.subject.clone(),
            activation_ref: req.activation_ref,
            action_id: req.action_id,
            sealed_cursor: crate::types::EventCursorRef {
                stream_id: req.stream_id,
                sequence: seq,
            },
        };

        self.expectations
            .insert(req.expectation.expectation_id.clone(), req.expectation);
        self.bindings.insert(req.binding_id, binding.clone());
        Ok(binding)
    }

    /// Enregistre une observation authentifiée pour un `action_id` déjà scellé.
    ///
    /// # Errors
    /// Renvoie [`SpikeError::ContractInvariant`] si l'action n'est rattachée à aucun binding scellé.
    pub fn record_observation(&mut self, obs: TestExecutionObservation) -> Result<(), SpikeError> {
        let binding = self
            .bindings
            .get(&obs.execution_binding_id)
            .ok_or_else(|| {
                SpikeError::ContractInvariant(format!(
                    "Binding '{}' introuvable lors de l'enregistrement d'observation",
                    obs.execution_binding_id
                ))
            })?;
        if binding.action_id != obs.action_id {
            return Err(SpikeError::ContractInvariant(format!(
                "action_id '{}' incohérent avec le binding scellé '{}'",
                obs.action_id, binding.action_id
            )));
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.observations_by_action
            .entry(obs.action_id.clone())
            .or_default()
            .push(ObservationRecord::projected(obs));
        Ok(())
    }

    /// Enregistre une collecte incomplète sans fabriquer de projection V1.
    ///
    /// # Errors
    /// Renvoie [`SpikeError::ContractInvariant`] si le binding ou l'action n'existe pas.
    pub fn record_incomplete_collection(
        &mut self,
        action_id: impl Into<String>,
        execution_binding_id: impl Into<String>,
        missing_component: impl Into<String>,
    ) -> Result<(), SpikeError> {
        let action_id = action_id.into();
        let execution_binding_id = execution_binding_id.into();
        let binding = self.bindings.get(&execution_binding_id).ok_or_else(|| {
            SpikeError::ContractInvariant(format!("Binding '{execution_binding_id}' introuvable"))
        })?;
        if binding.action_id != action_id {
            return Err(SpikeError::ContractInvariant(format!(
                "action_id '{action_id}' incohérent avec le binding scellé"
            )));
        }
        self.observations_by_action
            .entry(action_id.clone())
            .or_default()
            .push(ObservationRecord::IncompleteCollection {
                action_id,
                execution_binding_id,
                missing_component: missing_component.into(),
            });
        Ok(())
    }

    /// Renvoie toutes les observations enregistrées au journal pour une action donnée,
    /// indépendamment des omissions éventuelles d'un `Claim`.
    #[must_use]
    pub fn observations_for_action(&self, action_id: &str) -> &[ObservationRecord] {
        self.observations_by_action
            .get(action_id)
            .map_or(&[], Vec::as_slice)
    }
}

/// Issue d'une règle contractuelle (`contract-enforcement.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleOutcome {
    /// Règle satisfaite dans le domaine borné.
    Pass,
    /// Règle explicitement violée.
    Fail,
    /// Preuve manquante, incomplète ou capture non cohérente (jamais `Pass` par défaut).
    Unknown,
}

/// Politique sur `unknown` pour une règle obligatoire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnUnknownPolicy {
    /// Bloque l'étape (`ContractVerdictStatus::Unknown`).
    Block,
    /// Fait échouer immédiatement (`ContractVerdictStatus::Fail`).
    Fail,
}

/// Résultat d'évaluation d'une règle individuelle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleVerdict {
    /// Identifiant de la règle.
    pub rule_id: String,
    /// Identifiant du validateur allowlisté exécuté.
    pub validator_id: String,
    /// Règle obligatoire ou informative.
    pub mandatory: bool,
    /// Issue (`Pass`, `Fail`, `Unknown`).
    pub outcome: RuleOutcome,
    /// Raison typée normative en cas de `Fail` ou `Unknown`.
    pub reason: Option<ReasonCode>,
    /// Détail diagnostic inspectable.
    pub detail: String,
}

/// Statut agrégé d'un `ContractVerdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerdictStatus {
    /// Toutes les règles obligatoires sont `Pass`.
    Pass,
    /// Au moins une règle obligatoire a échoué (ou est `Unknown` sous politique `Fail`).
    Fail,
    /// Aucune règle obligatoire n'a échoué, mais au moins une est `Unknown` (`Block`).
    Unknown,
}

/// Verdict contractuel global produit à la clôture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractVerdict {
    /// Statut global (`Fail > Unknown > Pass`).
    pub status: VerdictStatus,
    /// Résultats détaillés par règle.
    pub rules: Vec<RuleVerdict>,
}

/// Agrège les verdicts individuels selon la règle canonique (`ADR-07`).
#[must_use]
pub fn evaluate_contract_verdict(rules: Vec<(RuleVerdict, OnUnknownPolicy)>) -> ContractVerdict {
    let mut has_fail = false;
    let mut has_unknown = false;
    let mut collected = Vec::with_capacity(rules.len());

    for (verdict, on_unknown) in rules {
        if verdict.mandatory {
            match verdict.outcome {
                RuleOutcome::Pass => {}
                RuleOutcome::Fail => has_fail = true,
                RuleOutcome::Unknown => match on_unknown {
                    OnUnknownPolicy::Block => has_unknown = true,
                    OnUnknownPolicy::Fail => has_fail = true,
                },
            }
        }
        collected.push(verdict);
    }

    let status = if has_fail {
        VerdictStatus::Fail
    } else if has_unknown {
        VerdictStatus::Unknown
    } else {
        VerdictStatus::Pass
    };

    ContractVerdict {
        status,
        rules: collected,
    }
}

/// Validateur `git_snapshot_matches` : retourne `Unknown` si la capture est `inconsistent` ou `unverified`.
#[must_use]
pub fn validate_git_snapshot_matches(
    rule_id: &str,
    mandatory: bool,
    expected: &SubjectRef,
    observed_source_ref: &crate::types::TypedSourceRef,
) -> RuleVerdict {
    match observed_source_ref.capture_consistency() {
        CaptureConsistency::Inconsistent => RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("git_snapshot_matches"),
            mandatory,
            outcome: RuleOutcome::Unknown,
            reason: Some(ReasonCode::SnapshotInconsistent),
            detail: String::from(
                "Capture Git/worktree incohérente (mutation détectée pendant capture)",
            ),
        },
        CaptureConsistency::Unverified => RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("git_snapshot_matches"),
            mandatory,
            outcome: RuleOutcome::Unknown,
            reason: Some(ReasonCode::EvidenceMissing),
            detail: String::from(
                "Capture séquentielle non atomique (unverified) : ne vaut pas preuve cohérente",
            ),
        },
        CaptureConsistency::GitTreeCoherent
        | CaptureConsistency::AtomicFsCoherent
        | CaptureConsistency::ImmutableVersionCoherent => {
            if expected.snapshot_ref.id == observed_source_ref.version_id()
                && expected.snapshot_ref.sha256.as_hex() == observed_source_ref.content_hex()
            {
                RuleVerdict {
                    rule_id: rule_id.to_owned(),
                    validator_id: String::from("git_snapshot_matches"),
                    mandatory,
                    outcome: RuleOutcome::Pass,
                    reason: None,
                    detail: String::from("Identité de snapshot cohérente vérifiée"),
                }
            } else {
                RuleVerdict {
                    rule_id: rule_id.to_owned(),
                    validator_id: String::from("git_snapshot_matches"),
                    mandatory,
                    outcome: RuleOutcome::Fail,
                    reason: Some(ReasonCode::EvidenceSnapshotMismatch),
                    detail: String::from(
                        "Divergence de version ou de digest entre snapshot attendu et observé",
                    ),
                }
            }
        }
    }
}

/// Validateur `test_execution_matches` (catalogue v2).
///
/// # Errors
/// Renvoie [`SpikeError::Protocol`] si le calcul de digest de `expected` échoue.
pub fn validate_test_execution_matches(
    rule_id: &str,
    mandatory: bool,
    expected: &ExpectedTestExecution,
    binding: &ExpectedExecutionBinding,
    records: &[ObservationRecord],
    observed_capture_consistency: CaptureConsistency,
) -> Result<RuleVerdict, SpikeError> {
    if let Some(consistency_verdict) = check_capture_consistency(
        rule_id,
        "test_execution_matches",
        mandatory,
        observed_capture_consistency,
    ) {
        return Ok(consistency_verdict);
    }

    let expected_ref = expected.exact_ref()?;
    if binding.expectation_ref != expected_ref || binding.subject != expected.subject {
        return Ok(RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("test_execution_matches"),
            mandatory,
            outcome: RuleOutcome::Fail,
            reason: Some(ReasonCode::EvidenceExecutionMismatch),
            detail: String::from("Le binding scellé ne correspond pas à l'attendu autoritaire"),
        });
    }

    let obs = match select_single_projected_observation(
        rule_id,
        "test_execution_matches",
        mandatory,
        records,
    ) {
        None => {
            return Ok(RuleVerdict {
                rule_id: rule_id.to_owned(),
                validator_id: String::from("test_execution_matches"),
                mandatory,
                outcome: RuleOutcome::Unknown,
                reason: Some(ReasonCode::EvidenceMissing),
                detail: String::from("Aucune projection d'observation V1 complète disponible"),
            });
        }
        Some(Err(ambiguous_verdict)) => return Ok(ambiguous_verdict),
        Some(Ok(obs)) => obs,
    };

    if let Some(mismatch) = check_observation_identity_against_expected(
        rule_id,
        "test_execution_matches",
        mandatory,
        expected,
        &expected_ref,
        binding,
        obs,
    ) {
        return Ok(mismatch);
    }

    if let Some(invalid_proj) =
        check_result_counters_coherent(rule_id, "test_execution_matches", mandatory, &obs.result)
    {
        return Ok(invalid_proj);
    }

    let outcome = evaluate_execution_outcome(expected, &obs.result);
    let (reason, detail) = match outcome {
        RuleOutcome::Pass => (
            None,
            String::from("Exécution, snapshot, parseur complet et compteurs conformes à l'attendu"),
        ),
        RuleOutcome::Fail => (
            Some(ReasonCode::ContractRejected),
            String::from("Résultat de test observé non conforme à la politique de l'attendu"),
        ),
        RuleOutcome::Unknown => (
            Some(ReasonCode::EvidenceMissing),
            String::from("Parsing incomplet ou issue de processus inconnue : verdict unknown"),
        ),
    };

    Ok(RuleVerdict {
        rule_id: rule_id.to_owned(),
        validator_id: String::from("test_execution_matches"),
        mandatory,
        outcome,
        reason,
        detail,
    })
}

/// Trace complète d'une réconciliation de claim conservant claim ET observations liées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReconciliationRecord {
    /// Claim évalué (conservé intact, même contredit).
    pub claim: ContractClaim,
    /// Toutes les observations autoritaires consultées pour l'action liée (jamais supprimées).
    pub linked_observations: Vec<ObservationRecord>,
    /// Verdict produit par `claim_predicate_matches`.
    pub verdict: RuleVerdict,
}

/// Validateur `claim_predicate_matches` (catalogue v2).
///
/// Consulte toutes les observations liées au `binding.action_id` dans le journal autoritaire,
/// même lorsque `claim.observation_ids` omet les échecs.
///
/// # Errors
/// Renvoie [`SpikeError::Protocol`] si la canonisation de `expected` échoue.
pub fn validate_claim_predicate_matches(
    rule_id: &str,
    mandatory: bool,
    claim: &ContractClaim,
    expected: &ExpectedTestExecution,
    binding: &ExpectedExecutionBinding,
    journal_records: &[ObservationRecord],
    observed_capture_consistency: CaptureConsistency,
) -> Result<ClaimReconciliationRecord, SpikeError> {
    let expected_ref = expected.exact_ref()?;
    let verdict = match &claim.proposition {
        ClaimProposition::Qualitative => RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("claim_predicate_matches"),
            mandatory,
            outcome: RuleOutcome::Unknown,
            reason: Some(ReasonCode::EvidenceMissing),
            detail: String::from(
                "Proposition qualitative : vérité observable unknown, statement ignoré",
            ),
        },
        ClaimProposition::ObservableTestExpectationSatisfied {
            expectation_ref,
            execution_binding_id,
            asserted_value,
        } => match prepare_observable_claim_check(
            rule_id,
            mandatory,
            claim,
            expected,
            &expected_ref,
            binding,
            expectation_ref,
            execution_binding_id,
            journal_records,
            observed_capture_consistency,
        ) {
            Ok(obs) => evaluate_expectation_satisfied_claim(
                rule_id,
                mandatory,
                expected,
                obs,
                *asserted_value,
            ),
            Err(early) => early,
        },
        ClaimProposition::ObservableProcessExitCode {
            expectation_ref,
            execution_binding_id,
            asserted_value,
        } => match prepare_observable_claim_check(
            rule_id,
            mandatory,
            claim,
            expected,
            &expected_ref,
            binding,
            expectation_ref,
            execution_binding_id,
            journal_records,
            observed_capture_consistency,
        ) {
            Ok(obs) => evaluate_process_exit_code_claim(rule_id, mandatory, obs, *asserted_value),
            Err(early) => early,
        },
    };

    Ok(ClaimReconciliationRecord {
        claim: claim.clone(),
        linked_observations: journal_records.to_vec(),
        verdict,
    })
}

fn evaluate_expectation_satisfied_claim(
    rule_id: &str,
    mandatory: bool,
    expected: &ExpectedTestExecution,
    obs: &TestExecutionObservation,
    asserted_value: bool,
) -> RuleVerdict {
    if let Some(invalid_proj) =
        check_result_counters_coherent(rule_id, "claim_predicate_matches", mandatory, &obs.result)
    {
        return invalid_proj;
    }

    let actual_outcome = evaluate_execution_outcome(expected, &obs.result);
    match actual_outcome {
        RuleOutcome::Unknown => RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("claim_predicate_matches"),
            mandatory,
            outcome: RuleOutcome::Unknown,
            reason: Some(ReasonCode::EvidenceMissing),
            detail: String::from("Projection déterministe inconnue (parsing/processus incomplet)"),
        },
        RuleOutcome::Pass | RuleOutcome::Fail => {
            let actual_bool = actual_outcome == RuleOutcome::Pass;
            if actual_bool == asserted_value {
                RuleVerdict {
                    rule_id: rule_id.to_owned(),
                    validator_id: String::from("claim_predicate_matches"),
                    mandatory,
                    outcome: RuleOutcome::Pass,
                    reason: None,
                    detail: format!(
                        "Prédicat test.expectation_satisfied={actual_bool} corroboré par l'observation scellée"
                    ),
                }
            } else {
                RuleVerdict {
                    rule_id: rule_id.to_owned(),
                    validator_id: String::from("claim_predicate_matches"),
                    mandatory,
                    outcome: RuleOutcome::Fail,
                    reason: Some(ReasonCode::ClaimContradicted),
                    detail: format!(
                        "Claim contredit : affirmé={asserted_value}, observé={actual_bool} (claim et observation conservés)"
                    ),
                }
            }
        }
    }
}

fn evaluate_process_exit_code_claim(
    rule_id: &str,
    mandatory: bool,
    obs: &TestExecutionObservation,
    asserted_value: i32,
) -> RuleVerdict {
    match (obs.result.process_state, obs.result.exit_code) {
        (ProcessState::Exited, Some(code)) if code == asserted_value => RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("claim_predicate_matches"),
            mandatory,
            outcome: RuleOutcome::Pass,
            reason: None,
            detail: format!("process.exit_code={code} corroboré"),
        },
        (ProcessState::Exited, Some(code)) => RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("claim_predicate_matches"),
            mandatory,
            outcome: RuleOutcome::Fail,
            reason: Some(ReasonCode::ClaimContradicted),
            detail: format!("Claim contredit : exit_code affirmé={asserted_value}, observé={code}"),
        },
        _ => RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("claim_predicate_matches"),
            mandatory,
            outcome: RuleOutcome::Unknown,
            reason: Some(ReasonCode::EvidenceMissing),
            detail: String::from("process.exit_code indisponible (processus non exited)"),
        },
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Facteur commun de préparation et de vérification d'identité d'un claim observable"
)]
fn prepare_observable_claim_check<'a>(
    rule_id: &str,
    mandatory: bool,
    claim: &ContractClaim,
    expected: &ExpectedTestExecution,
    expected_ref: &ExactRef,
    binding: &ExpectedExecutionBinding,
    claim_expectation_ref: &ExactRef,
    claim_binding_id: &str,
    journal_records: &'a [ObservationRecord],
    observed_capture_consistency: CaptureConsistency,
) -> Result<&'a TestExecutionObservation, RuleVerdict> {
    if let Some(mismatch) = verify_claim_binding_links(
        rule_id,
        mandatory,
        claim,
        expected,
        expected_ref,
        binding,
        claim_expectation_ref,
        claim_binding_id,
    ) {
        return Err(mismatch);
    }

    if let Some(consistency_verdict) = check_capture_consistency(
        rule_id,
        "claim_predicate_matches",
        mandatory,
        observed_capture_consistency,
    ) {
        return Err(consistency_verdict);
    }

    let obs = match select_single_projected_observation(
        rule_id,
        "claim_predicate_matches",
        mandatory,
        journal_records,
    ) {
        None => {
            return Err(RuleVerdict {
                rule_id: rule_id.to_owned(),
                validator_id: String::from("claim_predicate_matches"),
                mandatory,
                outcome: RuleOutcome::Unknown,
                reason: Some(ReasonCode::EvidenceMissing),
                detail: String::from("Aucune observation complète liée à l'action"),
            });
        }
        Some(Err(early)) => return Err(early),
        Some(Ok(o)) => o,
    };

    if let Some(mismatch) = check_observation_identity_against_expected(
        rule_id,
        "claim_predicate_matches",
        mandatory,
        expected,
        expected_ref,
        binding,
        obs,
    ) {
        return Err(mismatch);
    }

    Ok(obs)
}

fn check_capture_consistency(
    rule_id: &str,
    validator_id: &str,
    mandatory: bool,
    consistency: CaptureConsistency,
) -> Option<RuleVerdict> {
    match consistency {
        CaptureConsistency::Inconsistent => Some(RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: validator_id.to_owned(),
            mandatory,
            outcome: RuleOutcome::Unknown,
            reason: Some(ReasonCode::SnapshotInconsistent),
            detail: String::from(
                "Capture du sujet incohérente pendant l'analyse : verdict unknown",
            ),
        }),
        CaptureConsistency::Unverified => Some(RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: validator_id.to_owned(),
            mandatory,
            outcome: RuleOutcome::Unknown,
            reason: Some(ReasonCode::EvidenceMissing),
            detail: String::from("Capture du sujet non atomique (unverified) : verdict unknown"),
        }),
        CaptureConsistency::GitTreeCoherent
        | CaptureConsistency::AtomicFsCoherent
        | CaptureConsistency::ImmutableVersionCoherent => None,
    }
}

fn select_single_projected_observation<'a>(
    rule_id: &str,
    validator_id: &str,
    mandatory: bool,
    records: &'a [ObservationRecord],
) -> Option<Result<&'a TestExecutionObservation, RuleVerdict>> {
    if records.is_empty() {
        return None;
    }
    let projected: Vec<&TestExecutionObservation> = records
        .iter()
        .filter_map(|r| match r {
            ObservationRecord::Projected(obs) => Some(obs.as_ref()),
            ObservationRecord::IncompleteCollection { .. } => None,
        })
        .collect();

    if projected.len() > 1 {
        return Some(Err(RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: validator_id.to_owned(),
            mandatory,
            outcome: RuleOutcome::Unknown,
            reason: Some(ReasonCode::EvidenceAmbiguous),
            detail: String::from("Plusieurs observations concurrentes pour la même action"),
        }));
    }

    projected.into_iter().next().map(Ok)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Vérification explicite des 5 composantes de liaison claim/attendu/binding/sujet"
)]
fn verify_claim_binding_links(
    rule_id: &str,
    mandatory: bool,
    claim: &ContractClaim,
    expected: &ExpectedTestExecution,
    expected_ref: &ExactRef,
    binding: &ExpectedExecutionBinding,
    claim_expectation_ref: &ExactRef,
    claim_binding_id: &str,
) -> Option<RuleVerdict> {
    if binding.expectation_ref != *expected_ref
        || *claim_expectation_ref != binding.expectation_ref
        || claim_binding_id != binding.binding_id
        || claim.subject != binding.subject
        || binding.subject != expected.subject
    {
        return Some(RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: String::from("claim_predicate_matches"),
            mandatory,
            outcome: RuleOutcome::Fail,
            reason: Some(ReasonCode::EvidenceExecutionMismatch),
            detail: String::from(
                "Divergence de sujet, binding ou référence d'attendu sur le claim (vérité reste unknown)",
            ),
        });
    }
    None
}

fn check_observation_identity_against_expected(
    rule_id: &str,
    validator_id: &str,
    mandatory: bool,
    expected: &ExpectedTestExecution,
    expected_ref: &ExactRef,
    binding: &ExpectedExecutionBinding,
    obs: &TestExecutionObservation,
) -> Option<RuleVerdict> {
    if obs.subject != expected.subject {
        return Some(RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: validator_id.to_owned(),
            mandatory,
            outcome: RuleOutcome::Fail,
            reason: Some(ReasonCode::EvidenceSnapshotMismatch),
            detail: format!(
                "Snapshot de preuve divergent : attendu='{}', observé='{}'",
                expected.subject.snapshot_ref.id, obs.subject.snapshot_ref.id
            ),
        });
    }

    if obs.action_id != binding.action_id
        || obs.execution_binding_id != binding.binding_id
        || obs.expectation_ref != *expected_ref
        || !commands_match(&expected.command, &obs.observed_command)
    {
        return Some(RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: validator_id.to_owned(),
            mandatory,
            outcome: RuleOutcome::Fail,
            reason: Some(ReasonCode::EvidenceExecutionMismatch),
            detail: String::from("Divergence d'action, binding, attendu ou commande observée"),
        });
    }

    None
}

fn commands_match(expected: &CommandSpec, observed: &CommandSpec) -> bool {
    expected == observed
}

fn check_result_counters_coherent(
    rule_id: &str,
    validator_id: &str,
    mandatory: bool,
    result: &ExecutionResult,
) -> Option<RuleVerdict> {
    match (result.parse_state, &result.cases) {
        (ParseState::Complete, Some(cases)) => {
            if cases.total
                != cases
                    .passed
                    .saturating_add(cases.failed)
                    .saturating_add(cases.skipped)
            {
                return Some(RuleVerdict {
                    rule_id: rule_id.to_owned(),
                    validator_id: validator_id.to_owned(),
                    mandatory,
                    outcome: RuleOutcome::Fail,
                    reason: Some(ReasonCode::EvidenceProjectionInvalid),
                    detail: String::from(
                        "Compteurs de cas incohérents : total != passed + failed + skipped",
                    ),
                });
            }
        }
        (ParseState::Complete, None)
        | (ParseState::Incomplete | ParseState::Unavailable, Some(_)) => {
            return Some(RuleVerdict {
                rule_id: rule_id.to_owned(),
                validator_id: validator_id.to_owned(),
                mandatory,
                outcome: RuleOutcome::Fail,
                reason: Some(ReasonCode::EvidenceProjectionInvalid),
                detail: String::from("Combinaison parse_state/cases invalide dans l'observation"),
            });
        }
        (ParseState::Incomplete | ParseState::Unavailable, None) => {}
    }

    match (result.process_state, result.exit_code) {
        (ProcessState::Exited, Some(_))
        | (ProcessState::Terminated | ProcessState::Unknown, None) => None,
        _ => Some(RuleVerdict {
            rule_id: rule_id.to_owned(),
            validator_id: validator_id.to_owned(),
            mandatory,
            outcome: RuleOutcome::Fail,
            reason: Some(ReasonCode::EvidenceProjectionInvalid),
            detail: String::from("Combinaison process_state/exit_code invalide dans l'observation"),
        }),
    }
}

fn evaluate_execution_outcome(
    expected: &ExpectedTestExecution,
    result: &ExecutionResult,
) -> RuleOutcome {
    let process_cond = match (result.process_state, result.exit_code) {
        (ProcessState::Exited, Some(code)) if code == expected.result_policy.expected_exit_code => {
            RuleOutcome::Pass
        }
        (ProcessState::Exited, Some(_)) | (ProcessState::Terminated, _) => RuleOutcome::Fail,
        (ProcessState::Unknown, _) | (ProcessState::Exited, None) => RuleOutcome::Unknown,
    };

    let parse_cond = match result.parse_state {
        ParseState::Complete => RuleOutcome::Pass,
        ParseState::Incomplete | ParseState::Unavailable => RuleOutcome::Unknown,
    };

    let cases_cond = match (result.parse_state, &result.cases) {
        (ParseState::Complete, Some(cases)) => {
            let min_ok = cases.total >= expected.result_policy.min_cases;
            let all_ok = !expected.result_policy.require_all_passed
                || (cases.passed == cases.total && cases.failed == 0 && cases.skipped == 0);
            if min_ok && all_ok {
                RuleOutcome::Pass
            } else {
                RuleOutcome::Fail
            }
        }
        _ => RuleOutcome::Unknown,
    };

    if matches!(process_cond, RuleOutcome::Fail)
        || matches!(parse_cond, RuleOutcome::Fail)
        || matches!(cases_cond, RuleOutcome::Fail)
    {
        RuleOutcome::Fail
    } else if matches!(process_cond, RuleOutcome::Unknown)
        || matches!(parse_cond, RuleOutcome::Unknown)
        || matches!(cases_cond, RuleOutcome::Unknown)
    {
        RuleOutcome::Unknown
    } else {
        RuleOutcome::Pass
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn junit_parser_aggregates_multiple_suites_and_rejects_inconsistent_counts() {
        let multi = r#"
<testsuites>
  <testsuite name="a" tests="3" failures="1" errors="0" skipped="1"/>
  <testsuite name="b" tests="2" failures="0" errors="1" skipped="0"/>
</testsuites>"#;
        let parsed = parse_junit_xml(multi);
        assert_eq!(parsed.parse_state, ParseState::Complete);
        assert_eq!(
            parsed.cases,
            Some(TestCasesSummary {
                total: 5,
                passed: 2,
                failed: 2,
                skipped: 1,
            })
        );

        let impossible = r#"<testsuite name="bad" tests="1" failures="2" errors="0" skipped="0"/>"#;
        let bad = parse_junit_xml(impossible);
        assert_eq!(bad.parse_state, ParseState::Incomplete);
        assert_eq!(bad.cases, None);

        let empty_cargo = parse_cargo_test_text("");
        assert_eq!(empty_cargo.parse_state, ParseState::Incomplete);
        assert_eq!(empty_cargo.cases, None);
    }

    #[test]
    fn informative_failing_rule_does_not_fail_global_verdict() {
        let verdict = evaluate_contract_verdict(vec![
            (
                RuleVerdict {
                    rule_id: String::from("req"),
                    validator_id: String::from("test_execution_matches"),
                    mandatory: true,
                    outcome: RuleOutcome::Pass,
                    reason: None,
                    detail: String::from("ok"),
                },
                OnUnknownPolicy::Block,
            ),
            (
                RuleVerdict {
                    rule_id: String::from("info"),
                    validator_id: String::from("claim_predicate_matches"),
                    mandatory: false,
                    outcome: RuleOutcome::Fail,
                    reason: Some(ReasonCode::ClaimContradicted),
                    detail: String::from("info fail"),
                },
                OnUnknownPolicy::Block,
            ),
        ]);
        assert_eq!(verdict.status, VerdictStatus::Pass);
        assert_eq!(verdict.rules.len(), 2);
    }
}
