// SPDX-License-Identifier: Apache-2.0
//! Transport Unix asynchrone (`tokio::net::UnixStream`), codec borné 8 MiB et handshake `Hello`
//! négociant `unleash.ipc.v2`.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use unleash_domain::{AuthorityGeneration, StreamId};
use unleash_platform::{validate_socket_file, verify_same_uid_peer};
use unleash_protocol::{
    ClientKind, HelloRequest, HelloResponse, IpcError, MAX_FRAME_BYTES, ProtocolError,
    ProtocolVersion, from_json_str_strict,
};

use crate::errors::ClientError;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration d'une connexion cliente vers le daemon Unleash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// Chemin du socket Unix du daemon.
    pub socket_path: PathBuf,
    /// Nature du client (`Cli`, `Tui`, `Gui`, `Worker`, `Probe`).
    pub client_kind: ClientKind,
    /// Version applicative du client annoncée au handshake `Hello`.
    pub client_version: String,
    /// Capacités demandées ou supportées par le client.
    pub requested_capabilities: Vec<String>,
    /// Délai maximal d'établissement de la connexion et du handshake `Hello`.
    pub connect_timeout: Duration,
    /// Délai maximal d'attente par requête/réponse IPC.
    pub request_timeout: Duration,
    /// Active le contrôle `0700`/`0600`/`SUN_LEN` et `LOCAL_PEERCRED`/`SO_PEERCRED` (activé par défaut).
    pub verify_socket_security: bool,
}

impl ClientConfig {
    /// Construit une configuration client avec des délais bornés par défaut et vérification
    /// de sécurité du socket activée.
    #[must_use]
    pub fn new(
        socket_path: PathBuf,
        client_kind: ClientKind,
        client_version: impl Into<String>,
    ) -> Self {
        Self {
            socket_path,
            client_kind,
            client_version: client_version.into(),
            requested_capabilities: Vec::new(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            verify_socket_security: true,
        }
    }

    /// Configure les capacités annoncées lors du handshake `Hello`.
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.requested_capabilities = capabilities;
        self
    }

    /// Configure les délais bornés de connexion et de requête.
    #[must_use]
    pub const fn with_timeouts(mut self, connect: Duration, request: Duration) -> Self {
        self.connect_timeout = connect;
        self.request_timeout = request;
        self
    }
}

/// Connexion Unix authentifiée et négociée (`unleash.ipc.v2`) vers le daemon.
#[derive(Debug)]
pub struct IpcConnection {
    config: ClientConfig,
    stream: UnixStream,
    hello: HelloResponse,
    effective_max_frame_bytes: usize,
}

impl IpcConnection {
    /// Établit la connexion Unix, valide les permissions (`0700`/`0600`) et l'UID peer noyau,
    /// puis exécute le handshake `Hello` (`unleash.ipc.v2`).
    ///
    /// # Sûreté à l'annulation
    /// Annuler cette future à tout moment ferme proprement le descripteur de socket sans état
    /// partagé ni effet durable côté client.
    ///
    /// # Errors
    /// - [`ClientError::Platform`] si le répertoire `0700`, le socket `0600` ou le peer UID échoue.
    /// - [`ClientError::Timeout`] si la connexion ou le handshake dépasse `connect_timeout`.
    /// - [`ClientError::Remote`] / [`ClientError::ProtocolIncompatible`] si le serveur rejette v2.
    pub async fn connect(config: &ClientConfig) -> Result<Self, ClientError> {
        if config.verify_socket_security {
            validate_socket_file(&config.socket_path)?;
        }

        let connect_ms = duration_to_ms(config.connect_timeout);
        let mut stream = tokio::time::timeout(
            config.connect_timeout,
            UnixStream::connect(&config.socket_path),
        )
        .await
        .map_err(|_| ClientError::Timeout {
            operation: "UnixStream::connect",
            timeout_ms: connect_ms,
        })??;

        if config.verify_socket_security {
            verify_same_uid_peer(&stream)?;
        }

        let hello_req = HelloRequest {
            protocol_version: ProtocolVersion,
            client_kind: config.client_kind,
            client_version: config.client_version.clone(),
            capabilities: config.requested_capabilities.clone(),
        };

        write_json_frame_async(
            &mut stream,
            &hello_req,
            MAX_FRAME_BYTES,
            config.connect_timeout,
            "HelloRequest",
        )
        .await?;

        let raw_resp = read_raw_frame_async(
            &mut stream,
            MAX_FRAME_BYTES,
            config.connect_timeout,
            "HelloResponse",
        )
        .await?
        .ok_or(ClientError::Disconnected {
            operation: "HelloResponse",
        })?;

        let hello: HelloResponse = decode_response_or_ipc_error(&raw_resp, "HelloResponse")?;
        let max_frame_u32 = u32::try_from(MAX_FRAME_BYTES).unwrap_or(u32::MAX);
        if hello.max_frame_bytes == 0 || hello.max_frame_bytes > max_frame_u32 {
            return Err(ClientError::UnexpectedResponse {
                expected: "HelloResponse",
                detail: format!(
                    "max_frame_bytes hors bornes : {} (plafond {max_frame_u32})",
                    hello.max_frame_bytes
                ),
            });
        }
        if hello.suggested_segment_bytes == 0
            || hello.suggested_segment_bytes > hello.max_frame_bytes
        {
            return Err(ClientError::UnexpectedResponse {
                expected: "HelloResponse",
                detail: format!(
                    "suggested_segment_bytes hors bornes : {} (max_frame_bytes={})",
                    hello.suggested_segment_bytes, hello.max_frame_bytes
                ),
            });
        }

        let effective_max_frame_bytes = usize::try_from(hello.max_frame_bytes)
            .unwrap_or(MAX_FRAME_BYTES)
            .min(MAX_FRAME_BYTES);

        Ok(Self {
            config: config.clone(),
            stream,
            hello,
            effective_max_frame_bytes,
        })
    }

