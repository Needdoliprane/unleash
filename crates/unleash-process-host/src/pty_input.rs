// SPDX-License-Identifier: Apache-2.0
//! Entrées PTY numérotées avec reçu durable et interdiction de répétition automatique
//! après coupure ambiguë (`REQ-TERM-005`, `F011`, `C3`/`C4`/`C5`).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use unleash_domain::{ActionId, CommandId, ReceiptId, Sha256Digest, TimestampMs};
use unleash_platform::{
    assert_not_under_claude, ensure_private_rendezvous_dir, read_private_file_to_string,
    write_private_file_atomic,
};

use crate::error::ProcessHostError;

/// Points de coupure observables lors du traitement d'une entrée PTY numérotée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostInputCutpoint {
    /// Avant d'écrire le marqueur durable `WriteDispatched` (`C3`).
    C3BeforeWriteDispatched,
    /// Après le marqueur durable `WriteDispatched` et avant l'écriture au maître PTY (`C4`).
    C4AfterDispatchedBeforePtyWrite,
    /// Après l'écriture effective au maître PTY et avant la persistance du reçu durable (`C5`).
    C5AfterPtyWriteBeforeReceipt,
}

/// Observateur injectable des points de coupure d'entrée PTY (sans dépendance testkit en production).
pub trait HostCutpointObserver: Send + Sync {
    /// Notifie l'atteinte d'un point de coupure ; peut bloquer sur barrière ou injecter une erreur.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si le point de coupure déclenche une panne ou barrière annulée.
    fn hit(&self, cutpoint: HostInputCutpoint) -> Result<(), ProcessHostError>;
}

/// Observateur neutre sans effet.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopHostCutpointObserver;

impl HostCutpointObserver for NoopHostCutpointObserver {
    fn hit(&self, _cutpoint: HostInputCutpoint) -> Result<(), ProcessHostError> {
        Ok(())
    }
}

/// Nom du journal durable d'entrées PTY (`0600`).
pub const INPUT_JOURNAL_FILENAME: &str = "input-journal.json";

/// Maximum bytes for one input, independent of the private protocol frame size.
pub const MAX_PTY_INPUT_BYTES: usize = 65_536;

/// État durable d'une entrée numérotée soumise au PTY.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InputEntryState {
    /// Marqueur durable écrit AVANT l'écriture au maître PTY (`dispatch` engagé).
    WriteDispatched,
    /// Reçu durable confirmé APRÈS l'écriture au maître PTY.
    ReceiptConfirmed {
        /// Identifiant unique du reçu.
        receipt_id: ReceiptId,
        /// Horodatage UTC de confirmation.
        confirmed_at_ms: TimestampMs,
    },
    /// Coupure survenue entre l'écriture PTY et la persistance du reçu : aucune répétition permise.
    OutcomeUnknown {
        /// Motif technique de l'incertitude.
        reason: String,
    },
}

/// Enregistrement durable d'une action d'entrée numérotée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputJournalEntry {
    /// Numéro de séquence monotone (1-based).
    pub input_seq: u64,
    /// Clé d'idempotence de la commande.
    pub command_id: CommandId,
    /// Action autorisant l'entrée.
    pub action_id: ActionId,
    /// Empreinte SHA-256 des octets envoyés.
    pub payload_hash: Sha256Digest,
    /// État courant durable.
    pub entry_state: InputEntryState,
}

/// Reçu retourné au daemon après écriture confirmée au maître PTY.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PtyInputReceipt {
    /// Numéro de séquence confirmé.
    pub input_seq: u64,
    /// Commande associée.
    pub command_id: CommandId,
    /// Action associée.
    pub action_id: ActionId,
    /// Identifiant du reçu.
    pub receipt_id: ReceiptId,
    /// Digest SHA-256 de la charge écrite.
    pub payload_hash: Sha256Digest,
    /// Vrai si ce reçu provient d'une déduplication idempotente sans nouvelle écriture PTY.
    pub deduplicated: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputJournalDisk {
    schema_version: u32,
    entries: BTreeMap<u64, InputJournalEntry>,
}

struct TrackerInner {
    journal_path: PathBuf,
    disk: InputJournalDisk,
}

/// Gestionnaire thread-safe et durable des entrées PTY numérotées (`REQ-TERM-005`).
pub struct PtyInputTracker {
    inner: Mutex<TrackerInner>,
    cutpoints: Arc<dyn HostCutpointObserver>,
}

impl std::fmt::Debug for PtyInputTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f.debug_struct("PtyInputTracker")
            .field("journal_path", &guard.journal_path)
            .field("entry_count", &guard.disk.entries.len())
            .finish_non_exhaustive()
    }
}

