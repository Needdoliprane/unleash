//! Read-only ownership observations of an exact TCP loopback listener.
//!
//! An observation grants no process control, connection-peer identity,
//! confinement or inference-placement authority. The expected identity must
//! come from the caller's managed child, not an address-based process search.

use std::fmt;
use std::net::SocketAddr;

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