    /// Réponse `HelloResponse` négociée à l'ouverture de cette connexion.
    #[must_use]
    pub const fn hello(&self) -> &HelloResponse {
        &self.hello
    }

    /// Configuration utilisée pour établir cette connexion.
    #[must_use]
    pub const fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Lignée d'autorité active annoncée par le daemon dans `HelloResponse`.
    #[must_use]
    pub const fn authority_generation(&self) -> AuthorityGeneration {
        self.hello.authority_generation
    }

    /// Identifiant de flux d'événements annoncé par le daemon dans `HelloResponse`.
    #[must_use]
    pub const fn stream_id(&self) -> StreamId {
        self.hello.stream_id
    }

    /// Plafond effectif d'une trame négocié (toujours `<= 8 MiB`).
    #[must_use]
    pub const fn max_frame_bytes(&self) -> usize {
        self.effective_max_frame_bytes
    }

    /// Envoie une trame JSON typée sur le socket avec vérification stricte de taille `<= 8 MiB`.
    ///
    /// # Sûreté à l'annulation
    /// Annuler cette future en plein envoi peut laisser une trame partielle sur le flux ;
    /// en cas d'annulation, fermer la connexion et en ouvrir une nouvelle avant de réessayer.
    ///
    /// # Errors
    /// Retourne [`ClientError::Protocol`] si la charge dépasse le plafond, [`ClientError::Timeout`]
    /// ou [`ClientError::Disconnected`] en cas de coupure ou dépassement de délai.
    pub async fn send_json_frame<T: Serialize + Sync>(
        &mut self,
        payload: &T,
        op: &'static str,
    ) -> Result<(), ClientError> {
        write_json_frame_async(
            &mut self.stream,
            payload,
            self.effective_max_frame_bytes,
            self.config.request_timeout,
            op,
        )
        .await
    }

    /// Lit et décode strictement une réponse typée ou une erreur [`IpcError`] distante.
    ///
    /// # Sûreté à l'annulation
    /// Annuler en pleine lecture d'une trame peut désynchroniser le cadrage du flux ;
    /// fermer la connexion avant toute nouvelle requête.
    ///
    /// # Errors
    /// Retourne [`ClientError::Remote`] si le daemon répond par un [`IpcError`],
    /// [`ClientError::Protocol`] sur JSON invalide/clé dupliquée/trame > 8 MiB, ou
    /// [`ClientError::Disconnected`] / [`ClientError::Timeout`].
    pub async fn read_json_response<T: DeserializeOwned>(
        &mut self,
        expected: &'static str,
    ) -> Result<T, ClientError> {
        let raw = read_raw_frame_async(
            &mut self.stream,
            self.effective_max_frame_bytes,
            self.config.request_timeout,
            expected,
        )
        .await?
        .ok_or(ClientError::Disconnected {
            operation: expected,
        })?;
        decode_response_or_ipc_error(&raw, expected)
    }

    /// Lit une trame brute (retourne `Ok(None)` sur EOF propre à la frontière d'une trame)
    /// avec délai optionnel.
    ///
    /// # Sûreté à l'annulation
    /// Si annulée pendant la lecture du corps d'une trame déjà entamée, le flux doit être
    /// considéré fermé et reconnecté.
    ///
    /// # Errors
    /// Retourne [`ClientError::Protocol`] si la trame dépasse `max_frame_bytes` ou est tronquée,
    /// ou [`ClientError::Timeout`] si un délai configuré expire.
    pub(crate) async fn read_raw_frame_unbounded_wait(
        &mut self,
        op: &'static str,
    ) -> Result<Option<Vec<u8>>, ClientError> {
        read_raw_frame_no_timeout(&mut self.stream, self.effective_max_frame_bytes, op).await
    }

    /// Tente de lire une éventuelle trame [`IpcError`] restante après une fermeture anticipée
    /// durant un envoi segmenté.
    pub(crate) async fn try_read_trailing_ipc_error(&mut self) -> Option<IpcError> {
        let short_timeout = Duration::from_millis(200);
        let raw = read_raw_frame_async(
            &mut self.stream,
            self.effective_max_frame_bytes,
            short_timeout,
            "trailing_error",
        )
        .await
        .ok()??;
        let text = std::str::from_utf8(&raw).ok()?;
        from_json_str_strict::<IpcError>(text).ok()
    }
}

