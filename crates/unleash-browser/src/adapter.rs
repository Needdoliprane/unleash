//! Adaptateur `WebDriver` W3C (`FantocciniBrowserAdapter`) implémentant [`BrowserPort`].
//!
//! Pilote un driver `WebDriver` W3C (typiquement `chromedriver` appairé à `Chrome for Testing`)
//! via la crate `fantoccini` sur loopback, sans TLS fictif et sans téléchargement implicite.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, Permissions};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fantoccini::error::CmdError;
use fantoccini::wd::{Capabilities, TimeoutConfiguration};
use fantoccini::{Client, ClientBuilder, Locator};
use hyper_util::client::legacy::connect::HttpConnector;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use unleash_domain::{
    ClockPort, IdGeneratorPort, ModelPolicy, SessionId, Sha256Digest, TimestampMs,
};
use url::Url;
use uuid::Uuid;

use crate::egress::{EgressPolicy, normalize_origin};
use crate::error::BrowserError;
use crate::locator::{
    LocatorStrategy, ResolvedTarget, TargetSpec, enforce_unique_candidate, escape_css_attr_value,
    text_matches, verify_target_identity,
};
use crate::policy::{ModelAttemptKind, ModelPolicyEnforcer, check_execution_isolation};
use crate::port::{BoxFuture, BrowserPort};
use crate::runtime::{
    BrowserEngineKind, PinnedBrowserRuntime, RuntimeInspection, verify_pinned_runtime,
};
use crate::types::{
    ActionOutcome, AssertionEvaluationReport, AssertionRuleResult, AssertionVerdict,
    BrowserActionIntent, BrowserActionKind, BrowserCheckpoint, BrowserContextHandle,
    BusinessAssertion, CheckpointInput, CloseContextResult, CloseOutcome, CloseReason,
    EffectJournalPort, EffectState, InMemoryEffectJournal, ObserveScope, OpenContextRequest,
    PageObservation,
};
use crate::webdriver_ext::{GetComputedLabel, GetComputedRole, extract_w3c_optional_string};

#[derive(Debug, Clone)]
struct SessionEntry {
    client: Client,
    handle: BrowserContextHandle,
    egress_policy: EgressPolicy,
    page_generation: u64,
}

/// Horloge système par défaut utilisée hors tests déterministes.
#[derive(Debug, Default)]
pub struct SystemClockPort;

impl ClockPort for SystemClockPort {
    fn now_ms(&self) -> TimestampMs {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0_i64, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        TimestampMs::new(millis)
    }
}

/// Générateur d'identifiants séquentiel par défaut lorsque aucun `IdGeneratorPort` externe n'est injecté.
#[derive(Debug, Default)]
pub struct MonotonicIdPort {
    counter: AtomicU64,
}

impl IdGeneratorPort for MonotonicIdPort {
    fn new_uuid(&self) -> Uuid {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        Uuid::from_u128(0xB000_0000_0000_4000_8000_0000_0000_0000_u128 | u128::from(seq))
    }
}

/// Adaptateur Rust de [`BrowserPort`] reposant sur `fantoccini` et le protocole `WebDriver` W3C.
pub struct FantocciniBrowserAdapter {
    clock: Arc<dyn ClockPort>,
    id_gen: Arc<dyn IdGeneratorPort>,
    effect_journal: Arc<dyn EffectJournalPort>,
    model_guard: Arc<ModelPolicyEnforcer>,
    sessions: Arc<Mutex<HashMap<SessionId, SessionEntry>>>,
    next_context_generation: AtomicU64,
}

impl std::fmt::Debug for FantocciniBrowserAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FantocciniBrowserAdapter")
            .field(
                "next_context_generation",
                &self.next_context_generation.load(Ordering::SeqCst),
            )
            .finish_non_exhaustive()
    }
}

impl Default for FantocciniBrowserAdapter {
    fn default() -> Self {
        Self::new(
            Arc::new(SystemClockPort),
            Arc::new(MonotonicIdPort::default()),
            Arc::new(InMemoryEffectJournal::new()),
            Arc::new(ModelPolicyEnforcer::new()),
        )
    }
}

impl FantocciniBrowserAdapter {
    /// Construit l'adaptateur en injectant horloge, générateur d'IDs, journal d'effets et garde modèle.
    #[must_use]
    pub fn new(
        clock: Arc<dyn ClockPort>,
        id_gen: Arc<dyn IdGeneratorPort>,
        effect_journal: Arc<dyn EffectJournalPort>,
        model_guard: Arc<ModelPolicyEnforcer>,
    ) -> Self {
        Self {
            clock,
            id_gen,
            effect_journal,
            model_guard,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_context_generation: AtomicU64::new(1),
        }
    }

    /// Retourne une référence vers le journal d'effets utilisé par l'adaptateur.
    #[must_use]
    pub fn effect_journal(&self) -> &Arc<dyn EffectJournalPort> {
        &self.effect_journal
    }

    /// Retourne une référence vers le garde anti-modèle utilisé par l'adaptateur.
    #[must_use]
    pub fn model_guard(&self) -> &Arc<ModelPolicyEnforcer> {
        &self.model_guard
    }

