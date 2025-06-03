use alloc::{sync::Arc, vec, vec::Vec};
use arceos_posix_api::ctypes::{
    AF_INET, IPPROTO_TCP, IPPROTO_UDP, MAXADDRS, SOCK_STREAM, addrinfo, aibuf, aibuf_sa, in_addr,
    size_t, sockaddr, sockaddr_in, socklen_t, stat,
};
use arceos_posix_api::{FileLike, add_file_like, get_file_like};
use core::ffi::{CStr, c_char, c_int, c_void};
use core::mem::size_of;
use core::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

use axerrno::{LinuxError, LinuxResult};
use axio::PollState;
use axnet::{TcpSocket, UdpSocket};
use axsync::Mutex;
use num_enum::TryFromPrimitive;

#[derive(TryFromPrimitive, Debug)]
#[repr(u8)]
#[allow(non_camel_case_types)]
pub enum SocketOptionLevel {
    IP = 0,
    Socket = 1,
    Tcp = 6,
    IPv6 = 41,
}

pub enum Socket {
    Udp(Mutex<UdpSocket>),
    Tcp(Mutex<TcpSocket>),
}

impl Socket {
    fn add_to_fd_table(self) -> LinuxResult<c_int> {
        add_file_like(Arc::new(self))
    }

    fn from_fd(fd: c_int) -> LinuxResult<Arc<Self>> {
        let f = get_file_like(fd)?;
        f.into_any()
            .downcast::<Self>()
            .map_err(|_| LinuxError::EINVAL)
    }

    fn send(&self, buf: &[u8]) -> LinuxResult<usize> {
        match self {
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().send(buf)?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().send(buf)?),
        }
    }

    fn recv(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        match self {
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().recv_from(buf).map(|e| e.0)?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().recv(buf)?),
        }
    }

    pub fn poll(&self) -> LinuxResult<PollState> {
        match self {
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().poll()?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().poll()?),
        }
    }

    fn local_addr(&self) -> LinuxResult<SocketAddr> {
        match self {
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().local_addr()?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().local_addr()?),
        }
    }

    fn peer_addr(&self) -> LinuxResult<SocketAddr> {
        match self {
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().peer_addr()?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().peer_addr()?),
        }
    }

    fn bind(&self, addr: SocketAddr) -> LinuxResult {
        match self {
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().bind(addr)?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().bind(addr)?),
        }
    }

    fn connect(&self, addr: SocketAddr) -> LinuxResult {
        match self {
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().connect(addr)?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().connect(addr)?),
        }
    }

    fn sendto(&self, buf: &[u8], addr: SocketAddr) -> LinuxResult<usize> {
        match self {
            // diff: must bind before sendto
            Socket::Udp(udpsocket) => {
                let udpsocket = udpsocket.lock();
                udpsocket
                    .bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0))
                    .map_err(|_| LinuxError::EISCONN)?;
                udpsocket
                    .send_to(buf, addr)
                    .map_err(|_| LinuxError::EISCONN)
            }
            Socket::Tcp(_) => Err(LinuxError::EISCONN),
        }
    }

    fn recvfrom(&self, buf: &mut [u8]) -> LinuxResult<(usize, Option<SocketAddr>)> {
        match self {
            // diff: must bind before recvfrom
            Socket::Udp(udpsocket) => Ok(udpsocket
                .lock()
                .recv_from(buf)
                .map(|res| (res.0, Some(res.1)))?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().recv(buf).map(|res| (res, None))?),
        }
    }

    fn listen(&self) -> LinuxResult {
        match self {
            Socket::Udp(_) => Err(LinuxError::EOPNOTSUPP),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().listen()?),
        }
    }

    fn accept(&self) -> LinuxResult<TcpSocket> {
        match self {
            Socket::Udp(_) => Err(LinuxError::EOPNOTSUPP),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().accept()?),
        }
    }

    fn shutdown(&self) -> LinuxResult {
        match self {
            Socket::Udp(udpsocket) => {
                let udpsocket = udpsocket.lock();
                udpsocket.peer_addr()?;
                udpsocket.shutdown()?;
                Ok(())
            }

            Socket::Tcp(tcpsocket) => {
                let tcpsocket = tcpsocket.lock();
                tcpsocket.peer_addr()?;
                tcpsocket.shutdown()?;
                Ok(())
            }
        }
    }

    fn setsockopt(&self, level: u8, _optname: u8, _optval: &[u8]) -> LinuxResult {
        let Ok(level) = SocketOptionLevel::try_from(level) else {
            error!("[setsockopt()] level {level} not supported");
            unimplemented!();
        };
        match self {
            Socket::Udp(udpsocket) => {
                let _udpsocket = udpsocket.lock();
                // TODO: Implement setsockopt for UDP
                match level {
                    SocketOptionLevel::IP => {
                        warn!("unimplemented setsockopt for UDP");
                        Ok(())
                    }
                    SocketOptionLevel::Socket => {
                        warn!("unimplemented setsockopt for UDP");
                        Ok(())
                    }
                    _ => {
                        warn!("unimplemented setsockopt for UDP");
                        Ok(())
                    }
                }
            }

            Socket::Tcp(_tcpsocket) => {
                warn!("unimplemented setsockopt for TCP");
                Ok(())
            }
        }
    }
}