impl PtyInputTracker {
    /// Ouvre ou initialise le journal durable d'entrées dans `runtime_dir`.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si le chemin est invalide ou le JSON corrompu.
    pub fn open(
        runtime_dir: impl AsRef<Path>,
        cutpoints: Option<Arc<dyn HostCutpointObserver>>,
    ) -> Result<Self, ProcessHostError> {
        let dir = runtime_dir.as_ref();
        assert_not_under_claude(dir)?;
        ensure_private_rendezvous_dir(dir)?;
        let journal_path = dir.join(INPUT_JOURNAL_FILENAME);
        let disk = if let Some(raw) = read_private_file_to_string(&journal_path)? {
            serde_json::from_str::<InputJournalDisk>(&raw)?
        } else {
            let fresh = InputJournalDisk {
                schema_version: 1,
                entries: BTreeMap::new(),
            };
            persist_journal(&journal_path, &fresh)?;
            fresh
        };
        Ok(Self {
            inner: Mutex::new(TrackerInner { journal_path, disk }),
            cutpoints: cutpoints.unwrap_or_else(|| Arc::new(NoopHostCutpointObserver)),
        })
    }

    /// Réconcilie au démarrage/rattachement toute entrée restée bloquée en `WriteDispatched` :
    /// elle bascule définitivement en `OutcomeUnknown` et interdit toute répétition automatique.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si la persistance échoue.
    pub fn reconcile_in_flight(&self) -> Result<Vec<u64>, ProcessHostError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut unknown_seqs = Vec::new();
        for (seq, entry) in &mut guard.disk.entries {
            if matches!(entry.entry_state, InputEntryState::WriteDispatched) {
                entry.entry_state = InputEntryState::OutcomeUnknown {
                    reason: String::from(
                        "coupure survenue après écriture au PTY et avant reçu durable",
                    ),
                };
                unknown_seqs.push(*seq);
            }
        }
        if !unknown_seqs.is_empty() {
            persist_journal(&guard.journal_path, &guard.disk)?;
        }
        Ok(unknown_seqs)
    }

    /// Retourne l'état courant des séquences confirmées et ambiguës (`(last_confirmed, in_flight_unknown)`).
    #[must_use]
    pub fn summary(&self) -> (u64, Vec<u64>) {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut last_confirmed = 0_u64;
        let mut unknowns = Vec::new();
        for (seq, entry) in &guard.disk.entries {
            match &entry.entry_state {
                InputEntryState::ReceiptConfirmed { .. } => {
                    last_confirmed = last_confirmed.max(*seq);
                }
                InputEntryState::WriteDispatched | InputEntryState::OutcomeUnknown { .. } => {
                    unknowns.push(*seq);
                }
            }
        }
        (last_confirmed, unknowns)
    }

    /// Exécute ou déduplique une écriture numérotée vers `writer` selon le protocole à 2 phases.
    ///
    /// - Si `input_seq` possède déjà un reçu confirmé avec même `command_id` et `payload_hash` :
    ///   retourne ce reçu (`deduplicated = true`) sans réécrire au PTY.
    /// - Si `input_seq` est déjà en `WriteDispatched` ou `OutcomeUnknown` :
    ///   passe en `OutcomeUnknown` et retourne [`ProcessHostError::TerminalInputOutcomeUnknown`]
    ///   **sans jamais réécrire au PTY**.
    /// - Si `input_seq` diverge en `command_id` ou `payload_hash` : [`ProcessHostError::IdempotencyConflict`].
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] en cas de conflit, saut de séquence, erreur I/O ou issue inconnue.
    pub fn dispatch_input<W: Write + ?Sized>(
        &self,
        writer: &mut W,
        input_seq: u64,
        command_id: CommandId,
        action_id: ActionId,
        payload: &[u8],
        now_ms: TimestampMs,
    ) -> Result<PtyInputReceipt, ProcessHostError> {
        if payload.is_empty() || payload.len() > MAX_PTY_INPUT_BYTES {
            return Err(ProcessHostError::InvalidArgument {
                reason: String::from("input payload must contain 1 through 65536 bytes"),
            });
        }
        if input_seq == 0 {
            return Err(ProcessHostError::InvalidArgument {
                reason: String::from("input_seq doit être >= 1"),
            });
        }
        let payload_hash = Sha256Digest::from_bytes(payload);

        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if guard.disk.entries.contains_key(&input_seq) {
            return resolve_existing_entry(
                &mut guard,
                input_seq,
                command_id,
                action_id,
                payload_hash,
            );
        }

        let expected_next = guard
            .disk
            .entries
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        if input_seq != expected_next {
            return Err(ProcessHostError::InvalidArgument {
                reason: format!(
                    "séquence d'entrée inattendue : attendue {expected_next}, reçue {input_seq}"
                ),
            });
        }

        // Cutpoint C3 : avant marqueur durable de dispatch.
        self.hit_cutpoint(HostInputCutpoint::C3BeforeWriteDispatched)?;

        // Phase 1 : écrire le marqueur durable WriteDispatched AVANT l'écriture au maître PTY.
        guard.disk.entries.insert(
            input_seq,
            InputJournalEntry {
                input_seq,
                command_id,
                action_id,
                payload_hash,
                entry_state: InputEntryState::WriteDispatched,
            },
        );
        persist_journal(&guard.journal_path, &guard.disk)?;

        // Cutpoint C4 : après marqueur durable de dispatch, avant écriture PTY.
        self.hit_cutpoint(HostInputCutpoint::C4AfterDispatchedBeforePtyWrite)?;

        // Phase 2 : écriture effective vers le maître PTY.
        if let Err(io_err) = writer.write_all(payload).and_then(|()| writer.flush()) {
            // L'écriture a pu délivrer tout ou partie des octets : impossible de prouver aucun effet -> OutcomeUnknown.
            let reason = format!("erreur I/O pendant écriture PTY : {io_err}");
            if let Some(entry) = guard.disk.entries.get_mut(&input_seq) {
                entry.entry_state = InputEntryState::OutcomeUnknown {
                    reason: reason.clone(),
                };
                let _ = persist_journal(&guard.journal_path, &guard.disk);
            }
            return Err(ProcessHostError::TerminalInputOutcomeUnknown {
                input_seq,
                action_id,
                reason,
            });
        }

        // Cutpoint C5 : APRÈS écriture PTY et AVANT persistance du reçu durable !
        self.hit_cutpoint(HostInputCutpoint::C5AfterPtyWriteBeforeReceipt)?;

        // Phase 3 : persister le reçu durable ReceiptConfirmed.
        let receipt_id = ReceiptId::from_uuid(crate::manifest::new_random_uuid());
        if let Some(entry) = guard.disk.entries.get_mut(&input_seq) {
            entry.entry_state = InputEntryState::ReceiptConfirmed {
                receipt_id,
                confirmed_at_ms: now_ms,
            };
        }
        persist_journal(&guard.journal_path, &guard.disk)?;

        Ok(PtyInputReceipt {
            input_seq,
            command_id,
            action_id,
            receipt_id,
            payload_hash,
            deduplicated: false,
        })
    }

    fn hit_cutpoint(&self, cp: HostInputCutpoint) -> Result<(), ProcessHostError> {
        self.cutpoints.hit(cp)
    }
}

