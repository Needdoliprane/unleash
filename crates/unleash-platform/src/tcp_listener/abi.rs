//! Darwin SDK `sys/proc_info.h` socket record ABI. Only the TCP union member
//! is decoded. Other protocol storage remains opaque and has the SDK union's
//! full 528-byte size and eight-byte alignment.

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ProcFileInfo {
    open_flags: u32,
    status: u32,
    offset: i64,
    file_type: i32,
    guard_flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VinfoStat {
    dev: u32,
    mode: u16,
    nlink: u16,
    ino: u64,
    uid: u32,
    gid: u32,
    atime: i64,
    atime_ns: i64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
    birthtime: i64,
    birthtime_ns: i64,
    size: i64,
    blocks: i64,
    block_size: i32,
    flags: u32,
    generation: u32,
    rdev: u32,
    spare: [i64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct SockBufInfo {
    cc: u32,
    hiwat: u32,
    mbcnt: u32,
    mbmax: u32,
    lowat: u32,
    flags: i16,
    timeo: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Ipv6Info {
    hop_limit: u8,
    checksum: i32,
    interface_index: u16,
    hops: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct InSockInfo {
    pub(super) foreign_port: i32,
    pub(super) local_port: i32,
    pub(super) generation: u64,
    flags: u32,
    flow: u32,
    pub(super) version_flags: u8,
    ttl: u8,
    reserved: u32,
    pub(super) foreign_address: [u32; 4],
    pub(super) local_address: [u32; 4],
    ipv4_tos: u8,
    ipv6: Ipv6Info,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct TcpSockInfo {
    pub(super) ip: InSockInfo,
    pub(super) state: i32,
    timers: [i32; 4],
    mss: i32,
    flags: u32,
    reserved: u32,
    tcp_handle: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union SocketProtocol {
    pub(super) tcp: TcpSockInfo,
    storage: [u64; 66],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct SocketInfo {
    stat: VinfoStat,
    pub(super) socket_handle: u64,
    pcb_handle: u64,
    pub(super) socket_type: i32,
    pub(super) protocol_number: i32,
    pub(super) family: i32,
    pub(super) options: i16,
    linger: i16,
    state: i16,
    queue_len: i16,
    incomplete_queue_len: i16,
    queue_limit: i16,
    timeout: i16,
    error: u16,
    oob_mark: u32,
    receive: SockBufInfo,
    send: SockBufInfo,
    pub(super) kind: i32,
    reserved: u32,
    pub(super) protocol: SocketProtocol,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct SocketFdInfo {
    file: ProcFileInfo,
    pub(super) socket: SocketInfo,
}

pub(super) const fn supported() -> bool {
    use std::mem::{align_of, offset_of, size_of};
    size_of::<ProcFileInfo>() == 24
        && size_of::<VinfoStat>() == 136
        && size_of::<InSockInfo>() == 80
        && size_of::<TcpSockInfo>() == 120
        && size_of::<SocketProtocol>() == 528
        && size_of::<SocketInfo>() == 768
        && size_of::<SocketFdInfo>() == 792
        && align_of::<SocketFdInfo>() == 8
        && offset_of!(SocketFdInfo, socket) == 24
        && offset_of!(SocketInfo, protocol) == 240
        && offset_of!(SocketInfo, kind) == 232
        && offset_of!(InSockInfo, generation) == 8
        && offset_of!(InSockInfo, foreign_address) == 32
        && offset_of!(InSockInfo, local_address) == 48
        && offset_of!(TcpSockInfo, state) == 80
}