impl FileLike for Socket {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        self.recv(buf)
    }

    fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        self.send(buf)
    }

    fn stat(&self) -> LinuxResult<stat> {
        // TODO: implement socket stat
        let _mode = 0o140000 | 0o777u32; // S_IFSOCK | rwxrwxrwx
        Ok(stat::default())
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn core::any::Any + Send + Sync> {
        self
    }

    fn poll(&self) -> LinuxResult<PollState> {
        self.poll()
    }

    fn set_nonblocking(&self, nonblock: bool) -> LinuxResult {
        match self {
            Socket::Udp(udpsocket) => udpsocket.lock().set_nonblocking(nonblock),
            Socket::Tcp(tcpsocket) => tcpsocket.lock().set_nonblocking(nonblock),
        }
        Ok(())
    }
}

fn into_sockaddr(addr: SocketAddr) -> (sockaddr, socklen_t) {
    debug!("    Sockaddr: {}", addr);
    match addr {
        SocketAddr::V4(addr) => (
            unsafe { *(&sockaddr_in::from(addr) as *const _ as *const sockaddr) },
            size_of::<sockaddr>() as _,
        ),
        SocketAddr::V6(_) => panic!("IPv6 is not supported"),
    }
}

fn from_sockaddr(addr: *const sockaddr, addrlen: socklen_t) -> LinuxResult<SocketAddr> {
    if addr.is_null() {
        return Err(LinuxError::EFAULT);
    }
    if addrlen != size_of::<sockaddr>() as _ {
        return Err(LinuxError::EINVAL);
    }

    let mid = unsafe { *(addr as *const sockaddr_in) };
    if mid.sin_family != AF_INET as u16 {
        return Err(LinuxError::EINVAL);
    }

    let res = SocketAddr::V4(mid.into());
    debug!("    load sockaddr:{:#x} => {:?}", addr as usize, res);
    Ok(res)
}

/// Convert a C string to a Rust string
pub fn char_ptr_to_str<'a>(str: *const c_char) -> LinuxResult<&'a str> {
    if str.is_null() {
        Err(LinuxError::EFAULT)
    } else {
        let str = str as *const _;
        unsafe { CStr::from_ptr(str) }
            .to_str()
            .map_err(|_| LinuxError::EINVAL)
    }
}

pub const SOCKET_TYPE_MASK: i32 = 0xFF;
/// Set O_NONBLOCK flag on the open fd
pub const SOCK_NONBLOCK: u32 = 0x800;
/// Set FD_CLOEXEC flag on the new fd
pub const SOCK_CLOEXEC: u32 = 0x80000;

