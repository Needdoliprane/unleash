// SPDX-License-Identifier: Apache-2.0
//! File locale persistable de corrélation des `CommandEnvelope` (`REQ-CORE-005`, `F001`, `F014`).
//!
//! Garanties critiques :
//! - `authority_generation` et `command_id` sont épinglés à la création durable et conservés
//!   à l'identique sur reconnexion et retry.
//! - Chaque tentative génère un `request_id` frais (exclu du digest canonique RFC 8785).
//! - Un refus `StaleFence / STALE_AUTHORITY_GENERATION` verrouille définitivement l'entrée dans
//!   [`CorrelationState::StaleLineageRejected`] sans jamais réécrire silencieusement la lignée.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use unleash_domain::{AuthorityGeneration, CommandId, RequestId, Sha256Digest};
use unleash_platform::{
    assert_not_under_claude, read_private_file_to_string, write_private_file_atomic,
};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, ErrorCode, IpcError, ReasonCode, RetryClass,
    compute_command_digest_v2, from_json_str_strict,
};

use crate::errors::ClientError;

const CORRELATION_SCHEMA_VERSION: u32 = 1;

/// État durable d'une commande dans la file locale de corrélation du client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CorrelationState {
    /// Enveloppe enregistrée localement, prête pour sa première transmission.
    Pending {
        /// Nombre de tentatives transmises (0 initialement).
        attempts: u32,
    },
    /// Tentative émise (ou interrompue) sans réponse définitive du daemon.
    Uncertain {
        /// Nombre cumulé de tentatives préparées.
        attempts: u32,
        /// Dernier `request_id` de transport employé.
        last_request_id: RequestId,
    },
    /// Commande confirmée par reçu durable [`CommandReceipt`] du daemon.
    Confirmed {
        /// Reçu retourné par le serveur.
        receipt: CommandReceipt,
    },
    /// Commande refusée pour changement de lignée serveur (`StaleFence / STALE_AUTHORITY_GENERATION`).
    ///
    /// Ne doit jamais être réécrite automatiquement avec une nouvelle `authority_generation`.
    StaleLineageRejected {
        /// Erreur IPC exacte retournée par le daemon.
        error: IpcError,
        /// Lignée serveur observée au moment du rejet si connue.
        observed_daemon_generation: Option<AuthorityGeneration>,
    },
    /// Commande définitivement échouée sur un autre motif non relançable automatiquement.
    Failed {
        /// Erreur IPC finale.
        error: IpcError,
    },
}

/// Entrée persistée d'une commande suivie par [`CommandCorrelationQueue`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelatedCommand {
    /// Enveloppe mutante v2 conservant `command_id` et `authority_generation` d'origine.
    pub envelope: CommandEnvelope,
    /// Empreinte canonique RFC 8785 (`compute_command_digest_v2`) vérifiée au chargement.
    pub command_digest: Sha256Digest,
    /// Horodatage UTC de création en millisecondes.
    pub created_at_ms: i64,
    /// Horodatage UTC de dernière mise à jour en millisecondes.
    pub updated_at_ms: i64,
    /// État courant dans la file de corrélation.
    pub state: CorrelationState,
    /// Lien causal optionnel vers une ancienne commande `StaleLineageRejected` explicitement remplacée.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causal_supersedes: Option<CommandId>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrelationStoreSnapshot {
    schema_version: u32,
    entries: Vec<CorrelatedCommand>,
}

/// File locale de corrélation des `CommandEnvelope` (en mémoire ou persistée atomiquement sur disque).
#[derive(Debug)]
pub struct CommandCorrelationQueue {
    backing_path: Option<PathBuf>,
    entries: BTreeMap<CommandId, CorrelatedCommand>,
}

impl CommandCorrelationQueue {
    /// Crée une file de corrélation purement en mémoire (utile pour tests unitaires ou session
    /// éphémère CLI sans persistance fichier).
    #[must_use]
    pub const fn in_memory() -> Self {
        Self {
            backing_path: None,
            entries: BTreeMap::new(),
        }
    }

