// SPDX-License-Identifier: Apache-2.0
//! Spool de logs borné par segments avec redaction de secrets canaris, watermark durable vs
//! octets volatils, empreintes SHA-256 et gaps explicites (`REQ-TERM-008`, `F011`).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unleash_domain::Sha256Digest;
use unleash_platform::{
    assert_not_under_claude, ensure_private_rendezvous_dir, read_private_file_to_string,
    write_private_file_atomic,
};
use zeroize::Zeroize;

use crate::error::ProcessHostError;

/// Nom du fichier d'index durable du spool dans `spool_dir`.
pub const SPOOL_INDEX_FILENAME: &str = "spool-index.json";

const REDACTED_MARKER: &[u8] = b"[REDACTED]";

/// Configuration des bornes mémoire et disque du spool de logs (`REQ-TERM-008`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpoolConfig {
    /// Taille maximale en octets avant scellement d'un segment durable sur disque.
    pub max_segment_bytes: usize,
    /// Plafond total en octets conservés sur disque pour l'ensemble des segments scellés.
    pub max_total_bytes: usize,
    /// Plafond du tampon mémoire volatil (non encore scellé).
    pub max_volatile_bytes: usize,
}

impl Default for SpoolConfig {
    fn default() -> Self {
        Self {
            max_segment_bytes: 256 * 1024,
            max_total_bytes: 32 * 1024 * 1024,
            max_volatile_bytes: 256 * 1024,
        }
    }
}

/// Filtre de masquage des secrets canaris, sûr face aux secrets multi-lignes ou coupés entre
/// deux lectures consécutives du PTY (`security-boundaries.md`).
pub struct SecretRedactor {
    canaries: Vec<Vec<u8>>,
    holdback: Vec<u8>,
    max_canary_len: usize,
    redaction_count: u64,
}

impl Drop for SecretRedactor {
    fn drop(&mut self) {
        for c in &mut self.canaries {
            c.zeroize();
        }
        self.holdback.zeroize();
    }
}

impl std::fmt::Debug for SecretRedactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretRedactor")
            .field("canary_count", &self.canaries.len())
            .field("holdback_len", &self.holdback.len())
            .field("redaction_count", &self.redaction_count)
            .finish_non_exhaustive()
    }
}

impl SecretRedactor {
    /// Construit un filtre de redaction à partir des secrets canaris fournis.
    ///
    /// Pour chaque canari multi-ligne contenant `\n`, ajoute également la variante `\r\n` afin
    /// de couvrir la discipline de ligne PTY (`ONLCR`).
    #[must_use]
    pub fn new(secrets: &[SecretString]) -> Self {
        let mut canaries: Vec<Vec<u8>> = Vec::new();
        for secret in secrets {
            let raw = secret.expose_secret().as_bytes();
            if raw.is_empty() {
                continue;
            }
            canaries.push(raw.to_vec());
            if raw.contains(&b'\n') && !raw.windows(2).any(|w| w == b"\r\n") {
                let mut crlf = Vec::with_capacity(raw.len().saturating_add(8));
                for &b in raw {
                    if b == b'\n' {
                        crlf.push(b'\r');
                    }
                    crlf.push(b);
                }
                canaries.push(crlf);
            }
        }
        // Trier du plus long au plus court pour masquer prioritairement le secret le plus complet.
        canaries.sort_by_key(|c| std::cmp::Reverse(c.len()));
        canaries.dedup();
        let max_canary_len = canaries.iter().map(Vec::len).max().unwrap_or(0);
        Self {
            canaries,
            holdback: Vec::new(),
            max_canary_len,
            redaction_count: 0,
        }
    }

    /// Nombre total d'occurrences masquées depuis la création.
    #[must_use]
    pub const fn redaction_count(&self) -> u64 {
        self.redaction_count
    }

