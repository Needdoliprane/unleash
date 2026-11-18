//! Separate read-only connection for a daemon without a durable lineage.

use crate::errors::ClientError;
use crate::transport::{
    ClientConfig, decode_response_or_ipc_error, read_raw_frame_async, write_json_frame_async,
};
use tokio::net::UnixStream;
use unleash_domain::RequestId;
use unleash_platform::{read_admin_credential, validate_socket_file, verify_same_uid_peer};
use unleash_protocol::{
    AdminProof, ClientKind, DiagnosticHelloResponse, HelloRequest, MAX_FRAME_BYTES,
    ProtocolVersion, QueryEnvelope, QueryResponse, READ_ONLY_RECOVERY_CAPABILITY,
};

/// Bounded diagnostic connection. It cannot construct commands or subscriptions.
#[derive(Debug)]
pub struct DiagnosticClient {
    config: ClientConfig,
    stream: UnixStream,
    hello: DiagnosticHelloResponse,
}

impl DiagnosticClient {
    /// Negotiate the diagnostic-only handshake and verify the peer.
    ///
    /// # Errors
    /// Returns a transport, peer-validation or protocol error.
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError> {
        if config.verify_socket_security {
            validate_socket_file(&config.socket_path)?;
        }
        let mut stream = tokio::time::timeout(
            config.connect_timeout,
            UnixStream::connect(&config.socket_path),
        )
        .await
        .map_err(|_| ClientError::Timeout {
            operation: "UnixStream::connect",
            timeout_ms: u64::try_from(config.connect_timeout.as_millis()).unwrap_or(u64::MAX),
        })??;
        if config.verify_socket_security {
            verify_same_uid_peer(&stream)?;
        }
        let proof = if config.verify_socket_security
            && config.load_admin_credential
            && matches!(
                config.client_kind,
                ClientKind::Cli | ClientKind::Tui | ClientKind::Gui
            ) {
            config
                .socket_path
                .parent()
                .and_then(|dir| read_admin_credential(dir).ok().flatten())
                .and_then(|value| AdminProof::from_hex(&value))
        } else {
            None
        };
        let mut capabilities = config.requested_capabilities.clone();
        if !capabilities
            .iter()
            .any(|cap| cap == READ_ONLY_RECOVERY_CAPABILITY)
        {
            capabilities.push(READ_ONLY_RECOVERY_CAPABILITY.into());
        }
        let hello = HelloRequest {
            protocol_version: ProtocolVersion,
            client_kind: config.client_kind,
            client_version: config.client_version.clone(),
            capabilities,
            admin_proof: proof,
        };
        write_json_frame_async(
            &mut stream,
            &hello,
            MAX_FRAME_BYTES,
            config.connect_timeout,
            "HelloRequest",
        )
        .await?;
        let raw = read_raw_frame_async(
            &mut stream,
            MAX_FRAME_BYTES,
            config.connect_timeout,
            "DiagnosticHelloResponse",
        )
        .await?
        .ok_or(ClientError::Disconnected {
            operation: "DiagnosticHelloResponse",
        })?;
        let hello: DiagnosticHelloResponse =
            decode_response_or_ipc_error(&raw, "DiagnosticHelloResponse")?;
        if hello.max_frame_bytes == 0
            || usize::try_from(hello.max_frame_bytes).unwrap_or(usize::MAX) > MAX_FRAME_BYTES
        {
            return Err(ClientError::UnexpectedResponse {
                expected: "DiagnosticHelloResponse",
                detail: "max_frame_bytes hors bornes".into(),
            });
        }
        Ok(Self {
            config,
            stream,
            hello,
        })
    }

    /// Diagnostic handshake, which contains no durable identifiers.
    #[must_use]
    pub const fn hello(&self) -> &DiagnosticHelloResponse {
        &self.hello
    }

    /// Read bounded daemon status.
    ///
    /// # Errors
    /// Returns a transport or remote authorization error.
    pub async fn status(&mut self, request_id: RequestId) -> Result<QueryResponse, ClientError> {
        self.query("GetDaemonStatus", request_id).await
    }

    /// Read offline recovery information if administratively authorized.
    ///
    /// # Errors
    /// Returns a transport, recovery or remote authorization error.
    pub async fn inspect_recovery(
        &mut self,
        request_id: RequestId,
    ) -> Result<QueryResponse, ClientError> {
        self.query("InspectRecovery", request_id).await
    }

    async fn query(
        &mut self,
        name: &str,
        request_id: RequestId,
    ) -> Result<QueryResponse, ClientError> {
        let query = QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id,
            query_type: name.into(),
            project_id: None,
            payload: serde_json::Map::new(),
        };
        let max = usize::try_from(self.hello.max_frame_bytes).unwrap_or(MAX_FRAME_BYTES);
        write_json_frame_async(
            &mut self.stream,
            &query,
            max,
            self.config.request_timeout,
            "QueryEnvelope",
        )
        .await?;
        let raw = read_raw_frame_async(
            &mut self.stream,
            max,
            self.config.request_timeout,
            "QueryResponse",
        )
        .await?
        .ok_or(ClientError::Disconnected {
            operation: "QueryResponse",
        })?;
        let response: QueryResponse = decode_response_or_ipc_error(&raw, "QueryResponse")?;
        if response.request_id != request_id || response.watermark.is_some() {
            return Err(ClientError::UnexpectedResponse {
                expected: "Diagnostic QueryResponse",
                detail: "corrélation ou watermark invalide".into(),
            });
        }
        Ok(response)
    }
}