pub(crate) async fn write_json_frame_async<T: Serialize>(
    stream: &mut UnixStream,
    value: &T,
    max_bytes: usize,
    timeout: Duration,
    op: &'static str,
) -> Result<(), ClientError> {
    let encoded = serde_json::to_vec(value).map_err(ProtocolError::Json)?;
    let max_u32 = u32::try_from(max_bytes).unwrap_or(u32::MAX);
    if encoded.len() > max_bytes || encoded.len() > MAX_FRAME_BYTES {
        let len_u32 = u32::try_from(encoded.len()).unwrap_or(u32::MAX);
        return Err(ClientError::Protocol(ProtocolError::FrameTooLarge {
            length: len_u32,
            max: max_u32,
        }));
    }
    let len_u32 = u32::try_from(encoded.len()).map_err(|_| {
        ClientError::Protocol(ProtocolError::FrameTooLarge {
            length: u32::MAX,
            max: max_u32,
        })
    })?;
    let header = len_u32.to_be_bytes();
    let timeout_ms = duration_to_ms(timeout);

    tokio::time::timeout(timeout, async {
        stream.write_all(&header).await?;
        stream.write_all(&encoded).await?;
        stream.flush().await
    })
    .await
    .map_err(|_| ClientError::Timeout {
        operation: op,
        timeout_ms,
    })?
    .map_err(|e| map_write_io_error(e, op))
}

pub(crate) async fn read_raw_frame_async(
    stream: &mut UnixStream,
    max_bytes: usize,
    timeout: Duration,
    op: &'static str,
) -> Result<Option<Vec<u8>>, ClientError> {
    let timeout_ms = duration_to_ms(timeout);
    tokio::time::timeout(timeout, read_raw_frame_no_timeout(stream, max_bytes, op))
        .await
        .map_err(|_| ClientError::Timeout {
            operation: op,
            timeout_ms,
        })?
}

async fn read_raw_frame_no_timeout(
    stream: &mut UnixStream,
    max_bytes: usize,
    op: &'static str,
) -> Result<Option<Vec<u8>>, ClientError> {
    let mut header = [0u8; 4];
    let mut header_read = 0usize;
    while let Some(buf) = header.get_mut(header_read..) {
        if buf.is_empty() {
            break;
        }
        match stream.read(buf).await {
            Ok(0) => {
                if header_read == 0 {
                    return Ok(None);
                }
                return Err(ClientError::Protocol(ProtocolError::UnexpectedEof));
            }
            Ok(n) => header_read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_read_io_error(e, op)),
        }
    }

    let length_u32 = u32::from_be_bytes(header);
    let cap = max_bytes.min(MAX_FRAME_BYTES);
    let cap_u32 = u32::try_from(cap).unwrap_or(u32::MAX);
    let Ok(length) = usize::try_from(length_u32) else {
        return Err(ClientError::Protocol(ProtocolError::FrameTooLarge {
            length: length_u32,
            max: cap_u32,
        }));
    };
    if length > cap {
        return Err(ClientError::Protocol(ProtocolError::FrameTooLarge {
            length: length_u32,
            max: cap_u32,
        }));
    }

    // Allocation effectuée uniquement APRÈS validation stricte length <= cap <= 8 MiB.
    let mut payload = vec![0u8; length];
    if let Err(e) = stream.read_exact(&mut payload).await {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            return Err(ClientError::Protocol(ProtocolError::UnexpectedEof));
        }
        return Err(map_read_io_error(e, op));
    }

    Ok(Some(payload))
}

pub(crate) fn decode_response_or_ipc_error<T: DeserializeOwned>(
    raw: &[u8],
    expected: &'static str,
) -> Result<T, ClientError> {
    let text = std::str::from_utf8(raw).map_err(|e| {
        ClientError::Protocol(ProtocolError::Validation(format!(
            "Trame {expected} UTF-8 invalide : {e}"
        )))
    })?;

    match from_json_str_strict::<T>(text) {
        Ok(value) => Ok(value),
        Err(
            primary_err @ (ProtocolError::DuplicateKey { .. }
            | ProtocolError::ProtocolIncompatible { .. }),
        ) => Err(ClientError::Protocol(primary_err)),
        Err(primary_err) => {
            if let Ok(ipc_err) = from_json_str_strict::<IpcError>(text) {
                return Err(ClientError::Remote(ipc_err));
            }
            Err(ClientError::Protocol(primary_err))
        }
    }
}

fn map_write_io_error(err: std::io::Error, op: &'static str) -> ClientError {
    if matches!(
        err.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::UnexpectedEof
    ) {
        ClientError::Disconnected { operation: op }
    } else {
        ClientError::Io(err)
    }
}

fn map_read_io_error(err: std::io::Error, op: &'static str) -> ClientError {
    if matches!(
        err.kind(),
        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
    ) {
        ClientError::Disconnected { operation: op }
    } else {
        ClientError::Io(err)
    }
}

fn duration_to_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