    /// Filtre un fragment d'octets entrant et retourne les octets sûrs immédiatement exposables.
    ///
    /// Les octets terminaux correspondant à un préfixe strict d'un canari restent en retenue
    /// jusqu'au fragment suivant ou jusqu'à [`Self::flush`].
    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.process(chunk, false)
    }

    /// Vide le tampon de retenue en fin de flux (EOF) après ultime masquage.
    pub fn flush(&mut self) -> Vec<u8> {
        self.process(&[], true)
    }

    fn process(&mut self, chunk: &[u8], is_eof: bool) -> Vec<u8> {
        if self.canaries.is_empty() {
            if self.holdback.is_empty() {
                return chunk.to_vec();
            }
            let mut out = std::mem::take(&mut self.holdback);
            out.extend_from_slice(chunk);
            return out;
        }

        let mut work = Vec::with_capacity(self.holdback.len().saturating_add(chunk.len()));
        work.extend_from_slice(&self.holdback);
        work.extend_from_slice(chunk);
        self.holdback.zeroize();
        self.holdback.clear();

        let mut out = Vec::with_capacity(work.len());
        let mut pos = 0_usize;

        while pos < work.len() {
            let Some(rest) = work.get(pos..) else {
                break;
            };

            if let Some(matched_len) = self.find_match_at_start(rest) {
                out.extend_from_slice(REDACTED_MARKER);
                self.redaction_count = self.redaction_count.saturating_add(1);
                pos = pos.saturating_add(matched_len);
                continue;
            }

            if !is_eof && rest.len() < self.max_canary_len && self.is_strict_prefix_of_any(rest) {
                self.holdback.extend_from_slice(rest);
                break;
            }

            if let Some(&b) = work.get(pos) {
                out.push(b);
            }
            pos = pos.saturating_add(1);
        }

        work.zeroize();
        out
    }

    fn find_match_at_start(&self, rest: &[u8]) -> Option<usize> {
        for needle in &self.canaries {
            if rest.starts_with(needle) {
                return Some(needle.len());
            }
        }
        None
    }

    fn is_strict_prefix_of_any(&self, tail: &[u8]) -> bool {
        if tail.is_empty() {
            return false;
        }
        self.canaries
            .iter()
            .any(|needle| tail.len() < needle.len() && needle.starts_with(tail))
    }
}

/// Métadonnées d'un segment durable scellé et synchronisé (`fsync`) sur disque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogSegmentMeta {
    /// Numéro de séquence monotone (1-based) du segment.
    pub segment_seq: u64,
    /// Nom relatif du fichier segment dans `spool_dir`.
    pub filename: String,
    /// Offset cumulé de début (inclus).
    pub start_offset: u64,
    /// Offset cumulé de fin (exclusif).
    pub end_offset: u64,
    /// Taille du segment en octets.
    pub byte_len: u64,
    /// Empreinte SHA-256 des octets exacts (après redaction) du segment.
    pub sha256: Sha256Digest,
}

/// Trou explicite dans les logs suite à la rotation par saturation de quota (`REQ-TERM-008`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogGap {
    /// Premier segment supprimé.
    pub from_segment_seq: u64,
    /// Dernier segment supprimé (inclus).
    pub to_segment_seq: u64,
    /// Premier offset supprimé (inclus).
    pub from_offset: u64,
    /// Dernier offset supprimé (exclusif).
    pub to_offset: u64,
    /// Nombre d'octets écartés par la rotation.
    pub dropped_bytes: u64,
    /// Raison explicite du gap (`quota_rotation`).
    pub reason: String,
}

/// Index durable persisté dans `spool-index.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpoolIndex {
    /// Version du schéma d'index (`1`).
    pub schema_version: u32,
    /// Prochain numéro de segment à émettre.
    pub next_segment_seq: u64,
    /// Plus haut offset durable synchronisé sur disque (`fsync`).
    pub durable_watermark_offset: u64,
    /// Plus haut numéro de segment durable synchronisé sur disque.
    pub durable_watermark_seq: u64,
    /// Plus petit offset encore disponible sur disque.
    pub oldest_available_offset: u64,
    /// Nombre d'occurrences masquées.
    pub redaction_count: u64,
    /// Liste ordonnée des segments scellés encore présents sur disque.
    pub segments: Vec<LogSegmentMeta>,
    /// Liste ordonnée des gaps enregistrés lors des rotations.
    pub gaps: Vec<LogGap>,
}