/// Create an socket for communication.
///
/// Return the socket file descriptor.
pub fn sys_socket(domain: c_int, socktype: c_int, protocol: c_int) -> LinuxResult<isize> {
    debug!("sys_socket <= {} {} {}", domain, socktype, protocol);
    let (domain, socktype, protocol) = (
        domain as u32,
        (socktype & SOCKET_TYPE_MASK) as u32,
        protocol as u32,
    );
    match (domain, socktype, protocol) {
        (AF_INET, SOCK_STREAM, IPPROTO_TCP) | (AF_INET, SOCK_STREAM, 0) => {
            let socket = Socket::Tcp(Mutex::new(TcpSocket::new()));
            let _ = socket.set_nonblocking((socktype & SOCK_NONBLOCK) != 0);
            // TODO: set close on exec
            // socket.set_close_on_exec((socktype & SOCK_CLOEXEC) != 0);
            socket
                .add_to_fd_table()
                .map(|fd| fd as isize)
                .map_err(|_| LinuxError::EMFILE)
        }
        (AF_INET, SOCK_DGRAM, IPPROTO_UDP) | (AF_INET, SOCK_DGRAM, 0) => {
            Socket::Udp(Mutex::new(UdpSocket::new()))
                .add_to_fd_table()
                .map(|fd| fd as isize)
                .map_err(|_| LinuxError::EMFILE)
        }
        _ => Err(LinuxError::EINVAL),
    }
}

/// Bind a address to a socket.
///
/// Return 0 if success.
pub fn sys_bind(
    socket_fd: c_int,
    socket_addr: *const sockaddr,
    addrlen: socklen_t,
) -> LinuxResult<isize> {
    debug!(
        "sys_bind <= {} {:#x} {}",
        socket_fd, socket_addr as usize, addrlen
    );
    let addr = from_sockaddr(socket_addr, addrlen)?;
    Socket::from_fd(socket_fd)?.bind(addr)?;
    Ok(0)
}

/// Connects the socket to the address specified.
///
/// Return 0 if success.
pub fn sys_connect(
    socket_fd: c_int,
    socket_addr: *const sockaddr,
    addrlen: socklen_t,
) -> LinuxResult<isize> {
    debug!(
        "sys_connect <= {} {:#x} {}",
        socket_fd, socket_addr as usize, addrlen
    );
    let addr = from_sockaddr(socket_addr, addrlen)?;
    Socket::from_fd(socket_fd)?.connect(addr)?;
    Ok(0)
}

/// Send a message on a socket to the address specified.
///
/// Return the number of bytes sent if success.
pub fn sys_sendto(
    socket_fd: c_int,
    buf_ptr: *const c_void,
    len: size_t,
    flag: c_int, // currently not used
    socket_addr: *const sockaddr,
    addrlen: socklen_t,
) -> LinuxResult<isize> {
    debug!(
        "sys_sendto <= {} {:#x} {} {} {:#x} {}",
        socket_fd, buf_ptr as usize, len, flag, socket_addr as usize, addrlen
    );
    if buf_ptr.is_null() {
        return Err(LinuxError::EFAULT);
    }
    let addr = from_sockaddr(socket_addr, addrlen)?;
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };
    Socket::from_fd(socket_fd)?
        .sendto(buf, addr)
        .map(|res| res as isize)
        .map_err(|_| LinuxError::EISCONN)
}

/// Send a message on a socket to the address connected.
///
/// Return the number of bytes sent if success.
pub fn sys_send(
    socket_fd: c_int,
    buf_ptr: *const c_void,
    len: size_t,
    flag: c_int, // currently not used
) -> LinuxResult<isize> {
    debug!(
        "sys_sendto <= {} {:#x} {} {}",
        socket_fd, buf_ptr as usize, len, flag
    );
    if buf_ptr.is_null() {
        return Err(LinuxError::EFAULT);
    }
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };
    Socket::from_fd(socket_fd)?
        .send(buf)
        .map(|res| res as isize)
        .map_err(|_| LinuxError::EISCONN)
}

/// Receive a message on a socket and get its source address.
///
/// Return the number of bytes received if success.
pub fn sys_recvfrom(
    socket_fd: c_int,
    buf_ptr: *mut c_void,
    len: size_t,
    flag: c_int, // currently not used
    socket_addr: *mut sockaddr,
    addrlen: *mut socklen_t,
) -> LinuxResult<isize> {
    debug!(
        "sys_recvfrom <= {} {:#x} {} {} {:#x} {:#x}",
        socket_fd, buf_ptr as usize, len, flag, socket_addr as usize, addrlen as usize
    );
    if buf_ptr.is_null() || socket_addr.is_null() || addrlen.is_null() {
        return Err(LinuxError::EFAULT);
    }
    debug!(
        "sys_recvfrom <= {} {:#x} {} {} {:#x} {:#x}",
        socket_fd, buf_ptr as usize, len, flag, socket_addr as usize, addrlen as usize
    );
    let socket = Socket::from_fd(socket_fd)?;
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };

    let res = socket.recvfrom(buf)?;

    if let Some(addr) = res.1 {
        unsafe {
            (*socket_addr, *addrlen) = into_sockaddr(addr);
        }
    }
    Ok(res.0 as isize)
}