    fn get_session_entry(
        &self,
        handle: &BrowserContextHandle,
    ) -> Result<SessionEntry, BrowserError> {
        let guard = self
            .sessions
            .lock()
            .map_err(|_| BrowserError::InvalidConfiguration {
                reason: "Verrou des sessions navigateur corrompu".into(),
            })?;
        let entry =
            guard
                .get(&handle.session_id)
                .ok_or_else(|| BrowserError::StaleFenceOrContext {
                    reason: format!("Session navigateur {} absente ou fermée", handle.session_id),
                })?;
        if entry.handle.context_generation != handle.context_generation
            || entry.handle.fence != handle.fence
        {
            return Err(BrowserError::StaleFenceOrContext {
                reason: "Génération de contexte ou fence d'autorité divergent".into(),
            });
        }
        Ok(entry.clone())
    }

    fn bump_page_generation(&self, session_id: SessionId) -> Result<u64, BrowserError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| BrowserError::InvalidConfiguration {
                reason: "Verrou des sessions navigateur corrompu".into(),
            })?;
        let entry =
            guard
                .get_mut(&session_id)
                .ok_or_else(|| BrowserError::StaleFenceOrContext {
                    reason: format!("Session navigateur {session_id} absente"),
                })?;
        entry.page_generation = entry.page_generation.saturating_add(1);
        Ok(entry.page_generation)
    }

    fn remove_session_entry(
        &self,
        handle: &BrowserContextHandle,
    ) -> Result<SessionEntry, BrowserError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| BrowserError::InvalidConfiguration {
                reason: "Verrou des sessions navigateur corrompu".into(),
            })?;
        let existing =
            guard
                .get(&handle.session_id)
                .ok_or_else(|| BrowserError::StaleFenceOrContext {
                    reason: format!("Session navigateur {} déjà fermée", handle.session_id),
                })?;
        if existing.handle.context_generation != handle.context_generation {
            return Err(BrowserError::StaleFenceOrContext {
                reason: "Génération de contexte divergente à la fermeture".into(),
            });
        }
        guard
            .remove(&handle.session_id)
            .ok_or_else(|| BrowserError::StaleFenceOrContext {
                reason: "Session introuvable lors de la suppression".into(),
            })
    }
}

