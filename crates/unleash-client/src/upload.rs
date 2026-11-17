// SPDX-License-Identifier: Apache-2.0
//! Découpage et préparation des segments `ClientUploadSegment` pour l'import de données
//! `client_upload` (`F019`, `ipc-and-apis.md`).

use unleash_domain::{CommandId, Sha256Digest};
use unleash_protocol::{ClientUploadSegment, MAX_CLIENT_UPLOAD_BYTES, MAX_FRAME_BYTES};

use crate::errors::ClientError;

const JSON_SEGMENT_OVERHEAD_RESERVE: usize = 4096;

/// Résumé d'un transfert segmenté prêt ou transmis au daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadSummary {
    /// Identifiant de la commande associée (`ImportAutomationData`).
    pub command_id: CommandId,
    /// Nombre total de segments produits (`>= 1`).
    pub total_segments: u32,
    /// Longueur totale brute en octets (`<= 10 MiB`).
    pub byte_length: u64,
    /// Empreinte SHA-256 canonique de l'intégralité des octets bruts.
    pub sha256: Sha256Digest,
}

/// Découpe `bytes` en trames [`ClientUploadSegment`] validées, bornées par `MAX_CLIENT_UPLOAD_BYTES`
/// (10 MiB), `suggested_segment_bytes`, et la marge base64 sous `max_frame_bytes` (8 MiB).
///
/// Un fichier vide (`bytes.is_empty()`) produit exactement 1 segment de longueur 0 afin de
/// respecter l'invariant `total_segments >= 1`.
///
/// # Errors
/// - [`ClientError::UploadTooLarge`] si `bytes.len() > MAX_CLIENT_UPLOAD_BYTES` (10 MiB).
/// - [`ClientError::Protocol`] si la construction ou validation d'un segment échoue.
pub fn build_upload_segments(
    command_id: CommandId,
    bytes: &[u8],
    max_frame_bytes: u32,
    suggested_segment_bytes: u32,
) -> Result<(Vec<ClientUploadSegment>, UploadSummary), ClientError> {
    let total_len_u64 = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if total_len_u64 > MAX_CLIENT_UPLOAD_BYTES {
        return Err(ClientError::UploadTooLarge {
            length: total_len_u64,
            max: MAX_CLIENT_UPLOAD_BYTES,
        });
    }

    let capped_frame = usize::try_from(max_frame_bytes)
        .unwrap_or(MAX_FRAME_BYTES)
        .min(MAX_FRAME_BYTES);
    let max_b64_payload = capped_frame.saturating_sub(JSON_SEGMENT_OVERHEAD_RESERVE);
    // Base64 RFC 4648 étend 3 octets bruts en 4 caractères ASCII.
    let max_raw_for_frame = (max_b64_payload / 4) * 3;
    let suggested = usize::try_from(suggested_segment_bytes).unwrap_or(max_raw_for_frame);
    let chunk_limit = suggested.min(max_raw_for_frame).max(1);

    let full_digest = Sha256Digest::from_bytes(bytes);

    if bytes.is_empty() {
        let seg = ClientUploadSegment::from_bytes(command_id, 0, 1, 0, b"")?;
        let summary = UploadSummary {
            command_id,
            total_segments: 1,
            byte_length: 0,
            sha256: full_digest,
        };
        return Ok((vec![seg], summary));
    }

    let total_chunks = bytes.chunks(chunk_limit).len();
    let total_segments = u32::try_from(total_chunks).map_err(|_| ClientError::UploadTooLarge {
        length: total_len_u64,
        max: MAX_CLIENT_UPLOAD_BYTES,
    })?;

    let mut segments = Vec::with_capacity(total_chunks);
    let mut offset: u64 = 0;

    for (idx, chunk) in bytes.chunks(chunk_limit).enumerate() {
        let seg_index = u32::try_from(idx).map_err(|_| ClientError::UploadTooLarge {
            length: total_len_u64,
            max: MAX_CLIENT_UPLOAD_BYTES,
        })?;
        let seg =
            ClientUploadSegment::from_bytes(command_id, seg_index, total_segments, offset, chunk)?;
        offset = offset.saturating_add(u64::try_from(chunk.len()).unwrap_or(0));
        segments.push(seg);
    }

    let summary = UploadSummary {
        command_id,
        total_segments,
        byte_length: total_len_u64,
        sha256: full_digest,
    };

    Ok((segments, summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_chunk_clamps_under_8mib_even_with_large_suggestion() {
        let cmd_id = CommandId::from_u128(0xAA01);
        // Même si suggested_segment_bytes vaut presque 8 MiB, l'expansion base64 (4/3) + JSON
        // doit rester strictement sous MAX_FRAME_BYTES (8 MiB).
        let payload = vec![0x5Au8; 6_500_000];
        let max_frame = u32::try_from(MAX_FRAME_BYTES).unwrap_or(u32::MAX);
        let (segments, summary) =
            build_upload_segments(cmd_id, &payload, max_frame, 7_500_000).unwrap();

        assert!(segments.len() >= 2);
        assert_eq!(
            summary.total_segments,
            u32::try_from(segments.len()).unwrap_or(0)
        );
        for seg in &segments {
            let wire = serde_json::to_vec(seg).unwrap();
            assert!(
                wire.len() <= MAX_FRAME_BYTES,
                "Chaque segment JSON sérialisé doit tenir sous 8 MiB (observé {})",
                wire.len()
            );
            assert!(seg.validate().is_ok());
        }
    }
}