    /// Ouvre une file de corrélation existante ou crée un fichier persistant atomique à `path`.
    ///
    /// # Errors
    /// - [`ClientError::Platform`] si `path` touche `.claude/`.
    /// - [`ClientError::InvalidCorrelationStore`] si le format, la version de schéma ou un digest
    ///   canonique stocké est invalide/corrompu.
    /// - [`ClientError::Io`] en cas d'erreur d'accès disque.
    pub fn open_or_create(path: PathBuf) -> Result<Self, ClientError> {
        assert_not_under_claude(&path)?;

        if path
            .parent()
            .is_none_or(|p| p.as_os_str().is_empty() && path.file_name().is_none())
        {
            return Err(ClientError::InvalidCorrelationStore {
                reason: String::from("Le chemin de file doit avoir un répertoire parent"),
            });
        }

        let mut entries = BTreeMap::new();
        if let Some(raw) = read_private_file_to_string(&path)? {
            let snapshot: CorrelationStoreSnapshot =
                from_json_str_strict(&raw).map_err(|e| ClientError::InvalidCorrelationStore {
                    reason: format!("JSON strict invalide : {e}"),
                })?;
            if snapshot.schema_version != CORRELATION_SCHEMA_VERSION {
                return Err(ClientError::InvalidCorrelationStore {
                    reason: format!("schema_version non supporté : {}", snapshot.schema_version),
                });
            }
            for entry in snapshot.entries {
                let recomputed = compute_command_digest_v2(&entry.envelope)?;
                if recomputed != entry.command_digest {
                    return Err(ClientError::InvalidCorrelationStore {
                        reason: format!(
                            "Digest corrompu pour command_id {}",
                            entry.envelope.command_id
                        ),
                    });
                }
                if entries.insert(entry.envelope.command_id, entry).is_some() {
                    return Err(ClientError::InvalidCorrelationStore {
                        reason: String::from("Entrée dupliquée par command_id sur disque"),
                    });
                }
            }
        } else {
            let queue = Self {
                backing_path: Some(path),
                entries,
            };
            queue.persist_if_backed()?;
            return Ok(queue);
        }

        Ok(Self {
            backing_path: Some(path),
            entries,
        })
    }

    /// Enregistre durablement une nouvelle `CommandEnvelope` dans la file locale.
    ///
    /// Si le même `command_id` existe déjà avec le même digest canonique, l'opération est
    /// idempotente ; si le digest diffère, retourne [`ClientError::LocalIdempotencyConflict`].
    ///
    /// # Errors
    /// Retourne [`ClientError::LocalIdempotencyConflict`] sur digest divergent, ou erreur
    /// de canonisation/persistance.
    pub fn enqueue(&mut self, envelope: CommandEnvelope, now_ms: i64) -> Result<(), ClientError> {
        self.enqueue_with_causal_link(envelope, now_ms, None)
    }

    /// Variante de [`Self::enqueue`] enregistrant un lien causal explicite vers une commande
    /// antérieure rejetée pour lignée obsolète (`causal_supersedes`).
    ///
    /// # Errors
    /// Identique à [`Self::enqueue`].
    pub fn enqueue_with_causal_link(
        &mut self,
        envelope: CommandEnvelope,
        now_ms: i64,
        causal_supersedes: Option<CommandId>,
    ) -> Result<(), ClientError> {
        let digest = compute_command_digest_v2(&envelope)?;
        let cmd_id = envelope.command_id;

        if let Some(existing) = self.entries.get(&cmd_id) {
            if existing.command_digest == digest {
                return Ok(());
            }
            return Err(ClientError::LocalIdempotencyConflict { command_id: cmd_id });
        }

        let entry = CorrelatedCommand {
            envelope,
            command_digest: digest,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            state: CorrelationState::Pending { attempts: 0 },
            causal_supersedes,
        };
        self.entries.insert(cmd_id, entry);
        self.persist_if_backed()
    }

