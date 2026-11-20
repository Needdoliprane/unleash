//! Read-only ownership observations of exact TCP loopback sockets.
//!
//! An observation grants no process control, exclusive peer ownership,
//! confinement or inference-placement authority. The expected identity must
//! come from the caller's managed child, not an address-based process search.

use std::fmt;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::fd::BorrowedFd;

use crate::OsProcessIdentity;

#[cfg(target_os = "macos")]
mod abi;
#[cfg(target_os = "macos")]
mod darwin;

/// Closed native inspection failures; none permits a weaker fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TcpListenerError {
    /// Only a nonzero port on a literal loopback address is observable here.
    #[error("invalid loopback listener endpoint")]
    InvalidEndpoint,
    /// The expected process identity is no longer the complete OS identity.
    #[error("listener process identity changed")]
    IdentityChanged,
    /// The expected process has no exact matching TCP listener.
    #[error("owned listener not found")]
    ListenerNotFound,
    /// A later read found a different socket instance at the same endpoint.
    #[error("owned listener instance changed")]
    ListenerChanged,
    /// No accepted TCP connection matches the actual client socket's tuple.
    #[error("owned accepted connection not found")]
    ConnectionNotFound,
    /// The client socket does not address the expected listener.
    #[error("connected socket binding mismatch")]
    ConnectionMismatch,
    /// The original connection observation belongs to another socket instance.
    #[error("owned connection instance changed")]
    ConnectionChanged,
    /// Distinct native socket instances match the same connected tuple.
    #[error("owned accepted connection is ambiguous")]
    AmbiguousConnection,
    /// Several distinct or reusable socket instances make ownership ambiguous.
    #[error("owned listener is ambiguous")]
    AmbiguousListener,
    /// Native inspection was denied; absence must not be inferred from this.
    #[error("listener inspection denied")]
    AccessDenied,
    /// The complete descriptor set could not be obtained within the read bound.
    #[error("listener descriptor list incomplete")]
    IncompleteDescriptorList,
    /// A native record has an unsupported size or representation.
    #[error("listener inspection ABI mismatch")]
    AbiMismatch,
    /// The native observer is unavailable, without a qualified result.
    #[error("listener inspection unavailable")]
    Unavailable,
    /// This first observer is restricted to macOS.
    #[error("listener inspection unsupported on this platform")]
    UnsupportedPlatform,
}

/// An immutable snapshot of a socket held by the exact expected process.
/// This is deliberately not a serialized permission or a durable server ID.
#[derive(Clone, PartialEq, Eq)]
pub struct TcpListenerObservation {
    process: OsProcessIdentity,
    endpoint: SocketAddr,
    generation: u64,
    // Native opaque handle is only compared locally, never projected as authority.
    socket_handle: u64,
}

impl fmt::Debug for TcpListenerObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpListenerObservation")
            .field("pid", &self.process.pid)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl TcpListenerObservation {
    /// Full process identity observed before and after the descriptor read.
    #[must_use]
    pub const fn process_identity(&self) -> &OsProcessIdentity {
        &self.process
    }

    /// Exact local address and port observed in the native TCP listener.
    #[must_use]
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    /// Native generation of this socket instance, not a process or server ID.
    #[must_use]
    pub const fn socket_generation(&self) -> u64 {
        self.generation
    }
}

/// Observe only the expected process's exact loopback TCP listener.
///
/// # Errors
/// Rejects invalid endpoints, inaccessible or incomplete native records,
/// identity changes and ambiguous listeners. No process search or connection
/// is performed. IPv6 endpoints must have zero scope and flow information.
pub fn observe_owned_tcp_listener(
    expected: &OsProcessIdentity,
    endpoint: SocketAddr,
) -> Result<TcpListenerObservation, TcpListenerError> {
    if !endpoint.ip().is_loopback()
        || endpoint.port() == 0
        || matches!(endpoint,SocketAddr::V6(v6) if v6.scope_id()!=0 || v6.flowinfo()!=0)
    {
        return Err(TcpListenerError::InvalidEndpoint);
    }
    #[cfg(target_os = "macos")]
    {
        darwin::observe(expected, endpoint)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = expected;
        Err(TcpListenerError::UnsupportedPlatform)
    }
}

/// Recheck the original process, endpoint and socket generation.
///
/// # Errors
/// Propagates failed native inspection, or rejects a replacement listener.
pub fn verify_owned_tcp_listener(
    original: &TcpListenerObservation,
) -> Result<(), TcpListenerError> {
    let current = observe_owned_tcp_listener(&original.process, original.endpoint)?;
    if current != *original {
        return Err(TcpListenerError::ListenerChanged);
    }
    Ok(())
}

/// A sampled binding between the live client FD and a socket held by the server.
/// This is neither exclusive socket ownership nor confinement or placement proof.
#[derive(Clone, PartialEq, Eq)]
pub struct TcpConnectionObservation {
    listener: TcpListenerObservation,
    client_local: SocketAddr,
    client_peer: SocketAddr,
    client_generation: u64,
    client_handle: u64,
    server_generation: u64,
    server_handle: u64,
}

impl fmt::Debug for TcpConnectionObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpConnectionObservation")
            .field("pid", &self.listener.process.pid)
            .field("client_local", &self.client_local)
            .field("client_peer", &self.client_peer)
            .finish_non_exhaustive()
    }
}

impl TcpConnectionObservation {
    /// Complete identity of the expected server process at observation time.
    #[must_use]
    pub const fn process_identity(&self) -> &OsProcessIdentity {
        &self.listener.process
    }

    /// Local endpoint read from the actual live client socket.
    #[must_use]
    pub const fn client_local_addr(&self) -> SocketAddr {
        self.client_local
    }

    /// Peer endpoint read from that same client socket.
    #[must_use]
    pub const fn client_peer_addr(&self) -> SocketAddr {
        self.client_peer
    }

    /// Native generation of the accepted server socket, distinct from LISTEN.
    #[must_use]
    pub const fn server_socket_generation(&self) -> u64 {
        self.server_generation
    }
}

/// Observe the actual connected client FD and its inverse tuple in the server.
///
/// The borrowed FD must remain the connection used by the caller. This function
/// neither connects nor reads or writes application data. Shared or transferred
/// server FDs, future ownership and proxying cannot be excluded by this sample.
/// The concurrent presence of both sockets does not establish which listener
/// accepted the connection. To check continuity across a new connection,
/// observe the listener before connecting and recheck the same FD before use.
///
/// # Errors
/// Rejects a missing accepted FD, changed listener/process/socket, mismatched
/// endpoints, inaccessible or incomplete native records and unsupported ABIs.
/// No success guarantees continued ownership after this sampled observation.
#[cfg(unix)]
pub fn observe_owned_tcp_connection(
    expected: &OsProcessIdentity,
    listener: &TcpListenerObservation,
    client: BorrowedFd<'_>,
) -> Result<TcpConnectionObservation, TcpListenerError> {
    #[cfg(target_os = "macos")]
    {
        darwin::observe_connection(expected, listener, client)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (expected, listener, client);
        Err(TcpListenerError::UnsupportedPlatform)
    }
}

/// Reobserve the original binding using the actual connection FD.
///
/// # Errors
/// Propagates inspection failures or rejects a different connection instance.
#[cfg(unix)]
pub fn verify_owned_tcp_connection(
    original: &TcpConnectionObservation,
    client: BorrowedFd<'_>,
) -> Result<(), TcpListenerError> {
    let current =
        observe_owned_tcp_connection(original.process_identity(), &original.listener, client)?;
    if current != *original {
        return Err(TcpListenerError::ConnectionChanged);
    }
    Ok(())
}