fn resolve_existing_entry(
    guard: &mut TrackerInner,
    input_seq: u64,
    command_id: CommandId,
    action_id: ActionId,
    payload_hash: Sha256Digest,
) -> Result<PtyInputReceipt, ProcessHostError> {
    let mut transition_to_unknown = false;
    let outcome = {
        let Some(existing) = guard.disk.entries.get(&input_seq) else {
            return Err(ProcessHostError::InvalidArgument {
                reason: format!("entrée {input_seq} introuvable"),
            });
        };
        if existing.command_id != command_id
            || existing.action_id != action_id
            || existing.payload_hash != payload_hash
        {
            return Err(ProcessHostError::IdempotencyConflict {
                reason: format!(
                    "conflit sur input_seq={input_seq} : command_id, action_id ou payload_hash divergent"
                ),
            });
        }
        match &existing.entry_state {
            InputEntryState::ReceiptConfirmed { receipt_id, .. } => Ok(PtyInputReceipt {
                input_seq,
                command_id,
                action_id,
                receipt_id: *receipt_id,
                payload_hash,
                deduplicated: true,
            }),
            InputEntryState::WriteDispatched => {
                transition_to_unknown = true;
                Err(ProcessHostError::TerminalInputOutcomeUnknown {
                    input_seq,
                    action_id,
                    reason: String::from(
                        "entrée déjà écrite au PTY sans reçu confirmé : répétition automatique interdite",
                    ),
                })
            }
            InputEntryState::OutcomeUnknown { reason } => {
                Err(ProcessHostError::TerminalInputOutcomeUnknown {
                    input_seq,
                    action_id,
                    reason: reason.clone(),
                })
            }
        }
    };
    if transition_to_unknown {
        if let Some(entry) = guard.disk.entries.get_mut(&input_seq) {
            entry.entry_state = InputEntryState::OutcomeUnknown {
                reason: String::from(
                    "entrée déjà écrite au PTY sans reçu confirmé : répétition automatique interdite",
                ),
            };
        }
        persist_journal(&guard.journal_path, &guard.disk)?;
    }
    outcome
}

