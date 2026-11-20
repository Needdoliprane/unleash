//! Read the exact expected PID only, using the installed Darwin SDK ABI.

use std::mem::{MaybeUninit, size_of};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::{Duration, Instant};

use super::{
    OsProcessIdentity, TcpConnectionObservation, TcpListenerError, TcpListenerObservation, abi,
    verify_owned_tcp_listener,
};
use crate::{PlatformError, inspect_process_identity};

const PROC_PIDFDSOCKETINFO: i32 = 3;
const PROX_FDTYPE_SOCKET: u32 = 2;
const SOCKINFO_TCP: i32 = 2;
const TSI_S_LISTEN: i32 = 1;
const TSI_S_ESTABLISHED: i32 = 4;
const SO_ACCEPTCONN: i16 = 0x0002;
const SO_REUSEPORT: i16 = 0x0200;
// This bound fails the entire observation; no prefix is treated as complete.
const MAX_FD_LIST_BYTES: usize = 1_048_576;
const MAX_ATTEMPTS: usize = 3;
const INSPECTION_BUDGET: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Descriptor {
    fd: i32,
    kind: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Candidate {
    fd: i32,
    generation: u64,
    handle: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ConnectionSocket {
    local: SocketAddr,
    peer: SocketAddr,
    generation: u64,
    handle: u64,
}

pub(super) fn observe(
    expected: &OsProcessIdentity,
    endpoint: SocketAddr,
) -> Result<TcpListenerObservation, TcpListenerError> {
    if !abi::supported() || size_of::<libc::proc_fdinfo>() != 8 {
        return Err(TcpListenerError::AbiMismatch);
    }
    let deadline = Instant::now() + INSPECTION_BUDGET;
    for _ in 0..MAX_ATTEMPTS {
        verify_identity(expected)?;
        let first_fds = descriptors(expected.pid)?;
        let first = candidates(expected.pid, &first_fds, endpoint, deadline)?;
        let last_fds = descriptors(expected.pid)?;
        if first_fds != last_fds {
            continue;
        }
        let last = candidates(expected.pid, &last_fds, endpoint, deadline)?;
        verify_identity(expected)?;
        if first != last {
            continue;
        }
        if Instant::now() >= deadline {
            return Err(TcpListenerError::Unavailable);
        }
        let selected = select_candidate(&first)?;
        return Ok(TcpListenerObservation {
            process: expected.clone(),
            endpoint,
            generation: selected.generation,
            socket_handle: selected.handle,
        });
    }
    Err(TcpListenerError::IncompleteDescriptorList)
}

fn verify_identity(expected: &OsProcessIdentity) -> Result<(), TcpListenerError> {
    match inspect_process_identity(expected.pid) {
        Ok(actual) if actual == *expected => Ok(()),
        Ok(_) | Err(PlatformError::ProcessNotFound { .. }) => {
            Err(TcpListenerError::IdentityChanged)
        }
        Err(PlatformError::PermissionDenied { .. }) => Err(TcpListenerError::AccessDenied),
        Err(PlatformError::OsCall { source, .. }) => Err(native_error(source.raw_os_error())),
        Err(_) => Err(TcpListenerError::Unavailable),
    }
}

fn native_error(errno: Option<i32>) -> TcpListenerError {
    match errno {
        Some(libc::EPERM | libc::EACCES) => TcpListenerError::AccessDenied,
        Some(libc::ESRCH) => TcpListenerError::IdentityChanged,
        Some(libc::EBADF | libc::ENOTSOCK) => TcpListenerError::IncompleteDescriptorList,
        Some(libc::EINVAL) => TcpListenerError::AbiMismatch,
        Some(libc::ENOSYS | libc::ENOTSUP) => TcpListenerError::UnsupportedPlatform,
        _ => TcpListenerError::Unavailable,
    }
}

fn pid_number(pid: u32) -> Result<i32, TcpListenerError> {
    i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or(TcpListenerError::IdentityChanged)
}

#[allow(unsafe_code)]
fn descriptors(pid: u32) -> Result<Vec<Descriptor>, TcpListenerError> {
    let pid = pid_number(pid)?;
    // SAFETY: this thread's errno slot is valid; null buffer with size zero is
    // the documented descriptor-list sizing call and does not write memory.
    let needed = unsafe {
        *libc::__error() = 0;
        libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0)
    };
    if needed <= 0 {
        return Err(native_error(std::io::Error::last_os_error().raw_os_error()));
    }
    let bytes = usize::try_from(needed).map_err(|_| TcpListenerError::AbiMismatch)?;
    if bytes > MAX_FD_LIST_BYTES {
        return Err(TcpListenerError::IncompleteDescriptorList);
    }
    if bytes % size_of::<libc::proc_fdinfo>() != 0 {
        return Err(TcpListenerError::AbiMismatch);
    }
    let count = bytes / size_of::<libc::proc_fdinfo>();
    let mut records = Vec::new();
    records
        .try_reserve_exact(count)
        .map_err(|_| TcpListenerError::Unavailable)?;
    records.resize_with(count, || libc::proc_fdinfo {
        proc_fd: 0,
        proc_fdtype: 0,
    });
    // SAFETY: records is fully initialized and aligned, with exactly needed
    // writable bytes. The kernel receives that exact capacity.
    let returned = unsafe {
        *libc::__error() = 0;
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            records.as_mut_ptr().cast(),
            needed,
        )
    };
    let errno = std::io::Error::last_os_error().raw_os_error();
    if returned < 0 || returned == 0 && errno != Some(0) {
        return Err(native_error(errno));
    }
    let returned = usize::try_from(returned).map_err(|_| TcpListenerError::AbiMismatch)?;
    if returned >= bytes {
        return Err(TcpListenerError::IncompleteDescriptorList);
    }
    if returned % size_of::<libc::proc_fdinfo>() != 0 {
        return Err(TcpListenerError::AbiMismatch);
    }
    records.truncate(returned / size_of::<libc::proc_fdinfo>());
    let mut result = Vec::new();
    result
        .try_reserve_exact(records.len())
        .map_err(|_| TcpListenerError::Unavailable)?;
    for record in records {
        if record.proc_fd < 0 {
            return Err(TcpListenerError::AbiMismatch);
        }
        result.push(Descriptor {
            fd: record.proc_fd,
            kind: record.proc_fdtype,
        });
    }
    result.sort_unstable();
    if result
        .windows(2)
        .any(|pair| matches!(pair, [first, last] if first.fd == last.fd))
    {
        return Err(TcpListenerError::AbiMismatch);
    }
    Ok(result)
}

