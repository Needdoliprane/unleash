//! Native checks for the exact socket carried by an owned loopback request.

use std::net::SocketAddr;
use std::os::fd::AsFd as _;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::Instant;
use unleash_platform::{
    OsProcessIdentity, TcpListenerError, TcpListenerObservation, observe_owned_tcp_connection,
    observe_owned_tcp_listener, verify_owned_tcp_connection,
};

use crate::connect_transport::ConnectError;

pub(crate) async fn listener(
    process: OsProcessIdentity,
    endpoint: SocketAddr,
) -> Result<TcpListenerObservation, ConnectError> {
    tokio::task::spawn_blocking(move || observe_owned_tcp_listener(&process, endpoint))
        .await
        .map_err(|_| ConnectError::OwnershipUnavailable)?
        .map_err(|_| ConnectError::OwnershipUnavailable)
}

pub(crate) async fn check(
    mut socket: TcpStream,
    listener: TcpListenerObservation,
    deadline: Instant,
) -> Result<TcpStream, ConnectError> {
    loop {
        if Instant::now() >= deadline {
            return Err(ConnectError::DeadlineExpired);
        }
        let current = listener.clone();
        // Move the unique socket owner into the native task. If the caller is
        // cancelled, the task can only finish inspection and drop the socket.
        let (returned, ready) = tokio::task::spawn_blocking(move || {
            let result = match observe_owned_tcp_connection(
                current.process_identity(),
                &current,
                socket.as_fd(),
            ) {
                Ok(fact) => verify_owned_tcp_connection(&fact, socket.as_fd())
                    .map(|()| true)
                    .map_err(|_| ConnectError::OwnershipUnavailable),
                Err(TcpListenerError::ConnectionNotFound) => Ok(false),
                Err(_) => Err(ConnectError::OwnershipUnavailable),
            };
            (socket, result)
        })
        .await
        .map_err(|_| ConnectError::OwnershipUnavailable)?;
        if Instant::now() >= deadline {
            return Err(ConnectError::DeadlineExpired);
        }
        if ready? {
            return Ok(returned);
        }
        socket = returned;
        // The listener can be valid before its process accepts the queued TCP
        // connection. Only this absence is retried, without another connect.
        tokio::time::sleep_until((Instant::now() + Duration::from_millis(10)).min(deadline)).await;
    }
}