impl BrowserPort for FantocciniBrowserAdapter {
    fn inspect_runtime<'a>(
        &'a self,
        runtime_ref: &'a PinnedBrowserRuntime,
    ) -> BoxFuture<'a, Result<RuntimeInspection, BrowserError>> {
        let runtime = runtime_ref.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                verify_pinned_runtime(&runtime, crate::policy::ExecutionMode::Mediated)
            })
            .await
            .map_err(|err| BrowserError::RuntimeMismatch {
                details_ref: "runtime_inspection_join".into(),
                message: format!("Tâche d'inspection runtime interrompue : {err}"),
            })?
        })
    }

    fn open_context(
        &self,
        request: OpenContextRequest,
    ) -> BoxFuture<'_, Result<BrowserContextHandle, BrowserError>> {
        Box::pin(async move {
            check_execution_isolation(
                request.execution_mode,
                request.model_policy,
                request.isolation_level,
                &request.confinement_proof,
            )?;

            validate_loopback_webdriver_url(&request.webdriver_endpoint)?;

            let runtime_clone = request.runtime.clone();
            let mode = request.execution_mode;
            tokio::task::spawn_blocking(move || verify_pinned_runtime(&runtime_clone, mode))
                .await
                .map_err(|err| BrowserError::RuntimeMismatch {
                    details_ref: "runtime_open_join".into(),
                    message: format!("Tâche de vérification runtime interrompue : {err}"),
                })??;

            ensure_isolated_user_data_dir(&request.user_data_dir)?;

            let caps = build_webdriver_capabilities(&request)?;
            let connect_timeout = request.page_load_timeout.max(Duration::from_secs(5));
            let connector = HttpConnector::new();
            let mut builder = ClientBuilder::new(connector);
            builder.capabilities(caps);

            let client = tokio::time::timeout(
                connect_timeout,
                builder.connect(request.webdriver_endpoint.as_str()),
            )
            .await
            .map_err(|_| BrowserError::ActionabilityTimeout {
                message: format!(
                    "Timeout lors de l'ouverture de session WebDriver sur {}",
                    request.webdriver_endpoint
                ),
            })?
            .map_err(|err| BrowserError::RuntimeMismatch {
                details_ref: "webdriver_new_session".into(),
                message: format!("Échec de création de session WebDriver : {err}"),
            })?;

            let timeouts = TimeoutConfiguration::new(
                Some(request.script_timeout),
                Some(request.page_load_timeout),
                Some(Duration::from_secs(0)),
            );
            client
                .update_timeouts(timeouts)
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "update_timeouts"))?;

            let wd_session_id = client
                .session_id()
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "session_id"))?
                .unwrap_or_else(|| format!("session-{}", request.session_id));

            let context_generation = self.next_context_generation.fetch_add(1, Ordering::SeqCst);
            let handle = BrowserContextHandle {
                session_id: request.session_id,
                webdriver_session_id: wd_session_id,
                run_id: request.run_id,
                lease_id: request.lease_id,
                fence: request.fence,
                context_generation,
                execution_mode: request.execution_mode,
                model_policy: request.model_policy,
                user_data_dir: request.user_data_dir,
            };

            let entry = SessionEntry {
                client,
                handle: handle.clone(),
                egress_policy: request.egress_policy,
                page_generation: 1,
            };
            {
                let mut guard =
                    self.sessions
                        .lock()
                        .map_err(|_| BrowserError::InvalidConfiguration {
                            reason: "Verrou des sessions navigateur corrompu".into(),
                        })?;
                guard.insert(handle.session_id, entry);
            }

            tracing::info!(
                run_id = %handle.run_id,
                session_id = %handle.session_id,
                context_generation = handle.context_generation,
                "Contexte WebDriver isolé ouvert"
            );
            Ok(handle)
        })
    }

    fn observe<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        scope: ObserveScope,
    ) -> BoxFuture<'a, Result<PageObservation, BrowserError>> {
        Box::pin(async move {
            let entry = self.get_session_entry(handle)?;
            let url = entry
                .client
                .current_url()
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "current_url"))?;

            let origin = if url.as_str() == "about:blank" {
                "about:blank".to_owned()
            } else {
                let norm = normalize_origin(&url)?;
                entry
                    .egress_policy
                    .check_origin(&norm, handle.model_policy)?;
                norm
            };

            let title = entry
                .client
                .title()
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "title"))?;

            let text_excerpt = if scope.include_dom_summary {
                let raw_src = entry
                    .client
                    .source()
                    .await
                    .map_err(|err| map_pre_dispatch_cmd_error(&err, "source"))?;
                truncate_utf8(&raw_src, scope.max_text_bytes)
            } else {
                String::new()
            };

            let screenshot_sha256 = if scope.include_screenshot {
                let png = entry
                    .client
                    .screenshot()
                    .await
                    .map_err(|err| map_pre_dispatch_cmd_error(&err, "screenshot"))?;
                let digest_bytes: [u8; 32] = Sha256::digest(&png).into();
                Some(Sha256Digest::from_raw_bytes(digest_bytes))
            } else {
                None
            };

            let page_generation = self.bump_page_generation(handle.session_id)?;
            let obs_id = format!("obs-{}", self.id_gen.new_uuid());
            let observed_at_ms = self.clock.now_ms().as_i64();

            Ok(PageObservation {
                observation_id: obs_id,
                context_generation: handle.context_generation,
                page_generation,
                url,
                origin,
                title,
                text_excerpt,
                screenshot_sha256,
                observed_at_ms,
            })
        })
    }

    fn resolve_target<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        target: &'a TargetSpec,
        observation: &'a PageObservation,
    ) -> BoxFuture<'a, Result<ResolvedTarget, BrowserError>> {
        Box::pin(async move {
            let entry = self.get_session_entry(handle)?;
            if observation.context_generation != handle.context_generation
                || observation.page_generation != entry.page_generation
            {
                return Err(BrowserError::StaleFenceOrContext {
                    reason: format!(
                        "Observation périmée (page_gen={}, courante={}) ; réobservation requise",
                        observation.page_generation, entry.page_generation
                    ),
                });
            }

            ensure_frame_scope_supported(target)?;

            let matched = resolve_strategy_candidates(&entry.client, &target.strategy).await?;
            let chosen = enforce_unique_candidate(&target.strategy, matched)?;

            if let Some(expected_identity) = &target.expected_identity {
                verify_target_identity(
                    expected_identity,
                    &chosen.observed_text,
                    &chosen.observed_attributes,
                )?;
            }

            Ok(ResolvedTarget {
                element_id: chosen.element_id,
                strategy: target.strategy.clone(),
                context_generation: handle.context_generation,
                page_generation: entry.page_generation,
                computed_role: chosen.computed_role,
                computed_label: chosen.computed_label,
                observed_text: chosen.observed_text,
                observed_attributes: chosen.observed_attributes,
                candidate_count: 1,
            })
        })
    }

    fn perform_action<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        intent: &'a BrowserActionIntent,
        target: Option<&'a ResolvedTarget>,
    ) -> BoxFuture<'a, Result<ActionOutcome, BrowserError>> {
        Box::pin(async move {
            let entry = self.get_session_entry(handle)?;
            self.validate_action_preconditions(&entry, handle, intent, target)?;

            let is_mutation = intent.kind.is_mutation();
            if is_mutation {
                self.effect_journal
                    .check_unresolved_unknown(&intent.logical_effect_key)?;
                self.effect_journal.record_pre_dispatch(
                    intent.action_id,
                    &intent.logical_effect_key,
                    handle.fence,
                    &format!("{:?}", intent.kind),
                )?;
            }

            let exec_res = tokio::time::timeout(
                intent.timeout,
                dispatch_webdriver_action(&entry.client, &intent.kind, target),
            )
            .await;

            self.finalize_action_result(handle, intent, is_mutation, exec_res)
        })
    }

    fn evaluate_assertions<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        assertions: &'a [BusinessAssertion],
        observation: &'a PageObservation,
    ) -> BoxFuture<'a, Result<AssertionEvaluationReport, BrowserError>> {
        Box::pin(async move {
            let entry = self.get_session_entry(handle)?;
            if observation.context_generation != handle.context_generation {
                return Err(BrowserError::StaleFenceOrContext {
                    reason: "Observation d'assertion rattachée à une autre génération de contexte"
                        .into(),
                });
            }

            for rule in assertions {
                if let Some(target) = assertion_target(rule) {
                    ensure_frame_scope_supported(target)?;
                }
            }

            let current_url = entry
                .client
                .current_url()
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "assert_url"))?;
            let current_title = entry
                .client
                .title()
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "assert_title"))?;
            let current_origin = if current_url.as_str() == "about:blank" {
                "about:blank".to_owned()
            } else {
                normalize_origin(&current_url)?
            };

            let mut results = Vec::with_capacity(assertions.len());
            for rule in assertions {
                let res = evaluate_single_assertion(
                    &entry.client,
                    &current_url,
                    &current_origin,
                    &current_title,
                    rule,
                )
                .await;
                results.push(res);
            }

            let all_passed = results.iter().all(|r| r.verdict == AssertionVerdict::Pass);

            Ok(AssertionEvaluationReport {
                controlled_origin: current_origin,
                results,
                all_passed,
            })
        })
    }

    fn checkpoint<'a>(
        &'a self,
        handle: &'a BrowserContextHandle,
        input: CheckpointInput,
    ) -> BoxFuture<'a, Result<BrowserCheckpoint, BrowserError>> {
        Box::pin(async move {
            let _entry = self.get_session_entry(handle)?;
            let snapshot = self.effect_journal.snapshot();
            let mut confirmed_effects = Vec::new();
            let mut unknown_effects = Vec::new();
            for item in snapshot {
                match item.state {
                    EffectState::Confirmed => confirmed_effects.push(item.effect_key),
                    EffectState::OutcomeUnknown => unknown_effects.push(item.effect_key),
                    EffectState::None | EffectState::FailedKnown => {}
                }
            }
            confirmed_effects.sort();
            unknown_effects.sort();

            Ok(BrowserCheckpoint {
                run_id: handle.run_id,
                lease_id: handle.lease_id,
                fence: handle.fence,
                context_generation: handle.context_generation,
                step_ref: input.step_ref,
                last_observation_id: input.last_observation_id,
                confirmed_effects,
                unknown_effects,
                ephemeral_handles_invalidated: true,
            })
        })
    }

    fn close_context(
        &self,
        handle: BrowserContextHandle,
        _reason: CloseReason,
    ) -> BoxFuture<'_, Result<CloseContextResult, BrowserError>> {
        Box::pin(async move {
            let entry = self.remove_session_entry(&handle)?;
            let close_res =
                tokio::time::timeout(Duration::from_secs(5), entry.client.close()).await;
            let (outcome, detail) = match close_res {
                Ok(Ok(())) => (CloseOutcome::Closed, "Session WebDriver fermée".into()),
                Ok(Err(err)) => (
                    CloseOutcome::Unknown,
                    format!("Fermeture WebDriver incertaine : {err}"),
                ),
                Err(_) => (
                    CloseOutcome::Unknown,
                    "Timeout lors de la fermeture de session WebDriver".into(),
                ),
            };
            Ok(CloseContextResult {
                session_id: handle.session_id,
                outcome,
                detail,
            })
        })
    }
}

