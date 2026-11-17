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

use crate::errors::ProtocolError;
use std::io::{Read, Write};

/// Taille maximale d'une trame en octets (8 MiB = 8 * 1024 * 1024 = `8_388_608`).
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_FRAME_BYTES_U32: u32 = 8 * 1024 * 1024;

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

    while let Some(buf) = header.get_mut(bytes_read..) {
        if buf.is_empty() {
            break;
        }
        match reader.read(buf) {
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
    let Ok(length_usize) = usize::try_from(length) else {
        return Err(ProtocolError::FrameTooLarge {
            length,
            max: MAX_FRAME_BYTES_U32,
        });
    };
    if length_usize > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            length,
            max: MAX_FRAME_BYTES_U32,
        });
    }

    let mut payload = vec![0u8; length_usize];
    let mut payload_read = 0;

    while let Some(buf) = payload.get_mut(payload_read..) {
        if buf.is_empty() {
            break;
        }
        match reader.read(buf) {
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
            length: u32::try_from(payload.len()).unwrap_or(u32::MAX),
            max: MAX_FRAME_BYTES_U32,
        });
    }

    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge {
        length: u32::MAX,
        max: MAX_FRAME_BYTES_U32,
    })?;
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
        let Some(header_slice) = self.buffer.get(..4) else {
            return Ok(None);
        };
        let header: [u8; 4] = header_slice
            .try_into()
            .map_err(|_| ProtocolError::Validation("En-tête de trame IPC invalide".to_owned()))?;
        let length_u32 = u32::from_be_bytes(header);
        let Ok(length) = usize::try_from(length_u32) else {
            return Err(ProtocolError::FrameTooLarge {
                length: length_u32,
                max: MAX_FRAME_BYTES_U32,
            });
        };

        if length > MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameTooLarge {
                length: length_u32,
                max: MAX_FRAME_BYTES_U32,
            });
        }

        let Some(total_needed) = 4_usize.checked_add(length) else {
            return Err(ProtocolError::FrameTooLarge {
                length: length_u32,
                max: MAX_FRAME_BYTES_U32,
            });
        };
        let Some(frame_slice) = self.buffer.get(4..total_needed) else {
            return Ok(None);
        };

        // Extrait la trame et retire les octets consommés
        let frame = frame_slice.to_vec();
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