/// Tranche de logs renvoyée à un lecteur (`ReadTerminalLog`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogReadSegment {
    /// Numéro de segment (`Some(seq)` si durable, `None` si queue volatile non encore scellée).
    pub segment_seq: Option<u64>,
    /// Offset de début (inclus).
    pub start_offset: u64,
    /// Offset de fin (exclusif).
    pub end_offset: u64,
    /// Empreinte SHA-256 si segment durable scellé (`None` si volatil).
    pub sha256: Option<Sha256Digest>,
    /// Indique explicitement si ces octets sont volatils (non encore `fsync` sur disque).
    pub is_volatile: bool,
    /// Octets déjà nettoyés par redaction (encodés en base64 ou UTF-8 lossy selon vue ; ici octets bruts).
    pub bytes: Vec<u8>,
}

/// Résultat complet d'une lecture bornée du spool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogReadResult {
    /// Watermark durable (séquence de segment).
    pub durable_watermark_seq: u64,
    /// Watermark durable (offset cumulé en octets).
    pub durable_watermark_offset: u64,
    /// Plus ancien offset encore présent dans le spool.
    pub oldest_available_offset: u64,
    /// Plus haut offset émis (incluant la queue volatile éventuelle).
    pub volatile_head_offset: u64,
    /// Nombre total de masquages appliqués.
    pub redaction_count: u64,
    /// Gaps couvrant les segments déjà purgés par quota.
    pub gaps: Vec<LogGap>,
    /// Segments/tranches retournés à partir du curseur demandé.
    pub segments: Vec<LogReadSegment>,
}

struct SpoolInner {
    spool_dir: PathBuf,
    index_path: PathBuf,
    config: SpoolConfig,
    redactor: SecretRedactor,
    index: SpoolIndex,
    volatile_tail: Vec<u8>,
}

/// Spool de logs thread-safe et borné par segments durables (`REQ-TERM-008`).
pub struct LogSpool {
    inner: Mutex<SpoolInner>,
}

impl std::fmt::Debug for LogSpool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f.debug_struct("LogSpool")
            .field("spool_dir", &guard.spool_dir)
            .field(
                "durable_watermark_offset",
                &guard.index.durable_watermark_offset,
            )
            .field("durable_watermark_seq", &guard.index.durable_watermark_seq)
            .field("volatile_tail_len", &guard.volatile_tail.len())
            .finish()
    }
}

impl LogSpool {
    /// Ouvre ou initialise un spool dans `spool_dir` (permissions `0700`, hors `.claude/`).
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si les bornes sont nulles ou si le répertoire/index est invalide.
    pub fn open(
        spool_dir: impl AsRef<Path>,
        config: SpoolConfig,
        canaries: &[SecretString],
    ) -> Result<Self, ProcessHostError> {
        if config.max_segment_bytes == 0
            || config.max_total_bytes < config.max_segment_bytes
            || config.max_volatile_bytes == 0
        {
            return Err(ProcessHostError::InvalidArgument {
                reason: String::from(
                    "configuration spool invalide (max_segment_bytes > 0, max_total_bytes >= max_segment_bytes, max_volatile_bytes > 0)",
                ),
            });
        }

        let dir = spool_dir.as_ref().to_path_buf();
        assert_not_under_claude(&dir)?;
        ensure_private_rendezvous_dir(&dir)?;

        let index_path = dir.join(SPOOL_INDEX_FILENAME);
        let index = if let Some(raw) = read_private_file_to_string(&index_path)? {
            serde_json::from_str::<SpoolIndex>(&raw)?
        } else {
            let fresh = SpoolIndex {
                schema_version: 1,
                next_segment_seq: 1,
                durable_watermark_offset: 0,
                durable_watermark_seq: 0,
                oldest_available_offset: 0,
                redaction_count: 0,
                segments: Vec::new(),
                gaps: Vec::new(),
            };
            persist_index(&index_path, &fresh)?;
            fresh
        };

        Ok(Self {
            inner: Mutex::new(SpoolInner {
                spool_dir: dir,
                index_path,
                config,
                redactor: SecretRedactor::new(canaries),
                index,
                volatile_tail: Vec::new(),
            }),
        })
    }