    /// Prépare une tentative de transmission (ou de retry après reconnexion) pour `command_id`.
    ///
    /// - Génère/applique le `fresh_request_id` sans **jamais** modifier `authority_generation`,
    ///   `command_id` ni le payload.
    /// - Persiste l'état [`CorrelationState::Uncertain`] **avant** émission sur le socket.
    ///
    /// # Errors
    /// - [`ClientError::UnknownCommand`] si absente.
    /// - [`ClientError::StaleLineageLocked`] si la commande a déjà été refusée avec `STALE_AUTHORITY_GENERATION`.
    /// - [`ClientError::AlreadyTerminal`] si déjà confirmée ou échouée définitivement.
    pub fn prepare_attempt(
        &mut self,
        command_id: CommandId,
        fresh_request_id: RequestId,
        now_ms: i64,
    ) -> Result<CommandEnvelope, ClientError> {
        let prepared_envelope = {
            let entry = self
                .entries
                .get_mut(&command_id)
                .ok_or(ClientError::UnknownCommand { command_id })?;

            match &entry.state {
                CorrelationState::StaleLineageRejected { .. } => {
                    return Err(ClientError::StaleLineageLocked {
                        command_id,
                        pinned_generation: entry.envelope.authority_generation,
                    });
                }
                CorrelationState::Confirmed { .. } | CorrelationState::Failed { .. } => {
                    return Err(ClientError::AlreadyTerminal { command_id });
                }
                CorrelationState::Pending { attempts }
                | CorrelationState::Uncertain { attempts, .. } => {
                    let next_attempts = attempts.saturating_add(1);
                    entry.envelope.request_id = fresh_request_id;
                    entry.updated_at_ms = now_ms;
                    entry.state = CorrelationState::Uncertain {
                        attempts: next_attempts,
                        last_request_id: fresh_request_id,
                    };
                    entry.envelope.clone()
                }
            }
        };

        self.persist_if_backed()?;
        Ok(prepared_envelope)
    }

    /// Enregistre le reçu durable [`CommandReceipt`] retourné par le daemon.
    ///
    /// # Errors
    /// Retourne [`ClientError::StaleLineageLocked`] si la commande était déjà verrouillée sur
    /// conflit de lignée, ou [`ClientError::CorrelationMismatch`] si `receipt.command_id != command_id`.
    pub fn record_receipt(
        &mut self,
        command_id: CommandId,
        receipt: CommandReceipt,
        now_ms: i64,
    ) -> Result<(), ClientError> {
        if receipt.command_id != command_id {
            return Err(ClientError::CorrelationMismatch {
                expected: command_id.to_string(),
                actual: receipt.command_id.to_string(),
            });
        }
        {
            let entry = self
                .entries
                .get_mut(&command_id)
                .ok_or(ClientError::UnknownCommand { command_id })?;
            if matches!(entry.state, CorrelationState::StaleLineageRejected { .. }) {
                return Err(ClientError::StaleLineageLocked {
                    command_id,
                    pinned_generation: entry.envelope.authority_generation,
                });
            }
            entry.updated_at_ms = now_ms;
            entry.state = CorrelationState::Confirmed { receipt };
        }
        self.persist_if_backed()
    }

