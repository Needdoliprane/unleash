//! # Codec de trames Unix IPC
//!
//! Responsabilité exclusive : Encodage et décodage synchrone des trames sur socket Unix
//! (ou tout flux `std::io::Read` / `std::io::Write`) sans dépendance asynchrone vers `tokio`.
//!
//! Format de trame :
//! - Longueur : 4 octets (`u32` big-endian).
//! - Corps : payload JSON encodé en UTF-8.
//! - Plafond strict : **8 MiB = `8_388_608` octets**.
//! - Protection `DoS` : toute trame annonçant une taille supérieure à 8 MiB est immédiatement
//!   rejetée **avant** toute allocation mémoire proportionnelle.

#![allow(clippy::cast_possible_truncation, clippy::indexing_slicing)]

use crate::errors::ProtocolError;
use std::io::{Read, Write};

/// Taille maximale d'une trame en octets (8 MiB = 8 * 1024 * 1024 = `8_388_608`).
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Lit une trame complète depuis un lecteur synchrone [`std::io::Read`].
///
/// Si le flux est fermé proprement à la frontière exacte d'une trame (0 octet lu),
/// renvoie `Ok(None)`.
///
/// # Errors
/// - Renvoie [`ProtocolError::FrameTooLarge`] si la longueur annoncée excède 8 MiB (sans allouer de tampon géant).
/// - Renvoie [`ProtocolError::UnexpectedEof`] si le flux est interrompu au milieu d'une trame ou de son en-tête.
/// - Renvoie [`ProtocolError::Io`] en cas d'erreur de lecture I/O.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, ProtocolError> {
    let mut header = [0u8; 4];
    let mut bytes_read = 0;

    while bytes_read < 4 {
        match reader.read(&mut header[bytes_read..]) {
            Ok(0) => {
                if bytes_read == 0 {
                    return Ok(None);
                }
                return Err(ProtocolError::UnexpectedEof);
            }
            Ok(n) => bytes_read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(ProtocolError::Io(e)),
        }
    }

    let length = u32::from_be_bytes(header);
    if (length as usize) > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            length,
            max: MAX_FRAME_BYTES as u32,
        });
    }

    let mut payload = vec![0u8; length as usize];
    let mut payload_read = 0;

    while payload_read < length as usize {
        match reader.read(&mut payload[payload_read..]) {
            Ok(0) => return Err(ProtocolError::UnexpectedEof),
            Ok(n) => payload_read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(ProtocolError::Io(e)),
        }
    }

    Ok(Some(payload))
}

/// Écrit une trame complète vers un écrivain synchrone [`std::io::Write`].
///
/// # Errors
/// - Renvoie [`ProtocolError::FrameTooLarge`] si la charge utile dépasse 8 MiB.
/// - Renvoie [`ProtocolError::Io`] en cas d'erreur d'écriture ou de flush.
pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), ProtocolError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            length: payload.len() as u32,
            max: MAX_FRAME_BYTES as u32,
        });
    }

    let length = payload.len() as u32;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

/// Tampon incrémental pour le découpage de trames IPC à partir d'octets reçus par morceaux.
#[derive(Debug, Default)]
pub struct FrameBuffer {
    buffer: Vec<u8>,
}

impl FrameBuffer {
    /// Crée un nouveau tampon de réception de trames vide.
    #[must_use]
    pub const fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Ajoute des octets bruts au tampon.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Tente d'extraire la prochaine trame complète du tampon.
    ///
    /// Renvoie `Ok(Some(frame))` si une trame complète est prête,
    /// `Ok(None)` si des données supplémentaires sont requises.
    ///
    /// # Errors
    /// Renvoie [`ProtocolError::FrameTooLarge`] si la longueur annoncée par l'en-tête dépasse 8 MiB.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, ProtocolError> {
        if self.buffer.len() < 4 {
            return Ok(None);
        }

        let mut header = [0u8; 4];
        header.copy_from_slice(&self.buffer[..4]);
        let length = u32::from_be_bytes(header) as usize;

        if length > MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameTooLarge {
                length: length as u32,
                max: MAX_FRAME_BYTES as u32,
            });
        }

        let total_needed = 4 + length;
        if self.buffer.len() < total_needed {
            return Ok(None);
        }

        // Extrait la trame et retire les octets consommés
        let frame = self.buffer[4..total_needed].to_vec();
        self.buffer.drain(..total_needed);
        Ok(Some(frame))
    }

    /// Renvoie le nombre d'octets actuellement stockés dans le tampon.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Indique si le tampon est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}