fn persist_journal(path: &Path, disk: &InputJournalDisk) -> Result<(), ProcessHostError> {
    let bytes = serde_json::to_vec_pretty(disk)?;
    write_private_file_atomic(path, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unleash_testkit::{Cutpoint, CutpointAction, CutpointController, local_test_dir};

    struct TestkitCutpointAdapter(CutpointController);

    impl HostCutpointObserver for TestkitCutpointAdapter {
        fn hit(&self, cutpoint: HostInputCutpoint) -> Result<(), ProcessHostError> {
            let mapped = match cutpoint {
                HostInputCutpoint::C3BeforeWriteDispatched => Cutpoint::C3,
                HostInputCutpoint::C4AfterDispatchedBeforePtyWrite => Cutpoint::C4,
                HostInputCutpoint::C5AfterPtyWriteBeforeReceipt => Cutpoint::C5,
            };
            self.0
                .hit(mapped)
                .map_err(|e| ProcessHostError::TerminalUnavailable {
                    reason: format!("panne injectée à {mapped} : {e}"),
                })
        }
    }

    #[test]
    fn t_f011_005_crash_after_pty_write_before_receipt_becomes_outcome_unknown_without_retry() {
        let dir = local_test_dir("ph-input").unwrap();
        let runtime_dir = dir.path().join("host");
        let ctrl = CutpointController::new();
        ctrl.set_action_on_hit(Cutpoint::C5, 1, CutpointAction::Fail("killed_at_c5"))
            .unwrap();
        let adapter: Arc<dyn HostCutpointObserver> = Arc::new(TestkitCutpointAdapter(ctrl));

        let tracker = PtyInputTracker::open(&runtime_dir, Some(adapter)).unwrap();
        let mut sink = Vec::<u8>::new();
        let cmd_id = CommandId::from_u128(11);
        let act_id = ActionId::from_u128(22);

        // Premier essai : écrit dans sink puis tombe à C5 avant reçu
        let err = tracker
            .dispatch_input(
                &mut sink,
                1,
                cmd_id,
                act_id,
                b"apply-effect\n",
                TimestampMs::new(1000),
            )
            .unwrap_err();
        assert!(matches!(err, ProcessHostError::TerminalUnavailable { .. }));
        assert_eq!(sink, b"apply-effect\n");

        // Reprise après redémarrage : réconciliation marque seq=1 en OutcomeUnknown
        let tracker_reopened = PtyInputTracker::open(&runtime_dir, None).unwrap();
        let unknown_seqs = tracker_reopened.reconcile_in_flight().unwrap();
        assert_eq!(unknown_seqs, vec![1]);

        // Retry identique : refusé TerminalInputOutcomeUnknown et AUCUNE seconde écriture !
        let retry_err = tracker_reopened
            .dispatch_input(
                &mut sink,
                1,
                cmd_id,
                act_id,
                b"apply-effect\n",
                TimestampMs::new(2000),
            )
            .unwrap_err();
        assert!(matches!(
            retry_err,
            ProcessHostError::TerminalInputOutcomeUnknown { input_seq: 1, .. }
        ));
        assert_eq!(sink, b"apply-effect\n");
    }

    #[test]
    fn confirmed_input_is_deduplicated_without_rewriting() {
        let dir = local_test_dir("ph-dedup").unwrap();
        let runtime_dir = dir.path().join("host");
        let tracker = PtyInputTracker::open(&runtime_dir, None).unwrap();
        let mut sink = Vec::<u8>::new();
        let cmd_id = CommandId::from_u128(33);
        let act_id = ActionId::from_u128(44);

        let r1 = tracker
            .dispatch_input(
                &mut sink,
                1,
                cmd_id,
                act_id,
                b"echo 1\n",
                TimestampMs::new(10),
            )
            .unwrap();
        assert!(!r1.deduplicated);
        assert_eq!(sink, b"echo 1\n");

        let r2 = tracker
            .dispatch_input(
                &mut sink,
                1,
                cmd_id,
                act_id,
                b"echo 1\n",
                TimestampMs::new(20),
            )
            .unwrap();
        assert!(r2.deduplicated);
        assert_eq!(r2.receipt_id, r1.receipt_id);
        assert_eq!(sink, b"echo 1\n");
    }

    #[test]
    fn t_f011_control_input_replay_requires_the_original_action() {
        let dir = local_test_dir("ph-action-binding").unwrap();
        let tracker = PtyInputTracker::open(&dir.path().join("host"), None).unwrap();
        let mut sink = Vec::new();
        let command = CommandId::from_u128(1);
        let action = ActionId::from_u128(2);
        let receipt = tracker
            .dispatch_input(
                &mut sink,
                1,
                command,
                action,
                b"one-effect\n",
                TimestampMs::new(1),
            )
            .unwrap();
        assert_eq!(receipt.action_id, action);
        let foreign = tracker.dispatch_input(
            &mut sink,
            1,
            command,
            ActionId::from_u128(3),
            b"one-effect\n",
            TimestampMs::new(2),
        );
        assert!(
            matches!(foreign, Err(ProcessHostError::IdempotencyConflict { .. })),
            "a different action cannot adopt this receipt"
        );
        assert_eq!(sink, b"one-effect\n");
    }

    #[test]
    fn t_f011_005_crash_at_c3_before_dispatch_marker_leaves_clean_state_and_allows_retry() {
        let dir = local_test_dir("ph-c3").unwrap();
        let runtime_dir = dir.path().join("host");
        let ctrl = CutpointController::new();
        ctrl.set_action_on_hit(Cutpoint::C3, 1, CutpointAction::Fail("killed_at_c3"))
            .unwrap();
        let adapter: Arc<dyn HostCutpointObserver> = Arc::new(TestkitCutpointAdapter(ctrl));

        let tracker = PtyInputTracker::open(&runtime_dir, Some(adapter)).unwrap();
        let mut sink = Vec::<u8>::new();
        let cmd_id = CommandId::from_u128(55);
        let act_id = ActionId::from_u128(66);

        let err = tracker
            .dispatch_input(
                &mut sink,
                1,
                cmd_id,
                act_id,
                b"safe-before-dispatch\n",
                TimestampMs::new(100),
            )
            .unwrap_err();
        assert!(matches!(err, ProcessHostError::TerminalUnavailable { .. }));
        assert!(
            sink.is_empty(),
            "aucune écriture PTY ne doit avoir eu lieu à C3"
        );

        let reopened = PtyInputTracker::open(&runtime_dir, None).unwrap();
        assert!(reopened.reconcile_in_flight().unwrap().is_empty());
        assert_eq!(reopened.summary(), (0, Vec::new()));

        let receipt = reopened
            .dispatch_input(
                &mut sink,
                1,
                cmd_id,
                act_id,
                b"safe-before-dispatch\n",
                TimestampMs::new(200),
            )
            .unwrap();
        assert!(!receipt.deduplicated);
        assert_eq!(sink, b"safe-before-dispatch\n");
    }

    #[test]
    fn t_f011_005_crash_at_c4_after_dispatch_marker_before_pty_write_becomes_outcome_unknown_without_retry()
     {
        let dir = local_test_dir("ph-c4").unwrap();
        let runtime_dir = dir.path().join("host");
        let ctrl = CutpointController::new();
        ctrl.set_action_on_hit(Cutpoint::C4, 1, CutpointAction::Fail("killed_at_c4"))
            .unwrap();
        let adapter: Arc<dyn HostCutpointObserver> = Arc::new(TestkitCutpointAdapter(ctrl));

        let tracker = PtyInputTracker::open(&runtime_dir, Some(adapter)).unwrap();
        let mut sink = Vec::<u8>::new();
        let cmd_id = CommandId::from_u128(77);
        let act_id = ActionId::from_u128(88);

        let err = tracker
            .dispatch_input(
                &mut sink,
                1,
                cmd_id,
                act_id,
                b"post-dispatch\n",
                TimestampMs::new(300),
            )
            .unwrap_err();
        assert!(matches!(err, ProcessHostError::TerminalUnavailable { .. }));
        assert!(sink.is_empty(), "C4 intervient avant écriture PTY");

        let reopened = PtyInputTracker::open(&runtime_dir, None).unwrap();
        assert_eq!(reopened.reconcile_in_flight().unwrap(), vec![1]);

        let retry_err = reopened
            .dispatch_input(
                &mut sink,
                1,
                cmd_id,
                act_id,
                b"post-dispatch\n",
                TimestampMs::new(400),
            )
            .unwrap_err();
        assert!(matches!(
            retry_err,
            ProcessHostError::TerminalInputOutcomeUnknown { input_seq: 1, .. }
        ));
        assert!(sink.is_empty());
    }
}