#[allow(unsafe_code)]
fn socket_record(pid: u32, fd: i32) -> Result<abi::SocketFdInfo, TcpListenerError> {
    let pid = pid_number(pid)?;
    let length =
        i32::try_from(size_of::<abi::SocketFdInfo>()).map_err(|_| TcpListenerError::AbiMismatch)?;
    let mut record = MaybeUninit::<abi::SocketFdInfo>::zeroed();
    // SAFETY: record is aligned and has the full checked Darwin SDK size. No
    // field is read until the kernel reports that exact initialized size.
    let returned = unsafe {
        *libc::__error() = 0;
        libc::proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDSOCKETINFO,
            record.as_mut_ptr().cast(),
            length,
        )
    };
    if returned <= 0 {
        return Err(native_error(std::io::Error::last_os_error().raw_os_error()));
    }
    if returned != length {
        return Err(TcpListenerError::AbiMismatch);
    }
    // SAFETY: the complete record was written; its C fields are integers,
    // arrays and an opaque protocol union with no invalid bit representations.
    Ok(unsafe { record.assume_init() })
}

fn candidates(
    pid: u32,
    descriptors: &[Descriptor],
    endpoint: SocketAddr,
    deadline: Instant,
) -> Result<Vec<Candidate>, TcpListenerError> {
    let mut result = Vec::new();
    for descriptor in descriptors
        .iter()
        .filter(|descriptor| descriptor.kind == PROX_FDTYPE_SOCKET)
    {
        if Instant::now() >= deadline {
            return Err(TcpListenerError::Unavailable);
        }
        let record = socket_record(pid, descriptor.fd)?;
        if let Some(candidate) = matching_socket(descriptor.fd, &record.socket, endpoint)? {
            result
                .try_reserve(1)
                .map_err(|_| TcpListenerError::Unavailable)?;
            result.push(candidate);
        }
    }
    Ok(result)
}