/// Receive a message on a socket.
///
/// Return the number of bytes received if success.
pub fn sys_recv(
    socket_fd: c_int,
    buf_ptr: *mut c_void,
    len: size_t,
    flag: c_int, // currently not used
) -> LinuxResult<isize> {
    debug!(
        "sys_recv <= {} {:#x} {} {}",
        socket_fd, buf_ptr as usize, len, flag
    );
    if buf_ptr.is_null() {
        return Err(LinuxError::EFAULT);
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };
    Socket::from_fd(socket_fd)?
        .recv(buf)
        .map(|res| res as isize)
        .map_err(|_| LinuxError::EISCONN)
}

/// Listen for connections on a socket
///
/// Return 0 if success.
pub fn sys_listen(
    socket_fd: c_int,
    backlog: c_int, // currently not used
) -> LinuxResult<isize> {
    debug!("sys_listen <= {} {}", socket_fd, backlog);
    Socket::from_fd(socket_fd)?.listen()?;
    Ok(0)
}

/// Accept for connections on a socket
///
/// Return file descriptor for the accepted socket if success.
pub fn sys_accept(
    socket_fd: c_int,
    socket_addr: *mut sockaddr,
    socket_len: *mut socklen_t,
) -> LinuxResult<isize> {
    debug!(
        "sys_accept <= {} {:#x} {:#x}",
        socket_fd, socket_addr as usize, socket_len as usize
    );
    if socket_addr.is_null() || socket_len.is_null() {
        return Err(LinuxError::EFAULT);
    }
    let socket = Socket::from_fd(socket_fd)?;
    let new_socket = socket.accept()?;
    let addr = new_socket.peer_addr()?;
    let new_fd = Socket::add_to_fd_table(Socket::Tcp(Mutex::new(new_socket)))?;
    unsafe {
        (*socket_addr, *socket_len) = into_sockaddr(addr);
    }
    Ok(new_fd as isize)
}

/// Shut down a full-duplex connection.
///
/// Return 0 if success.
pub fn sys_shutdown(
    socket_fd: c_int,
    flag: c_int, // currently not used
) -> LinuxResult<isize> {
    debug!("sys_shutdown <= {} {}", socket_fd, flag);
    Socket::from_fd(socket_fd)?.shutdown()?;
    Ok(0)
}

