// SPDX-License-Identifier: Apache-2.0
//! Protocole privé authentifié entre le daemon Unleash et `unleash-process-host`.
//!
//! Utilise exclusivement les trames préfixées par longueur (8 MiB max) de `unleash_protocol::codec`
//! sur socket Unix privé `0600` dans un répertoire `0700`. Aucun secret n'est affiché dans `Debug`.

use core::fmt;
use serde::{Deserialize, Serialize};
use unleash_domain::{AuthorityGeneration, DaemonEpoch, Fence, ProcessRunUuid, TimestampMs};
use unleash_platform::OsProcessIdentity;
use unleash_protocol::IpcError;

use crate::anchor::{AnchorVerificationReport, ChildExitStatus, StopObservation};
use crate::fencing::HostCommandHeader;
use crate::manifest::HostLaunchState;
use crate::pty_input::PtyInputReceipt;
use crate::spool::LogReadResult;

/// Requête privée envoyée par le daemon au process-host (`#[serde(deny_unknown_fields)]`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostRequest {
    /// Handshake initial ou de récupération après crash daemon dans la même lignée (`REQ-TERM-003`).
    Handshake {
        /// En-tête de lignée, époque et fence.
        header: HostCommandHeader,
        /// Secret de contrôle courant authentifiant le daemon.
        current_secret: String,
        /// Nouveau secret optionnel pour rotation atomique de capacité.
        rotate_to_secret: Option<String>,
        /// Exige un `daemon_epoch` strictement supérieur au dernier accepté (cas récupération).
        require_higher_epoch: bool,
    },
    /// Envoie une entrée numérotée au maître PTY / stdin avec reçu durable (`REQ-TERM-005`).
    SendInput {
        /// En-tête de lignée, époque et fence.
        header: HostCommandHeader,
        /// Secret de contrôle actif.
        secret: String,
        /// Numéro de séquence monotone (1-based) de l'entrée.
        input_seq: u64,
        /// Octets exacts à écrire au maître PTY.
        payload_bytes: Vec<u8>,
        /// Horodatage UTC courant fourni par l'autorité.
        now_ms: TimestampMs,
    },
    /// Redimensionne le PTY et renvoie la taille observée après `ioctl` (`F011`).
    Resize {
        /// En-tête de lignée, époque et fence.
        header: HostCommandHeader,
        /// Secret de contrôle actif.
        secret: String,
        /// Nombre de lignes demandé (> 0).
        rows: u16,
        /// Nombre de colonnes demandé (> 0).
        cols: u16,
    },
    /// Lit les segments de logs et octets volatils depuis `after_offset` (`REQ-TERM-008`).
    ReadLog {
        /// En-tête de lignée, époque et fence.
        header: HostCommandHeader,
        /// Secret de contrôle actif.
        secret: String,
        /// Curseur d'offset de départ.
        after_offset: u64,
        /// Plafond d'octets à retourner.
        max_bytes: usize,
        /// Si vrai, retourne `CURSOR_EXPIRED` lorsque `after_offset < oldest_available_offset`.
        strict_cursor: bool,
    },
    /// Arrêt contrôlé de l'enfant et de son groupe sous ancrage non récolté (`REQ-TERM-006`).
    Stop {
        /// En-tête de lignée, époque et fence.
        header: HostCommandHeader,
        /// Secret de contrôle actif.
        secret: String,
        /// Si vrai, escalade à `SIGKILL` sur le groupe avant récolte si nécessaire.
        escalate_to_sigkill: bool,
        /// PIDs de descendants connus par observation pour rapporter un éventuel détachement.
        known_descendant_pids: Vec<u32>,
    },
    /// Réconciliation et observation d'identité/statut sans mutation (`F011` / `F018`).
    Reconcile {
        /// En-tête de lignée, époque et fence.
        header: HostCommandHeader,
        /// Secret de contrôle actif.
        secret: String,
    },
    /// Requête de test prouvant qu'un process-host refuse toute invocation modèle autonome (`REQ-TERM-004`).
    ModelCallAttempt {
        /// En-tête de lignée, époque et fence.
        header: HostCommandHeader,
        /// Secret de contrôle actif.
        secret: String,
        /// Description de l'appel tenté.
        prompt_summary: String,
    },
    /// Ferme proprement le serveur de contrôle après drainage et persistance finale.
    Shutdown {
        /// En-tête de lignée, époque et fence.
        header: HostCommandHeader,
        /// Secret de contrôle actif.
        secret: String,
    },
}

