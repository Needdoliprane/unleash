//! Separate read-only connection for a daemon without a durable lineage.

use crate::errors::ClientError;
use crate::transport::{
    ClientConfig, connect_verified_peer, decode_response_or_ipc_error, load_admin_proof,
    read_raw_frame_async, write_json_frame_async,
};
use tokio::net::UnixStream;
use unleash_domain::RequestId;
use unleash_protocol::{
    DiagnosticHelloResponse, HelloRequest, MAX_FRAME_BYTES, ProtocolVersion, QueryEnvelope,
    QueryResponse, READ_ONLY_RECOVERY_CAPABILITY,
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
        let mut stream = connect_verified_peer(&config).await?;
        let proof = load_admin_proof(&config);
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