    /// Lit uniquement l'index durable existant sur disque après arrêt/crash du host.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si l'index est absent ou corrompu.
    pub fn load_index_only(spool_dir: impl AsRef<Path>) -> Result<SpoolIndex, ProcessHostError> {
        let dir = spool_dir.as_ref();
        assert_not_under_claude(dir)?;
        let index_path = dir.join(SPOOL_INDEX_FILENAME);
        let raw = read_private_file_to_string(&index_path)?.ok_or_else(|| {
            ProcessHostError::TerminalUnavailable {
                reason: format!("index spool absent dans '{}'", dir.display()),
            }
        })?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Ajoute un fragment brut lu du processus enfant : applique immédiatement la redaction des
    /// secrets canaris avant toute exposition volatile ou écriture segment, puis scelle un ou
    /// plusieurs segments durables dès que la borne configurée est atteinte.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si l'écriture ou la synchronisation disque échoue.
    pub fn append_raw_chunk(&self, raw_bytes: &[u8]) -> Result<(), ProcessHostError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let redacted = guard.redactor.push_chunk(raw_bytes);
        guard.index.redaction_count = guard.redactor.redaction_count();
        if !redacted.is_empty() {
            guard.volatile_tail.extend_from_slice(&redacted);
        }
        seal_full_segments_locked(&mut guard)
    }