impl FantocciniBrowserAdapter {
    fn validate_action_preconditions(
        &self,
        entry: &SessionEntry,
        handle: &BrowserContextHandle,
        intent: &BrowserActionIntent,
        target: Option<&ResolvedTarget>,
    ) -> Result<(), BrowserError> {
        if intent.requires_model_call {
            self.model_guard.guard_model_call(
                handle.run_id,
                handle.model_policy,
                ModelAttemptKind::SelectorRepair,
                "ActionIntent demandant un appel modèle",
            )?;
        }

        if let BrowserActionKind::ModelSuppliedExecuteScript { .. } = &intent.kind {
            self.model_guard.guard_model_call(
                handle.run_id,
                handle.model_policy,
                ModelAttemptKind::ModelSuppliedScript,
                "execute_script fourni par un modèle",
            )?;
            return Err(BrowserError::UnsupportedLocatorOrScript {
                reason: "execute_script fourni par un modèle interdit même hors mode forbidden"
                    .into(),
            });
        }

        if intent.kind.requires_target() {
            let resolved = target.ok_or_else(|| BrowserError::InvalidConfiguration {
                reason: "Cette opération requiert une cible ResolvedTarget".into(),
            })?;
            if resolved.context_generation != handle.context_generation
                || resolved.page_generation != entry.page_generation
            {
                return Err(BrowserError::StaleFenceOrContext {
                    reason: "Cible résolue périmée suite à une navigation ou reprise".into(),
                });
            }
        }

        if let BrowserActionKind::Navigate { url } = &intent.kind {
            let origin = normalize_origin(url)?;
            entry
                .egress_policy
                .check_origin(&origin, handle.model_policy)?;
        }

        Ok(())
    }