    /// Enregistre une erreur [`IpcError`] retournée par le daemon pour `command_id`.
    ///
    /// - Si `StaleFence / STALE_AUTHORITY_GENERATION`, verrouille définitivement l'entrée dans
    ///   [`CorrelationState::StaleLineageRejected`] sans altérer `authority_generation` ni `command_id`.
    /// - Si `retry_class == AfterBackoff`, maintient l'état [`CorrelationState::Uncertain`].
    /// - Sinon, passe en [`CorrelationState::Failed`].
    ///
    /// # Errors
    /// Retourne [`ClientError::UnknownCommand`] si absente, [`ClientError::StaleLineageLocked`]
    /// si l'entrée est déjà verrouillée en [`CorrelationState::StaleLineageRejected`], ou une
    /// erreur de persistance.
    pub fn record_error(
        &mut self,
        command_id: CommandId,
        error: IpcError,
        observed_daemon_generation: Option<AuthorityGeneration>,
        now_ms: i64,
    ) -> Result<(), ClientError> {
        {
            let entry = self
                .entries
                .get_mut(&command_id)
                .ok_or(ClientError::UnknownCommand { command_id })?;
            if matches!(entry.state, CorrelationState::StaleLineageRejected { .. }) {
                return Err(ClientError::StaleLineageLocked {
                    command_id,
                    pinned_generation: entry.envelope.authority_generation,
                });
            }
            entry.updated_at_ms = now_ms;

            if error.code == ErrorCode::StaleFence
                && error.reason_code == Some(ReasonCode::StaleAuthorityGeneration)
            {
                entry.state = CorrelationState::StaleLineageRejected {
                    error,
                    observed_daemon_generation,
                };
            } else if error.retry_class == RetryClass::AfterBackoff {
                let attempts = match entry.state {
                    CorrelationState::Pending { attempts }
                    | CorrelationState::Uncertain { attempts, .. } => attempts,
                    _ => 1,
                };
                entry.state = CorrelationState::Uncertain {
                    attempts,
                    last_request_id: entry.envelope.request_id,
                };
            } else {
                entry.state = CorrelationState::Failed { error };
            }
        }
        self.persist_if_backed()
    }

    /// Construit explicitement une **nouvelle** intention de reprise suite à un refus
    /// `STALE_AUTHORITY_GENERATION`, dotée d'un nouveau `command_id` et de la nouvelle lignée,
    /// en retournant l'identifiant causal `stale_command_id` sans jamais altérer l'entrée d'origine.
    ///
    /// # Errors
    /// - [`ClientError::UnknownCommand`] si `stale_command_id` n'existe pas.
    /// - [`ClientError::UnexpectedResponse`] si la commande source n'est pas `StaleLineageRejected`
    ///   ou si `new_command_id == stale_command_id`.
    pub fn create_explicit_recovery_intention(
        &self,
        stale_command_id: CommandId,
        new_command_id: CommandId,
        new_request_id: RequestId,
        new_generation: AuthorityGeneration,
    ) -> Result<(CommandEnvelope, CommandId), ClientError> {
        if new_command_id == stale_command_id {
            return Err(ClientError::UnexpectedResponse {
                expected: "nouveau command_id",
                detail: String::from(
                    "Une nouvelle intention de récupération exige un nouveau command_id distinct",
                ),
            });
        }
        let old = self
            .entries
            .get(&stale_command_id)
            .ok_or(ClientError::UnknownCommand {
                command_id: stale_command_id,
            })?;
        if !matches!(old.state, CorrelationState::StaleLineageRejected { .. }) {
            return Err(ClientError::UnexpectedResponse {
                expected: "StaleLineageRejected",
                detail: format!("La commande {stale_command_id} n'est pas en conflit de lignée"),
            });
        }

        let mut new_env = old.envelope.clone();
        new_env.command_id = new_command_id;
        new_env.request_id = new_request_id;
        new_env.authority_generation = new_generation;
        Ok((new_env, stale_command_id))
    }

    /// Consulte une entrée par `command_id`.
    #[must_use]
    pub fn get(&self, command_id: CommandId) -> Option<&CorrelatedCommand> {
        self.entries.get(&command_id)
    }

    /// Liste toutes les entrées suivies dans la file.
    pub fn entries(&self) -> impl Iterator<Item = &CorrelatedCommand> {
        self.entries.values()
    }