    /// Vide le tampon de retenue du redacteur (en fin de flux) et synchronise durablement
    /// tous les octets en attente sur disque (`fsync`).
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si le scellement ou `sync_all` échoue.
    pub fn flush_and_sync_durable(&self) -> Result<u64, ProcessHostError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let flushed = guard.redactor.flush();
        guard.index.redaction_count = guard.redactor.redaction_count();
        if !flushed.is_empty() {
            guard.volatile_tail.extend_from_slice(&flushed);
        }
        seal_all_pending_locked(&mut guard)?;
        Ok(guard.index.durable_watermark_offset)
    }

    /// Synchronise immédiatement sur disque les octets volatils accumulés jusqu'ici sans clore le flux.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] en cas d'erreur d'écriture ou de `sync_all`.
    pub fn sync_durable_now(&self) -> Result<u64, ProcessHostError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seal_all_pending_locked(&mut guard)?;
        Ok(guard.index.durable_watermark_offset)
    }

    /// Retourne les métadonnées courantes du spool sans lire les octets.
    #[must_use]
    pub fn snapshot_index(&self) -> SpoolIndex {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.index.clone()
    }

    /// Lit les segments disponibles à partir d'un offset donné, dans un budget maximal d'octets.
    ///
    /// Si `strict_cursor` vaut `true` et que `after_offset < oldest_available_offset`, retourne
    /// [`ProcessHostError::LogCursorExpired`]. Sinon, retourne les `gaps` explicites puis les
    /// segments disponibles (durables puis queue volatile marquée `is_volatile: true`).
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::LogCursorExpired`] ou erreur I/O si la relecture échoue.
    pub fn read_log(
        &self,
        after_offset: u64,
        max_bytes: usize,
        strict_cursor: bool,
    ) -> Result<LogReadResult, ProcessHostError> {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if strict_cursor && after_offset < guard.index.oldest_available_offset {
            return Err(ProcessHostError::LogCursorExpired {
                requested_seq: after_offset,
                oldest_available_seq: guard.index.oldest_available_offset,
            });
        }

        let mut remaining = max_bytes;
        let mut segments_out = Vec::new();
        let effective_start = after_offset.max(guard.index.oldest_available_offset);

        for seg in &guard.index.segments {
            if remaining == 0 {
                break;
            }
            if seg.end_offset <= effective_start {
                continue;
            }
            let seg_path = guard.spool_dir.join(&seg.filename);
            let raw_bytes = fs::read(&seg_path)?;
            let skip = usize::try_from(effective_start.saturating_sub(seg.start_offset))
                .unwrap_or(usize::MAX);
            let available = raw_bytes.get(skip..).unwrap_or(&[]);
            let take = available.len().min(remaining);
            if take == 0 {
                continue;
            }
            let slice = available.get(..take).unwrap_or(&[]);
            let start_off = seg
                .start_offset
                .saturating_add(u64::try_from(skip).unwrap_or(u64::MAX));
            let end_off = start_off.saturating_add(u64::try_from(slice.len()).unwrap_or(u64::MAX));
            segments_out.push(LogReadSegment {
                segment_seq: Some(seg.segment_seq),
                start_offset: start_off,
                end_offset: end_off,
                sha256: Some(seg.sha256),
                is_volatile: false,
                bytes: slice.to_vec(),
            });
            remaining = remaining.saturating_sub(take);
        }

        let durable_head = guard.index.durable_watermark_offset;
        let volatile_len_u64 = u64::try_from(guard.volatile_tail.len()).unwrap_or(u64::MAX);
        let volatile_head = durable_head.saturating_add(volatile_len_u64);

        if remaining > 0 && !guard.volatile_tail.is_empty() && effective_start < volatile_head {
            let skip = usize::try_from(effective_start.saturating_sub(durable_head)).unwrap_or(0);
            let available = guard.volatile_tail.get(skip..).unwrap_or(&[]);
            let take = available.len().min(remaining);
            if take > 0 {
                let slice = available.get(..take).unwrap_or(&[]);
                let start_off = durable_head.saturating_add(u64::try_from(skip).unwrap_or(0));
                let end_off =
                    start_off.saturating_add(u64::try_from(slice.len()).unwrap_or(u64::MAX));
                segments_out.push(LogReadSegment {
                    segment_seq: None,
                    start_offset: start_off,
                    end_offset: end_off,
                    sha256: None,
                    is_volatile: true,
                    bytes: slice.to_vec(),
                });
            }
        }

        let relevant_gaps: Vec<LogGap> = guard
            .index
            .gaps
            .iter()
            .filter(|g| g.to_offset > after_offset)
            .cloned()
            .collect();

        Ok(LogReadResult {
            durable_watermark_seq: guard.index.durable_watermark_seq,
            durable_watermark_offset: guard.index.durable_watermark_offset,
            oldest_available_offset: guard.index.oldest_available_offset,
            volatile_head_offset: volatile_head,
            redaction_count: guard.index.redaction_count,
            gaps: relevant_gaps,
            segments: segments_out,
        })
    }

    /// Relit chaque segment durable encore enregistré et vérifie l'intégrité SHA-256 sur disque.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::TerminalUnavailable`] si un segment manque ou diverge en SHA-256.
    pub fn verify_durable_segments(
        spool_dir: impl AsRef<Path>,
    ) -> Result<SpoolIndex, ProcessHostError> {
        let dir = spool_dir.as_ref();
        let index = Self::load_index_only(dir)?;
        for seg in &index.segments {
            let path = dir.join(&seg.filename);
            let bytes = fs::read(&path)?;
            let actual = Sha256Digest::from_bytes(&bytes);
            if actual != seg.sha256 {
                return Err(ProcessHostError::TerminalUnavailable {
                    reason: format!(
                        "corruption SHA-256 sur segment {} : attendu {}, observé {}",
                        seg.filename, seg.sha256, actual
                    ),
                });
            }
        }
        Ok(index)
    }
}