#[allow(unsafe_code)]
fn matching_socket(
    fd: i32,
    socket: &abi::SocketInfo,
    endpoint: SocketAddr,
) -> Result<Option<Candidate>, TcpListenerError> {
    if socket.kind != SOCKINFO_TCP {
        return Ok(None);
    }
    if socket.socket_type != libc::SOCK_STREAM || socket.protocol_number != libc::IPPROTO_TCP {
        return Err(TcpListenerError::AbiMismatch);
    }
    // SAFETY: the checked native discriminator is SOCKINFO_TCP and the full
    // union is initialized. This member consists only of valid integer bits.
    let tcp = unsafe { socket.protocol.tcp };
    if tcp.state != TSI_S_LISTEN {
        return Ok(None);
    }
    if socket.options & SO_ACCEPTCONN == 0 {
        return Err(TcpListenerError::AbiMismatch);
    }
    let local = local_address(socket.family, &tcp.ip)?;
    if local != endpoint {
        return Ok(None);
    }
    if socket.options & SO_REUSEPORT != 0 {
        return Err(TcpListenerError::AmbiguousListener);
    }
    if tcp.ip.generation == 0 {
        return Err(TcpListenerError::AbiMismatch);
    }
    Ok(Some(Candidate {
        fd,
        generation: tcp.ip.generation,
        handle: socket.socket_handle,
    }))
}

fn local_address(family: i32, ip: &abi::InSockInfo) -> Result<SocketAddr, TcpListenerError> {
    socket_address(family, ip.version_flags, ip.local_port, ip.local_address)
}

fn socket_address(
    family: i32,
    version_flags: u8,
    raw_port: i32,
    words: [u32; 4],
) -> Result<SocketAddr, TcpListenerError> {
    let raw_port = u16::try_from(raw_port).map_err(|_| TcpListenerError::AbiMismatch)?;
    let port = u16::from_be(raw_port);
    let mut address = [0_u8; 16];
    for (chunk, word) in address.chunks_exact_mut(4).zip(words) {
        chunk.copy_from_slice(&word.to_ne_bytes());
    }
    let ip = match family {
        libc::AF_INET if version_flags & 1 != 0 => IpAddr::V4(Ipv4Addr::new(
            address[12],
            address[13],
            address[14],
            address[15],
        )),
        libc::AF_INET6 if version_flags & 2 != 0 => IpAddr::V6(Ipv6Addr::from(address)),
        _ => return Err(TcpListenerError::AbiMismatch),
    };
    Ok(SocketAddr::new(ip, port))
}

fn select_candidate(candidates: &[Candidate]) -> Result<Candidate, TcpListenerError> {
    let selected = candidates
        .first()
        .copied()
        .ok_or(TcpListenerError::ListenerNotFound)?;
    // Duplicate FDs for one kernel socket do not invent a second listener.
    if candidates.iter().any(|candidate| {
        candidate.generation != selected.generation || candidate.handle != selected.handle
    }) {
        return Err(TcpListenerError::AmbiguousListener);
    }
    Ok(selected)
}

pub(super) fn observe_connection(
    expected: &OsProcessIdentity,
    listener: &TcpListenerObservation,
    client: BorrowedFd<'_>,
) -> Result<TcpConnectionObservation, TcpListenerError> {
    if expected != &listener.process {
        return Err(TcpListenerError::IdentityChanged);
    }
    let deadline = Instant::now() + INSPECTION_BUDGET;
    verify_owned_tcp_listener(listener)?;
    let client_start = client_connection(client)?;
    if client_start.peer != listener.endpoint {
        return Err(TcpListenerError::ConnectionMismatch);
    }
    let accepted = stable_connection(expected, client_start, deadline)?;
    verify_owned_tcp_listener(listener)?;
    if client_connection(client)? != client_start {
        return Err(TcpListenerError::ConnectionChanged);
    }
    verify_identity(expected)?;
    if Instant::now() >= deadline {
        return Err(TcpListenerError::Unavailable);
    }
    Ok(TcpConnectionObservation {
        listener: listener.clone(),
        client_local: client_start.local,
        client_peer: client_start.peer,
        client_generation: client_start.generation,
        client_handle: client_start.handle,
        server_generation: accepted.generation,
        server_handle: accepted.handle,
    })
}