impl fmt::Debug for HostRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handshake {
                header,
                rotate_to_secret,
                require_higher_epoch,
                ..
            } => f
                .debug_struct("Handshake")
                .field("header", header)
                .field("current_secret", &"[REDACTED]")
                .field(
                    "rotate_to_secret",
                    &rotate_to_secret.as_ref().map(|_| "[REDACTED]"),
                )
                .field("require_higher_epoch", require_higher_epoch)
                .finish(),
            Self::SendInput {
                header,
                input_seq,
                payload_bytes,
                now_ms,
                ..
            } => f
                .debug_struct("SendInput")
                .field("header", header)
                .field("secret", &"[REDACTED]")
                .field("input_seq", input_seq)
                .field("payload_len", &payload_bytes.len())
                .field("now_ms", now_ms)
                .finish(),
            Self::Resize {
                header, rows, cols, ..
            } => f
                .debug_struct("Resize")
                .field("header", header)
                .field("secret", &"[REDACTED]")
                .field("rows", rows)
                .field("cols", cols)
                .finish(),
            Self::ReadLog {
                header,
                after_offset,
                max_bytes,
                strict_cursor,
                ..
            } => f
                .debug_struct("ReadLog")
                .field("header", header)
                .field("secret", &"[REDACTED]")
                .field("after_offset", after_offset)
                .field("max_bytes", max_bytes)
                .field("strict_cursor", strict_cursor)
                .finish(),
            Self::Stop {
                header,
                escalate_to_sigkill,
                known_descendant_pids,
                ..
            } => f
                .debug_struct("Stop")
                .field("header", header)
                .field("secret", &"[REDACTED]")
                .field("escalate_to_sigkill", escalate_to_sigkill)
                .field("known_descendant_pids", known_descendant_pids)
                .finish(),
            Self::Reconcile { header, .. } => f
                .debug_struct("Reconcile")
                .field("header", header)
                .field("secret", &"[REDACTED]")
                .finish(),
            Self::ModelCallAttempt {
                header,
                prompt_summary,
                ..
            } => f
                .debug_struct("ModelCallAttempt")
                .field("header", header)
                .field("secret", &"[REDACTED]")
                .field("prompt_summary", prompt_summary)
                .finish(),
            Self::Shutdown { header, .. } => f
                .debug_struct("Shutdown")
                .field("header", header)
                .field("secret", &"[REDACTED]")
                .finish(),
        }
    }
}