fn seal_full_segments_locked(guard: &mut SpoolInner) -> Result<(), ProcessHostError> {
    let seal_threshold = guard
        .config
        .max_segment_bytes
        .min(guard.config.max_volatile_bytes);
    while guard.volatile_tail.len() >= seal_threshold {
        let chunk_len = guard
            .config
            .max_segment_bytes
            .min(guard.volatile_tail.len());
        let bytes: Vec<u8> = guard.volatile_tail.drain(..chunk_len).collect();
        write_and_record_segment_locked(guard, &bytes)?;
    }
    Ok(())
}

fn seal_all_pending_locked(guard: &mut SpoolInner) -> Result<(), ProcessHostError> {
    while !guard.volatile_tail.is_empty() {
        let chunk_len = guard
            .config
            .max_segment_bytes
            .min(guard.volatile_tail.len());
        let bytes: Vec<u8> = guard.volatile_tail.drain(..chunk_len).collect();
        write_and_record_segment_locked(guard, &bytes)?;
    }
    Ok(())
}

fn write_and_record_segment_locked(
    guard: &mut SpoolInner,
    bytes: &[u8],
) -> Result<(), ProcessHostError> {
    if bytes.is_empty() {
        return Ok(());
    }

    let seg_seq = guard.index.next_segment_seq;
    let filename = format!("seg-{seg_seq:06}.log");
    let seg_path = guard.spool_dir.join(&filename);
    assert_not_under_claude(&seg_path)?;

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&seg_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let sha256 = Sha256Digest::from_raw_bytes(hasher.finalize().into());

    let start_offset = guard.index.durable_watermark_offset;
    let byte_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let end_offset = start_offset.saturating_add(byte_len);

    guard.index.segments.push(LogSegmentMeta {
        segment_seq: seg_seq,
        filename,
        start_offset,
        end_offset,
        byte_len,
        sha256,
    });
    guard.index.next_segment_seq = seg_seq.saturating_add(1);
    guard.index.durable_watermark_offset = end_offset;
    guard.index.durable_watermark_seq = seg_seq;

    enforce_total_quota_locked(guard);
    persist_index(&guard.index_path, &guard.index)?;
    Ok(())
}

fn enforce_total_quota_locked(guard: &mut SpoolInner) {
    let max_total = u64::try_from(guard.config.max_total_bytes).unwrap_or(u64::MAX);
    let mut total_bytes: u64 = guard.index.segments.iter().map(|s| s.byte_len).sum();
    if total_bytes <= max_total {
        return;
    }

    let mut dropped_from_seq = None;
    let mut dropped_to_seq = 0_u64;
    let mut dropped_from_off = 0_u64;
    let mut dropped_to_off = 0_u64;
    let mut dropped_sum = 0_u64;

    while total_bytes > max_total && guard.index.segments.len() > 1 {
        let oldest = guard.index.segments.remove(0);
        let path = guard.spool_dir.join(&oldest.filename);
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        if dropped_from_seq.is_none() {
            dropped_from_seq = Some(oldest.segment_seq);
            dropped_from_off = oldest.start_offset;
        }
        dropped_to_seq = oldest.segment_seq;
        dropped_to_off = oldest.end_offset;
        dropped_sum = dropped_sum.saturating_add(oldest.byte_len);
        total_bytes = total_bytes.saturating_sub(oldest.byte_len);
        guard.index.oldest_available_offset = oldest.end_offset;
    }

    if let Some(from_seq) = dropped_from_seq {
        guard.index.gaps.push(LogGap {
            from_segment_seq: from_seq,
            to_segment_seq: dropped_to_seq,
            from_offset: dropped_from_off,
            to_offset: dropped_to_off,
            dropped_bytes: dropped_sum,
            reason: String::from("quota_rotation"),
        });
    }
}