fn stable_connection(
    expected: &OsProcessIdentity,
    client: ConnectionSocket,
    deadline: Instant,
) -> Result<Candidate, TcpListenerError> {
    for _ in 0..MAX_ATTEMPTS {
        verify_identity(expected)?;
        let first_fds = descriptors(expected.pid)?;
        let first = accepted_connections(expected.pid, &first_fds, client, deadline)?;
        let last_fds = descriptors(expected.pid)?;
        if first_fds != last_fds {
            continue;
        }
        let last = accepted_connections(expected.pid, &last_fds, client, deadline)?;
        verify_identity(expected)?;
        if first != last {
            continue;
        }
        let selected = first
            .first()
            .copied()
            .ok_or(TcpListenerError::ConnectionNotFound)?;
        // Duplicated descriptors count as one observed socket, not exclusive ownership.
        if first.iter().any(|candidate| {
            candidate.generation != selected.generation || candidate.handle != selected.handle
        }) {
            return Err(TcpListenerError::AmbiguousConnection);
        }
        return Ok(selected);
    }
    Err(TcpListenerError::IncompleteDescriptorList)
}

fn accepted_connections(
    pid: u32,
    descriptors: &[Descriptor],
    client: ConnectionSocket,
    deadline: Instant,
) -> Result<Vec<Candidate>, TcpListenerError> {
    let mut matches = Vec::new();
    for descriptor in descriptors
        .iter()
        .filter(|descriptor| descriptor.kind == PROX_FDTYPE_SOCKET)
    {
        if Instant::now() >= deadline {
            return Err(TcpListenerError::Unavailable);
        }
        let record = socket_record(pid, descriptor.fd)?;
        let Some(socket) = established_connection(&record.socket)? else {
            continue;
        };
        if socket.local == client.peer && socket.peer == client.local {
            matches
                .try_reserve(1)
                .map_err(|_| TcpListenerError::Unavailable)?;
            matches.push(Candidate {
                fd: descriptor.fd,
                generation: socket.generation,
                handle: socket.handle,
            });
        }
    }
    Ok(matches)
}

fn client_connection(client: BorrowedFd<'_>) -> Result<ConnectionSocket, TcpListenerError> {
    let local = rustix::net::getsockname(client)
        .map_err(|error| native_error(Some(error.raw_os_error())))?;
    let local = SocketAddr::try_from(local).map_err(|_| TcpListenerError::ConnectionMismatch)?;
    let peer = rustix::net::getpeername(client)
        .map_err(|error| native_error(Some(error.raw_os_error())))?
        .ok_or(TcpListenerError::ConnectionMismatch)?;
    let peer = SocketAddr::try_from(peer).map_err(|_| TcpListenerError::ConnectionMismatch)?;
    if !loopback_endpoint(local) || !loopback_endpoint(peer) || local.is_ipv4() != peer.is_ipv4() {
        return Err(TcpListenerError::ConnectionMismatch);
    }
    let record = socket_record(std::process::id(), client.as_raw_fd())?;
    let socket =
        established_connection(&record.socket)?.ok_or(TcpListenerError::ConnectionMismatch)?;
    if socket.local != local || socket.peer != peer {
        return Err(TcpListenerError::ConnectionMismatch);
    }
    Ok(socket)
}

fn loopback_endpoint(endpoint: SocketAddr) -> bool {
    endpoint.ip().is_loopback()
        && endpoint.port() != 0
        && !matches!(endpoint, SocketAddr::V6(v6) if v6.scope_id()!=0 || v6.flowinfo()!=0)
}

#[allow(unsafe_code)]
fn established_connection(
    socket: &abi::SocketInfo,
) -> Result<Option<ConnectionSocket>, TcpListenerError> {
    if socket.kind != SOCKINFO_TCP {
        return Ok(None);
    }
    if socket.socket_type != libc::SOCK_STREAM || socket.protocol_number != libc::IPPROTO_TCP {
        return Err(TcpListenerError::AbiMismatch);
    }
    // SAFETY: the full native record has the checked TCP discriminator. Its
    // TCP union member contains only initialized integer bit representations.
    let tcp = unsafe { socket.protocol.tcp };
    if tcp.state != TSI_S_ESTABLISHED {
        return Ok(None);
    }
    if tcp.ip.generation == 0 || socket.socket_handle == 0 {
        return Err(TcpListenerError::AbiMismatch);
    }
    Ok(Some(ConnectionSocket {
        local: local_address(socket.family, &tcp.ip)?,
        peer: socket_address(
            socket.family,
            tcp.ip.version_flags,
            tcp.ip.foreign_port,
            tcp.ip.foreign_address,
        )?,
        generation: tcp.ip.generation,
        handle: socket.socket_handle,
    }))
}