    fn persist_if_backed(&self) -> Result<(), ClientError> {
        let Some(ref path) = self.backing_path else {
            return Ok(());
        };
        let snapshot = CorrelationStoreSnapshot {
            schema_version: CORRELATION_SCHEMA_VERSION,
            entries: self.entries.values().cloned().collect(),
        };
        let bytes =
            serde_json::to_vec_pretty(&snapshot).map_err(unleash_protocol::ProtocolError::Json)?;
        write_private_file_atomic(path, &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, json};
    use std::os::unix::fs::symlink;
    use unleash_domain::{ProjectId, Revision};
    use unleash_protocol::ProtocolVersion;
    use unleash_testkit::local_test_dir;
    use uuid::Uuid;

    fn sample_envelope(cmd_id: CommandId, reason: &str) -> CommandEnvelope {
        let mut payload = Map::new();
        payload.insert(String::from("reason"), json!(reason));
        CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(1),
            command_id: cmd_id,
            authority_generation: AuthorityGeneration::from_u128(0x100),
            actor_ref: None,
            project_id: Some(ProjectId::from_u128(0x200)),
            aggregate_id: Uuid::from_u128(0x300),
            expected_revision: Revision::new(1),
            command_type: String::from("PauseMission"),
            payload,
        }
    }

    #[test]
    fn queue_rejects_divergent_payload_and_tampered_disk_digest() {
        let dir = local_test_dir("corr-unit").unwrap();
        let path = dir.path().join("queue.json");
        let cmd_id = CommandId::from_u128(0x1111);

        let mut q = CommandCorrelationQueue::open_or_create(path.clone()).unwrap();
        q.enqueue(sample_envelope(cmd_id, "initial"), 10).unwrap();
        // Idempotent avec même digest
        q.enqueue(sample_envelope(cmd_id, "initial"), 11).unwrap();
        // Rejet si payload divergent
        assert!(matches!(
            q.enqueue(sample_envelope(cmd_id, "modifie"), 12),
            Err(ClientError::LocalIdempotencyConflict { .. })
        ));

        // Altérer le payload sur disque sans mettre à jour command_digest => détection corruption
        let raw = std::fs::read_to_string(&path).unwrap();
        let tampered = raw.replace("\"initial\"", "\"pirate\"");
        std::fs::write(&path, tampered).unwrap();
        assert!(matches!(
            CommandCorrelationQueue::open_or_create(path),
            Err(ClientError::InvalidCorrelationStore { .. })
        ));

        // Rejet lien symbolique
        let real_file = dir.path().join("real.json");
        std::fs::write(&real_file, "{}").unwrap();
        let link_path = dir.path().join("link.json");
        symlink(&real_file, &link_path).unwrap();
        assert!(CommandCorrelationQueue::open_or_create(link_path).is_err());
    }

    #[test]
    fn record_error_refuses_to_overwrite_stale_lineage_rejected() {
        let mut q = CommandCorrelationQueue::in_memory();
        let cmd_id = CommandId::from_u128(0x2222);
        let env = sample_envelope(cmd_id, "pinned");
        let pinned_gen = env.authority_generation;
        q.enqueue(env, 100).unwrap();

        let observed_gen = AuthorityGeneration::from_u128(0x999);
        let stale_err = IpcError::new(
            ErrorCode::StaleFence,
            Some(ReasonCode::StaleAuthorityGeneration),
            "génération obsolète",
            RetryClass::Never,
        );
        q.record_error(cmd_id, stale_err, Some(observed_gen), 200)
            .unwrap();

        let retryable_err = IpcError::new(
            ErrorCode::ResourceBusy,
            None,
            "temporaire",
            RetryClass::AfterBackoff,
        );
        assert!(matches!(
            q.record_error(cmd_id, retryable_err, None, 300),
            Err(ClientError::StaleLineageLocked {
                command_id,
                pinned_generation,
            }) if command_id == cmd_id && pinned_generation == pinned_gen
        ));

        let permanent_err = IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "tardif",
            RetryClass::Never,
        );
        assert!(matches!(
            q.record_error(cmd_id, permanent_err, None, 400),
            Err(ClientError::StaleLineageLocked {
                command_id,
                pinned_generation,
            }) if command_id == cmd_id && pinned_generation == pinned_gen
        ));

        let entry = q.get(cmd_id).unwrap();
        assert_eq!(entry.updated_at_ms, 200);
        assert!(matches!(
            entry.state,
            CorrelationState::StaleLineageRejected {
                observed_daemon_generation: Some(obs),
                ..
            } if obs == observed_gen
        ));
    }
}