    fn finalize_action_result(
        &self,
        handle: &BrowserContextHandle,
        intent: &BrowserActionIntent,
        is_mutation: bool,
        exec_res: Result<Result<DispatchOutput, CmdError>, tokio::time::error::Elapsed>,
    ) -> Result<ActionOutcome, BrowserError> {
        match exec_res {
            Ok(Ok(outcome)) => {
                if is_mutation {
                    self.effect_journal
                        .mark_confirmed(&intent.logical_effect_key, "webdriver_ok");
                    let _ = self.bump_page_generation(handle.session_id);
                }
                Ok(ActionOutcome {
                    action_id: intent.action_id,
                    effect_state: if is_mutation {
                        EffectState::Confirmed
                    } else {
                        EffectState::None
                    },
                    dispatch_confirmed: true,
                    text_output: outcome.text_output,
                    screenshot_sha256: outcome.screenshot_sha256,
                })
            }
            Ok(Err(cmd_err)) => {
                if !is_mutation {
                    return Err(map_pre_dispatch_cmd_error(&cmd_err, "perform_action_read"));
                }
                if is_conclusive_pre_mutation_error(&cmd_err) {
                    self.effect_journal
                        .mark_failed_known(&intent.logical_effect_key, &cmd_err.to_string());
                    return Err(map_pre_dispatch_cmd_error(&cmd_err, "perform_action"));
                }
                let reason = format!("Coupure ou erreur non concluante après dispatch : {cmd_err}");
                self.effect_journal.mark_outcome_unknown(
                    intent.action_id,
                    &intent.logical_effect_key,
                    &reason,
                );
                Err(BrowserError::OutcomeUnknown {
                    action_id: intent.action_id,
                    effect_key: intent.logical_effect_key.clone(),
                    reason,
                })
            }
            Err(_elapsed) => {
                if !is_mutation {
                    return Err(BrowserError::ActionabilityTimeout {
                        message: "Timeout sur opération de lecture".into(),
                    });
                }
                let reason = format!(
                    "Dépassement de délai ({:?}) après dispatch de la mutation",
                    intent.timeout
                );
                self.effect_journal.mark_outcome_unknown(
                    intent.action_id,
                    &intent.logical_effect_key,
                    &reason,
                );
                Err(BrowserError::OutcomeUnknown {
                    action_id: intent.action_id,
                    effect_key: intent.logical_effect_key.clone(),
                    reason,
                })
            }
        }
    }
}

struct CandidateElement {
    element_id: String,
    computed_role: Option<String>,
    computed_label: Option<String>,
    observed_text: String,
    observed_attributes: BTreeMap<String, String>,
}

struct DispatchOutput {
    text_output: Option<String>,
    screenshot_sha256: Option<Sha256Digest>,
}

async fn resolve_strategy_candidates(
    client: &Client,
    strategy: &LocatorStrategy,
) -> Result<Vec<CandidateElement>, BrowserError> {
    match strategy {
        LocatorStrategy::TestId { .. } | LocatorStrategy::Css { .. } => {
            let css =
                strategy
                    .as_direct_css()
                    .ok_or_else(|| BrowserError::InvalidConfiguration {
                        reason: "Sélecteur CSS indisponible".into(),
                    })?;
            let elements = client
                .find_all(Locator::Css(&css))
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "find_all_css"))?;
            let mut out = Vec::with_capacity(elements.len());
            for el in elements {
                out.push(inspect_candidate_element(client, el, false).await?);
            }
            Ok(out)
        }
        LocatorStrategy::RoleName { role, name, exact } => {
            let elements = client
                .find_all(Locator::Css(
                    "button, a, input, select, textarea, [role], h1, h2, h3, h4, h5, h6, summary, [tabindex]",
                ))
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "find_all_role_candidates"))?;
            let mut matched = Vec::new();
            for el in elements {
                let cand = inspect_candidate_element(client, el, true).await?;
                let role_ok = cand
                    .computed_role
                    .as_deref()
                    .is_some_and(|r| r.eq_ignore_ascii_case(role.trim()));
                let label_ok = cand
                    .computed_label
                    .as_deref()
                    .is_some_and(|l| text_matches(name, l, *exact));
                if role_ok && label_ok {
                    matched.push(cand);
                }
            }
            Ok(matched)
        }
        LocatorStrategy::Label { text, exact } => {
            let elements = client
                .find_all(Locator::Css(
                    "input, select, textarea, button, [aria-label], [aria-labelledby]",
                ))
                .await
                .map_err(|err| map_pre_dispatch_cmd_error(&err, "find_all_label_candidates"))?;
            let mut matched = Vec::new();
            for el in elements {
                let cand = inspect_candidate_element(client, el, true).await?;
                let label_ok = cand
                    .computed_label
                    .as_deref()
                    .is_some_and(|l| text_matches(text, l, *exact));
                if label_ok {
                    matched.push(cand);
                }
            }
            Ok(matched)
        }
    }
}

async fn inspect_candidate_element(
    client: &Client,
    el: fantoccini::elements::Element,
    query_accessibility: bool,
) -> Result<CandidateElement, BrowserError> {
    let element_id = el.element_id().to_string();
    let observed_text = el
        .text()
        .await
        .map_err(|err| map_pre_dispatch_cmd_error(&err, "element_text"))?;

    let mut observed_attributes = BTreeMap::new();
    for attr_name in [
        "id",
        "data-testid",
        "data-sku",
        "data-account",
        "data-price-minor",
        "data-currency",
        "aria-label",
        "role",
        "name",
        "value",
        "href",
    ] {
        if let Ok(Some(val)) = el.attr(attr_name).await {
            observed_attributes.insert(attr_name.to_owned(), val);
        }
    }

    let (computed_role, computed_label) = if query_accessibility {
        let role_val = client
            .issue_cmd(GetComputedRole {
                element_id: element_id.clone(),
            })
            .await
            .map_err(|err| map_pre_dispatch_cmd_error(&err, "computedrole"))?;
        let label_val = client
            .issue_cmd(GetComputedLabel {
                element_id: element_id.clone(),
            })
            .await
            .map_err(|err| map_pre_dispatch_cmd_error(&err, "computedlabel"))?;

        let role = extract_w3c_optional_string(role_val, "computedrole")?
            .or_else(|| observed_attributes.get("role").cloned());
        let label = extract_w3c_optional_string(label_val, "computedlabel")?
            .or_else(|| observed_attributes.get("aria-label").cloned())
            .or_else(|| {
                if observed_text.trim().is_empty() {
                    None
                } else {
                    Some(observed_text.clone())
                }
            });
        (role, label)
    } else {
        (None, None)
    };

    Ok(CandidateElement {
        element_id,
        computed_role,
        computed_label,
        observed_text,
        observed_attributes,
    })
}