/// Réponse privée retournée par `unleash-process-host` au daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostResponse {
    /// Accusé du handshake authentifié et état courant après fencing/rotation.
    HandshakeAck {
        /// Effective explicit environment verified before spawn; absent on legacy hosts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effective_environment_digest: Option<unleash_domain::Sha256Digest>,
        /// Allocation link supplied by the authenticated launcher.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allocation_context_digest: Option<unleash_domain::Sha256Digest>,
        /// UUID de l'exécution physique.
        process_run_uuid: ProcessRunUuid,
        /// Lignée active confirmée.
        authority_generation: AuthorityGeneration,
        /// Plus haute époque acceptée.
        daemon_epoch: DaemonEpoch,
        /// Plus haut fence accepté.
        highest_fence: Fence,
        /// Identité OS de l'enfant capturée au lancement.
        os_identity: OsProcessIdentity,
        /// Plus haut offset durable committé dans le spool.
        durable_log_seq: u64,
        /// Dernière séquence d'entrée confirmée.
        last_confirmed_input_seq: u64,
        /// Séquences d'entrée trouvées en vol et classées `OutcomeUnknown`.
        unknown_input_seqs: Vec<u64>,
        /// Indique si le secret de contrôle a été tourné lors de ce handshake.
        secret_rotated: bool,
    },
    /// Reçu durable d'une écriture PTY numérotée.
    InputReceipt {
        /// Détail du reçu.
        receipt: PtyInputReceipt,
    },
    /// Taille effectivement observée sur le maître PTY après `TIOCSWINSZ`.
    ResizeAck {
        /// Nombre de lignes observé.
        rows: u16,
        /// Nombre de colonnes observé.
        cols: u16,
    },
    /// Tranche de logs retournée par `ReadLog`.
    LogChunk {
        /// Segments durables, octets volatils et gaps.
        result: LogReadResult,
    },
    /// Observation produite par `Stop`.
    StopObserved {
        /// Détail de l'arrêt et de l'observation des descendants.
        observation: StopObservation,
    },
    /// État complet retourné par `Reconcile`.
    ReconcileState {
        /// État de lancement courant.
        launch_state: HostLaunchState,
        /// Diagnostic d'ancrage.
        anchor: AnchorVerificationReport,
        /// État courant de vérification OS.
        os_verification: String,
        /// Plus haut offset durable dans le spool.
        durable_log_seq: u64,
        /// Dernière séquence d'entrée confirmée.
        last_confirmed_input_seq: u64,
        /// Séquences d'entrée ambiguës (`OutcomeUnknown`).
        unknown_input_seqs: Vec<u64>,
        /// Statut de sortie si déjà observé/récolté.
        exit_status: Option<ChildExitStatus>,
    },
    /// Accusé de fermeture propre du serveur host.
    ShutdownAck,
    /// Erreur canonique Unleash sérialisée au format [`IpcError`].
    Error {
        /// Erreur projetée.
        error: IpcError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use unleash_domain::{ActionId, CommandId, Sha256Digest};

    #[test]
    fn host_request_debug_never_leaks_secrets() {
        let header = HostCommandHeader {
            protocol_version: 1,
            process_run_uuid: ProcessRunUuid::from_u128(1),
            authority_generation: AuthorityGeneration::from_u128(2),
            daemon_epoch: DaemonEpoch::new(1),
            resource_fence: Fence::new(1),
            command_id: CommandId::from_u128(3),
            action_id: ActionId::from_u128(4),
            payload_hash: Sha256Digest::from_bytes(b"p"),
        };
        let req = HostRequest::Handshake {
            header,
            current_secret: String::from("ULTRA_SECRET_TOKEN_OLD"),
            rotate_to_secret: Some(String::from("ULTRA_SECRET_TOKEN_NEW")),
            require_higher_epoch: true,
        };
        let dbg = format!("{req:?}");
        assert!(!dbg.contains("ULTRA_SECRET_TOKEN_OLD"));
        assert!(!dbg.contains("ULTRA_SECRET_TOKEN_NEW"));
        assert!(dbg.contains("[REDACTED]"));
    }

    proptest::proptest! {
        #[test]
        fn prop_host_request_debug_never_leaks_random_secrets_and_roundtrips(
            s_a in "[A-Za-z0-9_-]{8,24}",
            s_b in "[A-Za-z0-9_-]{8,24}",
            seq in 1_u64..=10_000,
            rows in 1_u16..=500,
            cols in 1_u16..=500,
            epoch in 1_u64..=500,
            fence in 1_u64..=500,
        ) {
            let secret = format!("sec-cur-{s_a}");
            let next_secret = format!("sec-rot-{s_b}");
            let header = HostCommandHeader {
                protocol_version: 1,
                process_run_uuid: ProcessRunUuid::from_u128(11),
                authority_generation: AuthorityGeneration::from_u128(22),
                daemon_epoch: DaemonEpoch::new(epoch),
                resource_fence: Fence::new(fence),
                command_id: CommandId::from_u128(u128::from(seq)),
                action_id: ActionId::from_u128(u128::from(seq) + 1),
                payload_hash: Sha256Digest::from_bytes(secret.as_bytes()),
            };
            let variants = [
                HostRequest::Handshake {
                    header: header.clone(),
                    current_secret: secret.clone(),
                    rotate_to_secret: Some(next_secret.clone()),
                    require_higher_epoch: true,
                },
                HostRequest::SendInput {
                    header: header.clone(),
                    secret: secret.clone(),
                    input_seq: seq,
                    payload_bytes: vec![b'a', b'\n'],
                    now_ms: TimestampMs::new(i64::try_from(seq * 10).unwrap()),
                },
                HostRequest::Resize {
                    header: header.clone(),
                    secret: secret.clone(),
                    rows,
                    cols,
                },
                HostRequest::ReadLog {
                    header: header.clone(),
                    secret: secret.clone(),
                    after_offset: seq,
                    max_bytes: usize::from(cols) * 16,
                    strict_cursor: true,
                },
                HostRequest::Stop {
                    header: header.clone(),
                    secret: secret.clone(),
                    escalate_to_sigkill: true,
                    known_descendant_pids: vec![u32::from(rows)],
                },
                HostRequest::Reconcile {
                    header: header.clone(),
                    secret: secret.clone(),
                },
                HostRequest::Shutdown {
                    header,
                    secret: secret.clone(),
                },
            ];
            for req in variants {
                let dbg = format!("{req:?}");
                proptest::prop_assert!(!dbg.contains(&secret));
                proptest::prop_assert!(!dbg.contains(&next_secret));
                let json = serde_json::to_string(&req).unwrap();
                let decoded: HostRequest = serde_json::from_str(&json).unwrap();
                proptest::prop_assert_eq!(decoded, req);
                let tampered = format!("{},\"unexpected_unknown_field\":1}}", json.trim_end_matches('}'));
                proptest::prop_assert!(serde_json::from_str::<HostRequest>(&tampered).is_err());
            }
        }
    }
}