fn persist_index(index_path: &Path, index: &SpoolIndex) -> Result<(), ProcessHostError> {
    let bytes = serde_json::to_vec_pretty(index)?;
    write_private_file_atomic(index_path, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unleash_testkit::local_test_dir;

    #[test]
    fn redactor_masks_split_chunk_and_multiline_canaries_without_delaying_ordinary_text() {
        let canaries = vec![
            SecretString::from("sk-live-CANARY-SPLIT-9876"),
            SecretString::from("BEGIN_MULTI\nLINE_SECRET\nEND_MULTI"),
        ];
        let mut redactor = SecretRedactor::new(&canaries);

        // Texte ordinaire non préfixe : émis immédiatement sans retenue
        assert_eq!(redactor.push_chunk(b"prompt$ "), b"prompt$ ");

        // Canari coupé en 3 morceaux
        let o1 = redactor.push_chunk(b"tok=sk-live-");
        assert_eq!(o1, b"tok=");
        let o2 = redactor.push_chunk(b"CANARY-SPLIT");
        assert_eq!(o2, b"");
        let o3 = redactor.push_chunk(b"-9876 done\n");
        assert_eq!(o3, b"[REDACTED] done\n");

        // Canari multi-ligne coupé au milieu du saut de ligne
        let m1 = redactor.push_chunk(b"hdr BEGIN_MULTI\nLINE_");
        assert_eq!(m1, b"hdr ");
        let m2 = redactor.push_chunk(b"SECRET\nEND_MULTI tail");
        assert_eq!(m2, b"[REDACTED] tail");

        // Variante PTY ONLCR (\n converti en \r\n) sur canari multi-ligne
        let c1 = redactor.push_chunk(b"pty BEGIN_MULTI\r\nLINE_");
        assert_eq!(c1, b"pty ");
        let c2 = redactor.push_chunk(b"SECRET\r\nEND_MULTI ok");
        assert_eq!(c2, b"[REDACTED] ok");

        assert_eq!(redactor.flush(), b"");
        assert_eq!(redactor.redaction_count(), 3);
    }

    #[test]
    fn spool_separates_volatile_from_durable_and_records_explicit_gap_on_quota() {
        let dir = local_test_dir("ph-spool").unwrap();
        let spool_dir = dir.path().join("spool");
        let cfg = SpoolConfig {
            max_segment_bytes: 32,
            max_total_bytes: 64,
            max_volatile_bytes: 64,
        };
        let canaries = vec![SecretString::from("CANARY_XYZ")];
        let spool = LogSpool::open(&spool_dir, cfg, &canaries).unwrap();

        // 1. Octets volatils avant scellement (< 32 octets)
        spool.append_raw_chunk(b"hello CANARY_XYZ").unwrap();
        let read1 = spool.read_log(0, 1024, false).unwrap();
        assert_eq!(read1.durable_watermark_offset, 0);
        assert_eq!(read1.segments.len(), 1);
        let seg0 = read1.segments.first().unwrap();
        assert!(seg0.is_volatile);
        let text1 = String::from_utf8_lossy(&seg0.bytes);
        assert!(!text1.contains("CANARY_XYZ"));
        assert!(text1.contains("[REDACTED]"));

        // 2. Flood dépassant max_total_bytes (64) -> rotation + gap explicite
        for i in 0..6 {
            let line = format!("segment-payload-line-{i:02}-0123456789\n");
            spool.append_raw_chunk(line.as_bytes()).unwrap();
        }
        spool.flush_and_sync_durable().unwrap();

        let verified = LogSpool::verify_durable_segments(&spool_dir).unwrap();
        assert!(!verified.gaps.is_empty());
        assert!(verified.oldest_available_offset > 0);
        assert!(verified.durable_watermark_offset > verified.oldest_available_offset);

        // Curseur strict sur offset expiré -> LogCursorExpired (ResyncRequired/CURSOR_EXPIRED)
        let err = spool.read_log(0, 1024, true).unwrap_err();
        assert!(matches!(err, ProcessHostError::LogCursorExpired { .. }));

        // Lecture tolérante expose le gap explicite + segments encore présents
        let read2 = spool.read_log(0, 4096, false).unwrap();
        assert!(!read2.gaps.is_empty());
        assert!(!read2.segments.is_empty());
        assert!(read2.segments.iter().all(|s| !s.is_volatile));
    }
}