async fn dispatch_webdriver_action(
    client: &Client,
    kind: &BrowserActionKind,
    target: Option<&ResolvedTarget>,
) -> Result<DispatchOutput, CmdError> {
    match kind {
        BrowserActionKind::Navigate { url } => {
            client.goto(url.as_str()).await?;
            Ok(DispatchOutput::empty())
        }
        BrowserActionKind::TakeScreenshot => {
            let bytes = client.screenshot().await?;
            let digest_bytes: [u8; 32] = Sha256::digest(&bytes).into();
            Ok(DispatchOutput {
                text_output: None,
                screenshot_sha256: Some(Sha256Digest::from_raw_bytes(digest_bytes)),
            })
        }
        BrowserActionKind::SwitchFrame { index } => {
            match index {
                Some(idx) => client.enter_frame(*idx).await?,
                None => client.enter_parent_frame().await?,
            }
            Ok(DispatchOutput::empty())
        }
        BrowserActionKind::ModelSuppliedExecuteScript { .. } => Err(CmdError::InvalidArgument(
            "script".into(),
            "model_supplied_script_forbidden".into(),
        )),
        other => dispatch_element_action(client, other, target).await,
    }
}

impl DispatchOutput {
    const fn empty() -> Self {
        Self {
            text_output: None,
            screenshot_sha256: None,
        }
    }
}

async fn dispatch_element_action(
    client: &Client,
    kind: &BrowserActionKind,
    target: Option<&ResolvedTarget>,
) -> Result<DispatchOutput, CmdError> {
    let resolved = target.ok_or_else(|| {
        CmdError::InvalidArgument("target".into(), "missing ResolvedTarget".into())
    })?;
    let el = fantoccini::elements::Element::from_element_id(
        client.clone(),
        resolved.element_id.clone().into(),
    );

    match kind {
        BrowserActionKind::Click => {
            el.click().await?;
            Ok(DispatchOutput::empty())
        }
        BrowserActionKind::Fill { value, clear_first } => {
            if *clear_first {
                el.clear().await?;
            }
            el.send_keys(value).await?;
            Ok(DispatchOutput::empty())
        }
        BrowserActionKind::SelectOption { value } => {
            let selector = format!("option[value=\"{}\"]", escape_css_attr_value(value));
            let options = el.find_all(Locator::Css(&selector)).await?;
            let opt = options.into_iter().next().ok_or_else(|| {
                CmdError::InvalidArgument("option".into(), format!("Option '{value}' introuvable"))
            })?;
            opt.click().await?;
            Ok(DispatchOutput::empty())
        }
        BrowserActionKind::SetChecked { checked } => {
            if el.is_selected().await? != *checked {
                el.click().await?;
            }
            Ok(DispatchOutput::empty())
        }
        BrowserActionKind::PressKey { key } => {
            el.send_keys(key).await?;
            Ok(DispatchOutput::empty())
        }
        BrowserActionKind::ReadText => {
            let txt = el.text().await?;
            Ok(DispatchOutput {
                text_output: Some(txt),
                screenshot_sha256: None,
            })
        }
        BrowserActionKind::ReadAttribute { name } => {
            let attr = el.attr(name).await?;
            Ok(DispatchOutput {
                text_output: attr,
                screenshot_sha256: None,
            })
        }
        _ => Ok(DispatchOutput::empty()),
    }
}

async fn evaluate_single_assertion(
    client: &Client,
    current_url: &Url,
    current_origin: &str,
    current_title: &str,
    rule: &BusinessAssertion,
) -> AssertionRuleResult {
    let rule_id = rule.rule_id().to_owned();
    match rule {
        BusinessAssertion::UrlOriginEquals {
            expected_origin, ..
        } => bool_rule_result(
            rule_id,
            current_origin == expected_origin,
            format!("attendu '{expected_origin}', observé '{current_origin}'"),
        ),
        BusinessAssertion::UrlPathEquals { expected_path, .. } => {
            let obs_path = current_url.path();
            bool_rule_result(
                rule_id,
                obs_path == expected_path,
                format!("attendu '{expected_path}', observé '{obs_path}'"),
            )
        }
        BusinessAssertion::TitleContains { substring, .. } => bool_rule_result(
            rule_id,
            current_title.contains(substring),
            format!("sous-chaîne '{substring}' dans titre '{current_title}'"),
        ),
        BusinessAssertion::ElementCountEquals {
            strategy, expected, ..
        } => match resolve_strategy_candidates(client, strategy).await {
            Ok(cands) => bool_rule_result(
                rule_id,
                cands.len() == *expected,
                format!("cardinalité attendue {expected}, observée {}", cands.len()),
            ),
            Err(err) => AssertionRuleResult {
                rule_id,
                verdict: AssertionVerdict::Unknown,
                detail: err.to_string(),
            },
        },
        other => evaluate_target_assertion(client, rule_id, other).await,
    }
}