/// Query addresses for a domain name.
///
/// Only IPv4. Ports are always 0. Ignore servname and hint.
/// Results' ai_flags and ai_canonname are 0 or NULL.
///
/// Return address number if success.
pub unsafe fn sys_getaddrinfo(
    nodename: *const c_char,
    servname: *const c_char,
    _hints: *const addrinfo,
    res: *mut *mut addrinfo,
) -> LinuxResult<isize> {
    let name = char_ptr_to_str(nodename);
    let port = char_ptr_to_str(servname);
    debug!("sys_getaddrinfo <= {:?} {:?}", name, port);
    if nodename.is_null() && servname.is_null() {
        return Ok(0);
    }
    if res.is_null() {
        return Err(LinuxError::EFAULT);
    }

    let port = port.map_or(0, |p| p.parse::<u16>().unwrap_or(0));
    let ip_addrs = if let Ok(domain) = name {
        if let Ok(a) = domain.parse::<IpAddr>() {
            vec![a]
        } else {
            axnet::dns_query(domain)?
        }
    } else {
        vec![Ipv4Addr::LOCALHOST.into()]
    };

    let len = ip_addrs.len().min(MAXADDRS as usize);
    if len == 0 {
        return Ok(0);
    }

    let mut out: Vec<aibuf> = Vec::with_capacity(len);
    for (i, &ip) in ip_addrs.iter().enumerate().take(len) {
        let buf = match ip {
            IpAddr::V4(ip) => aibuf {
                ai: addrinfo {
                    ai_family: AF_INET as _,
                    // TODO: This is a hard-code part, only return TCP parameters
                    ai_socktype: SOCK_STREAM as _,
                    ai_protocol: IPPROTO_TCP as _,
                    ai_addrlen: size_of::<sockaddr_in>() as _,
                    ai_addr: core::ptr::null_mut(),
                    ai_canonname: core::ptr::null_mut(),
                    ai_next: core::ptr::null_mut(),
                    ai_flags: 0,
                },
                sa: aibuf_sa {
                    sin: SocketAddrV4::new(ip, port).into(),
                },
                slot: i as i16,
                lock: [0],
                ref_: 0,
            },
            _ => panic!("IPv6 is not supported"),
        };
        out.push(buf);
        out[i].ai.ai_addr = unsafe { core::ptr::addr_of_mut!(out[i].sa.sin) as *mut sockaddr };
        if i > 0 {
            out[i - 1].ai.ai_next = core::ptr::addr_of_mut!(out[i].ai);
        }
    }

    out[0].ref_ = len as i16;
    unsafe { *res = core::ptr::addr_of_mut!(out[0].ai) };
    core::mem::forget(out); // drop in `sys_freeaddrinfo`
    Ok(len as isize)
}

/// Free queried `addrinfo` struct
pub unsafe fn sys_freeaddrinfo(res: *mut addrinfo) {
    if res.is_null() {
        return;
    }
    let aibuf_ptr = res as *mut aibuf;
    let len = unsafe { *aibuf_ptr }.ref_ as usize;
    assert_eq!(unsafe { *aibuf_ptr }.slot, 0);
    assert!(len > 0);
    let vec = unsafe { Vec::from_raw_parts(aibuf_ptr, len, len) }; // TODO: lock
    drop(vec);
}

/// Get current address to which the socket sockfd is bound.
pub fn sys_getsockname(
    sock_fd: c_int,
    addr: *mut sockaddr,
    addrlen: *mut socklen_t,
) -> LinuxResult<isize> {
    debug!(
        "sys_getsockname <= {} {:#x} {:#x}",
        sock_fd, addr as usize, addrlen as usize
    );
    if addr.is_null() || addrlen.is_null() {
        return Err(LinuxError::EFAULT);
    }
    if unsafe { *addrlen } < size_of::<sockaddr>() as u32 {
        return Err(LinuxError::EINVAL);
    }
    unsafe {
        (*addr, *addrlen) = into_sockaddr(Socket::from_fd(sock_fd)?.local_addr()?);
    }
    Ok(0)
}

/// Get peer address to which the socket sockfd is connected.
pub fn sys_getpeername(
    sock_fd: c_int,
    addr: *mut sockaddr,
    addrlen: *mut socklen_t,
) -> LinuxResult<isize> {
    debug!(
        "sys_getpeername <= {} {:#x} {:#x}",
        sock_fd, addr as usize, addrlen as usize
    );
    if addr.is_null() || addrlen.is_null() {
        return Err(LinuxError::EFAULT);
    }
    if unsafe { *addrlen } < size_of::<sockaddr>() as u32 {
        return Err(LinuxError::EINVAL);
    }
    unsafe {
        (*addr, *addrlen) = into_sockaddr(Socket::from_fd(sock_fd)?.peer_addr()?);
    }
    Ok(0)
}

/// Set options on sockets
pub fn sys_setsockopt(
    sockfd: i32,
    level: usize,
    optname: usize,
    optval: *const u8,
    optlen: u32,
) -> LinuxResult<isize> {
    debug!(
        "sys_setsockopt <= {} {} {} {:#x} {}",
        sockfd, level, optname, optval as usize, optlen
    );
    if optval.is_null() {
        return Err(LinuxError::EFAULT);
    }

    let buf = unsafe { core::slice::from_raw_parts(optval, optlen as usize) };
    Socket::from_fd(sockfd)?.setsockopt(level as u8, optname as u8, buf)?;
    Ok(0)
}