async fn evaluate_target_assertion(
    client: &Client,
    rule_id: String,
    rule: &BusinessAssertion,
) -> AssertionRuleResult {
    let Some(target) = assertion_target(rule) else {
        return AssertionRuleResult {
            rule_id,
            verdict: AssertionVerdict::Unknown,
            detail: "Règle non ciblée".into(),
        };
    };

    let cand = match resolve_unique_for_assertion(client, target).await {
        Ok(c) => c,
        Err(err) => {
            return AssertionRuleResult {
                rule_id,
                verdict: AssertionVerdict::Fail,
                detail: err.to_string(),
            };
        }
    };

    match rule {
        BusinessAssertion::ElementTextEquals { expected, .. } => {
            let obs = cand.observed_text.trim();
            bool_rule_result(
                rule_id,
                obs == expected.trim(),
                format!("attendu '{expected}', observé '{obs}'"),
            )
        }
        BusinessAssertion::ElementTextContains { substring, .. } => bool_rule_result(
            rule_id,
            cand.observed_text.contains(substring),
            format!(
                "sous-chaîne '{substring}' dans '{}'",
                cand.observed_text.trim()
            ),
        ),
        BusinessAssertion::ElementAttrEquals { attr, expected, .. } => {
            let obs = cand
                .observed_attributes
                .get(attr)
                .map_or("<absent>", String::as_str);
            bool_rule_result(
                rule_id,
                obs == expected,
                format!("{attr}: attendu '{expected}', observé '{obs}'"),
            )
        }
        BusinessAssertion::NumericMinorAtMost { max_minor, .. } => {
            evaluate_numeric_rule(rule_id, &cand.observed_text, |val| {
                (
                    val <= *max_minor,
                    format!("montant observé {val} <= plafond {max_minor}"),
                )
            })
        }
        BusinessAssertion::QuantityEquals { expected, .. } => {
            evaluate_numeric_rule(rule_id, &cand.observed_text, |val| {
                (
                    val == *expected,
                    format!("quantité attendue {expected}, observée {val}"),
                )
            })
        }
        _ => AssertionRuleResult {
            rule_id,
            verdict: AssertionVerdict::Unknown,
            detail: "Règle inattendue".into(),
        },
    }
}

fn bool_rule_result(rule_id: String, ok: bool, detail: String) -> AssertionRuleResult {
    AssertionRuleResult {
        rule_id,
        verdict: if ok {
            AssertionVerdict::Pass
        } else {
            AssertionVerdict::Fail
        },
        detail,
    }
}

fn evaluate_numeric_rule(
    rule_id: String,
    raw_text: &str,
    check: impl FnOnce(u64) -> (bool, String),
) -> AssertionRuleResult {
    match raw_text.trim().parse::<u64>() {
        Ok(val) => {
            let (ok, detail) = check(val);
            bool_rule_result(rule_id, ok, detail)
        }
        Err(_) => AssertionRuleResult {
            rule_id,
            verdict: AssertionVerdict::Unknown,
            detail: format!("Valeur non numérique '{}'", raw_text.trim()),
        },
    }
}

fn ensure_frame_scope_supported(target: &TargetSpec) -> Result<(), BrowserError> {
    if let Some(scope) = &target.frame_scope {
        return Err(BrowserError::CapabilityMissing {
            details_ref: "target.frame_scope".into(),
            message: format!(
                "Portée TargetSpec::frame_scope ('{scope}') non prise en charge dans ce pilote ; utiliser BrowserActionKind::SwitchFrame"
            ),
        });
    }
    Ok(())
}

const fn assertion_target(rule: &BusinessAssertion) -> Option<&TargetSpec> {
    match rule {
        BusinessAssertion::ElementTextEquals { target, .. }
        | BusinessAssertion::ElementTextContains { target, .. }
        | BusinessAssertion::ElementAttrEquals { target, .. }
        | BusinessAssertion::NumericMinorAtMost { target, .. }
        | BusinessAssertion::QuantityEquals { target, .. } => Some(target),
        BusinessAssertion::UrlOriginEquals { .. }
        | BusinessAssertion::UrlPathEquals { .. }
        | BusinessAssertion::TitleContains { .. }
        | BusinessAssertion::ElementCountEquals { .. } => None,
    }
}

async fn resolve_unique_for_assertion(
    client: &Client,
    target: &TargetSpec,
) -> Result<CandidateElement, BrowserError> {
    ensure_frame_scope_supported(target)?;
    let matched = resolve_strategy_candidates(client, &target.strategy).await?;
    let chosen = enforce_unique_candidate(&target.strategy, matched)?;
    if let Some(expected_identity) = &target.expected_identity {
        verify_target_identity(
            expected_identity,
            &chosen.observed_text,
            &chosen.observed_attributes,
        )?;
    }
    Ok(chosen)
}

fn validate_loopback_webdriver_url(url: &Url) -> Result<(), BrowserError> {
    let scheme = url.scheme().to_ascii_lowercase();
    if scheme == "https" {
        return Err(BrowserError::CapabilityMissing {
            details_ref: "remote_https_webdriver_tls_not_enabled".into(),
            message: "Le pilote V1 loopback utilise HTTP local sans TLS fictif (Q-18)".into(),
        });
    }
    if scheme != "http" {
        return Err(BrowserError::InvalidConfiguration {
            reason: format!("Schéma WebDriver non supporté : '{scheme}'"),
        });
    }
    let host = url.host_str().unwrap_or("");
    let is_loopback =
        host == "127.0.0.1" || host == "::1" || host.eq_ignore_ascii_case("localhost");
    if !is_loopback {
        return Err(BrowserError::InvalidConfiguration {
            reason: format!("Endpoint WebDriver HTTP non loopback interdit en V1 : '{host}'"),
        });
    }
    Ok(())
}

fn ensure_isolated_user_data_dir(dir: &Path) -> Result<(), BrowserError> {
    if !dir.is_absolute() {
        return Err(BrowserError::InvalidConfiguration {
            reason: "user_data_dir doit être un chemin absolu isolé".into(),
        });
    }
    fs::create_dir_all(dir).map_err(|err| BrowserError::InvalidConfiguration {
        reason: format!(
            "Impossible de créer user_data_dir '{}': {err}",
            dir.display()
        ),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, Permissions::from_mode(0o700)).map_err(|err| {
            BrowserError::InvalidConfiguration {
                reason: format!(
                    "Impossible d'appliquer les permissions 0700 sur '{}': {err}",
                    dir.display()
                ),
            }
        })?;
    }
    Ok(())
}

fn build_webdriver_capabilities(
    request: &OpenContextRequest,
) -> Result<Capabilities, BrowserError> {
    let mut caps = Map::new();
    let browser_name = match request.runtime.engine {
        BrowserEngineKind::ChromeForTesting => "chrome",
        BrowserEngineKind::FirefoxOptional => "firefox",
        BrowserEngineKind::SafariUnsupported => {
            return Err(BrowserError::CapabilityMissing {
                details_ref: "safari_mediated_unsupported".into(),
                message: "Safari n'est pas pris en charge en mode mediated V1".into(),
            });
        }
    };
    caps.insert("browserName".into(), Value::String(browser_name.into()));

    if request.runtime.engine == BrowserEngineKind::ChromeForTesting {
        let mut args = vec![
            format!("--user-data-dir={}", request.user_data_dir.display()),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            "--disable-background-networking".into(),
            "--disable-component-update".into(),
            "--disable-quic".into(),
            "--webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
            "--force-webrtc-ip-handling-policy".into(),
        ];
        if request.headless {
            args.push("--headless=new".into());
        }
        if let Some(proxy_url) = &request.proxy_url {
            let proxy_str = proxy_url.as_str().trim_end_matches('/').to_owned();
            args.push(format!("--proxy-server={proxy_str}"));
            args.push("--proxy-bypass-list=<-loopback>".into());
        }
        let mut chrome_opts = Map::new();
        chrome_opts.insert(
            "binary".into(),
            Value::String(request.runtime.browser.path.display().to_string()),
        );
        chrome_opts.insert(
            "args".into(),
            Value::Array(args.into_iter().map(Value::String).collect()),
        );
        caps.insert("goog:chromeOptions".into(), Value::Object(chrome_opts));
    }

    if let Some(proxy_url) = &request.proxy_url {
        let host_port = match (proxy_url.host_str(), proxy_url.port_or_known_default()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            _ => {
                return Err(BrowserError::InvalidConfiguration {
                    reason: format!("URL de proxy invalide : '{proxy_url}'"),
                });
            }
        };
        let mut proxy_obj = Map::new();
        proxy_obj.insert("proxyType".into(), Value::String("manual".into()));
        proxy_obj.insert("httpProxy".into(), Value::String(host_port.clone()));
        proxy_obj.insert("sslProxy".into(), Value::String(host_port));
        caps.insert("proxy".into(), Value::Object(proxy_obj));
    }

    if request.model_policy == ModelPolicy::Forbidden {
        caps.insert(
            "unleash:modelPolicy".into(),
            Value::String("forbidden".into()),
        );
    }

    Ok(caps)
}

fn is_conclusive_pre_mutation_error(err: &CmdError) -> bool {
    err.is_no_such_element()
        || err.is_element_not_interactable()
        || err.is_stale_element_reference()
        || err.is_invalid_selector()
        || err.is_invalid_argument()
}

fn map_pre_dispatch_cmd_error(err: &CmdError, op: &str) -> BrowserError {
    if err.is_no_such_element() {
        return BrowserError::TargetMissing {
            locator: format!("{op}:no_such_element"),
        };
    }
    if err.is_element_not_interactable() || err.is_timeout() || err.is_script_timeout() {
        return BrowserError::ActionabilityTimeout {
            message: format!("{op}: {err}"),
        };
    }
    if err.is_stale_element_reference() {
        return BrowserError::StaleFenceOrContext {
            reason: format!("{op}: élément DOM devenu périmé"),
        };
    }
    if err.is_invalid_argument() || err.is_invalid_selector() {
        return BrowserError::InvalidConfiguration {
            reason: format!("{op}: {err}"),
        };
    }
    BrowserError::ActionabilityTimeout {
        message: format!("{op}: erreur WebDriver avant mutation ({err})"),
    }
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("").to_owned()
}
