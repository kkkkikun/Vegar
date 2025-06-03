use crate::ctypes::{FD_CLOEXEC, O_NONBLOCK, O_RDWR, timespec};
use crate::imp::fd_ops::poll_flags::*;
use crate::imp::pipe::Pipe;
use crate::imp::stdio::{stdin, stdout};
use crate::{File, ctypes};
use alloc::sync::Arc;
use alloc::vec::Vec;
use axerrno::{LinuxError, LinuxResult};
use axhal::time::{NANOS_PER_MICROS, NANOS_PER_SEC};
use axio::PollState;
use axns::{ResArc, def_resource};
use axtask::yield_now;
use core::ffi::{c_int, c_void};
use core::mem::replace;
use core::ops::Deref;
use core::ptr::drop_in_place;
use flatten_objects::FlattenObjects;
use spin::RwLock;

pub const AX_FILE_LIMIT: usize = 1024;

#[allow(dead_code)]
pub trait FileLike: Send + Sync {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize>;
    fn write(&self, buf: &[u8]) -> LinuxResult<usize>;
    fn stat(&self) -> LinuxResult<ctypes::stat>;
    fn into_any(self: Arc<Self>) -> Arc<dyn core::any::Any + Send + Sync>;
    fn poll(&self) -> LinuxResult<PollState>;
    fn set_nonblocking(&self, nonblocking: bool) -> LinuxResult;
}

def_resource! {
    pub static FD_TABLE: ResArc<RwLock<FlattenObjects<Arc<dyn FileLike>, AX_FILE_LIMIT>>> = ResArc::new();
}

impl FD_TABLE {
    /// Return a copy of the inner table.
    pub fn copy_inner(&self) -> RwLock<FlattenObjects<Arc<dyn FileLike>, AX_FILE_LIMIT>> {
        let table = self.read();
        let mut new_table = FlattenObjects::new();
        for id in table.ids() {
            let _ = new_table.add_at(id, table.get(id).unwrap().clone());
        }
        RwLock::new(new_table)
    }
}

/// Get a file by `fd`.
pub fn get_file_like(fd: c_int) -> LinuxResult<Arc<dyn FileLike>> {
    FD_TABLE
        .read()
        .get(fd as usize)
        .cloned()
        .ok_or(LinuxError::EBADF)
}

/// Add a file to the file descriptor table.
pub fn add_file_like(f: Arc<dyn FileLike>) -> LinuxResult<c_int> {
    Ok(FD_TABLE.write().add(f).map_err(|_| LinuxError::EMFILE)? as c_int)
}

/// Close a file by `fd`.
pub fn close_file_like(fd: c_int) -> LinuxResult {
    let f = FD_TABLE
        .write()
        .remove(fd as usize)
        .ok_or(LinuxError::EBADF)?;
    drop(f);
    Ok(())
}

pub fn close_all_file_like() {
    let ref_count = FD_TABLE.ref_count();
    debug!("ref count for FD_TABLE is {}", ref_count);

    if ref_count == 1 {
        let mut fd_table = FD_TABLE.write();
        let all_ids: Vec<_> = fd_table.ids().collect();
        for id in all_ids {
            drop(fd_table.remove(id));
        }
    }

    let res = FD_TABLE.deref();
    let res_ptr = FD_TABLE::as_ptr(res);
    unsafe {
        drop_in_place(res_ptr);
    }
}

/// Close a file by `fd`.
pub fn sys_close(fd: c_int) -> c_int {
    debug!("sys_close <= {}", fd);
    syscall_body!(sys_close, close_file_like(fd).map(|_| 0))
}

fn dup_fd(old_fd: c_int) -> LinuxResult<c_int> {
    let f = get_file_like(old_fd)?;
    let new_fd = add_file_like(f)?;
    Ok(new_fd)
}

/// Duplicate a file descriptor.
pub fn sys_dup(old_fd: c_int) -> c_int {
    debug!("sys_dup <= {}", old_fd);
    syscall_body!(sys_dup, dup_fd(old_fd))
}

/// Duplicate a file descriptor, but it uses the file descriptor number specified in `new_fd`.
pub fn sys_dup2(old_fd: c_int, new_fd: c_int) -> c_int {
    debug!("sys_dup2 <= old_fd: {}, new_fd: {}", old_fd, new_fd);
    syscall_body!(sys_dup2, {
        if old_fd == new_fd {
            let r = sys_fcntl(old_fd, ctypes::F_GETFD as _, 0);
            return if r >= 0 { Ok(old_fd) } else { Ok(r) };
        }
        if new_fd as usize >= AX_FILE_LIMIT {
            return Err(LinuxError::EBADF);
        }

        // check if the old fd is open
        let f = get_file_like(old_fd)?;
        // close the new_fd if it is already opened
        // ignore any error during the close
        close_file_like(new_fd).unwrap_or(());
        FD_TABLE
            .write()
            .add_at(new_fd as usize, f)
            .map_err(|_| LinuxError::EMFILE)?;

        Ok(new_fd)
    })
}

/// Manipulate file descriptor.
///
/// TODO: `SET/GET` command is ignored, hard-code stdin/stdout
pub fn sys_fcntl(fd: c_int, cmd: c_int, arg: usize) -> c_int {
    debug!("sys_fcntl <= fd: {} cmd: {} arg: {}", fd, cmd, arg);
    syscall_body!(sys_fcntl, {
        match cmd as u32 {
            ctypes::F_DUPFD => dup_fd(fd),
            ctypes::F_DUPFD_CLOEXEC => {
                // TODO: Change fd flags
                dup_fd(fd)
            }
            ctypes::F_SETFL => {
                if fd == 0 || fd == 1 || fd == 2 {
                    return Ok(0);
                }
                get_file_like(fd)?.set_nonblocking(arg & (ctypes::O_NONBLOCK as usize) > 0)?;
                Ok(0)
            }
            ctypes::F_GETFD => {
                warn!("unsupported fcntl parameters: F_GETFD, returning FD_CLOEXEC");
                Ok(FD_CLOEXEC as _)
            }
            ctypes::F_GETFL => {
                let file = get_file_like(fd)?.into_any();
                if let Some(_) = file.downcast_ref::<File>() {
                    warn!("unsupported fcntl parameters: F_GETFL, returning O_RDWR");
                    Ok(O_RDWR as _)
                } else {
                    warn!("unsupported fcntl parameters: F_GETFL, returning O_NONBLOCK");
                    Ok(O_NONBLOCK as _)
                }
            }
            _ => {
                warn!("unsupported fcntl parameters: cmd {}", cmd);
                Ok(0)
            }
        }
    })
}

#[ctor_bare::register_ctor]
fn init_stdio() {
    let mut fd_table = flatten_objects::FlattenObjects::new();
    fd_table
        .add_at(0, Arc::new(stdin()) as _)
        .unwrap_or_else(|_| panic!()); // stdin
    fd_table
        .add_at(1, Arc::new(stdout()) as _)
        .unwrap_or_else(|_| panic!()); // stdout
    fd_table
        .add_at(2, Arc::new(stdout()) as _)
        .unwrap_or_else(|_| panic!()); // stderr
    FD_TABLE.init_new(spin::RwLock::new(fd_table));
}

pub fn sys_poll(fds: &mut [PollFd], timeout: i32) -> i32 {
    debug!("sys_poll <= fds: {:?}, timeout: {}", fds, timeout);
    syscall_body!(sys_poll, {
        let block = timeout < 0;
        let timeout = if timeout < 0 {
            0
        } else {
            timeout as u64 * NANOS_PER_MICROS
        };
        sys_poll_impl(fds, timeout, block)
    })
}

pub fn sys_ppoll(fds: &mut [PollFd], timeout: *const timespec, _sigmask: *const c_void) -> i32 {
    debug!("sys_ppoll <= fds: {:?}, timeout: {:?}", fds, timeout);
    syscall_body!(sys_poll, {
        let mut block = false;
        let mut timeout_nanos: u64 = 0;
        if timeout.is_null() {
            block = true;
        } else {
            let secs;
            let nsecs;
            unsafe {
                secs = (*timeout).tv_sec;
                nsecs = (*timeout).tv_nsec;
            }
            if secs < 0 || nsecs < 0 || nsecs > 999_999_999 {
                return Err(LinuxError::EINVAL);
            }
            timeout_nanos = secs as u64 * NANOS_PER_SEC + nsecs as u64;
        }
        sys_poll_impl(fds, timeout_nanos, block)
    })
}

/// Poll: Monitors multiple file descriptors for event readiness, with millisecond timeout precision.
/// int poll(struct pollfd *fds, nfds_t nfds, int timeout);
///
/// # Parameters
/// - `fds`: A mutable slice of [`PollFd`] structures specifying file descriptors to monitor.
/// - `timeout`: Timeout in milliseconds. Negative value blocks indefinitely, zero returns immediately.
///
/// # Returns
/// - `Ok(i32)`: Number of ready file descriptors (≥0).
/// - `Err(LinuxError)`: Returns Linux errno on error.
///
/// # Safety
/// - The caller must ensure `fds` elements remain valid throughout the call.
/// - The memory layout of [`PollFd`] must match C's `struct pollfd`.
pub fn sys_poll_impl(fds: &mut [PollFd], timeout: u64, block: bool) -> LinuxResult<i32> {
    for fd in fds.iter_mut() {
        fd.revents = 0;
    }
    let now = axhal::time::monotonic_time_nanos();
    loop {
        let mut updated = false;
        for fd in fds.iter_mut() {
            if fd.fd < 0 {
                // ignore it
                continue;
            }
            let f = get_file_like(fd.fd);
            if let Err(_) = f {
                // invalid request: fd isn't open
                fd.revents = POLLNVAL;
                continue;
            }
            if let Some(pipe) = f.clone()?.into_any().downcast_ref::<Pipe>() {
                if pipe.write_end_close() {
                    fd.revents |= POLLHUP;
                    updated = true;
                }
            }
            match f?.poll() {
                Ok(state) => {
                    if state.readable && fd.events & POLLIN != 0 {
                        fd.revents |= POLLIN;
                        updated = true;
                    }
                    if state.writable && fd.events & POLLOUT != 0 {
                        fd.revents |= POLLOUT;
                        updated = true;
                    }
                }
                Err(_) => {
                    // poll error, for example, pipe closed
                    fd.revents = POLLERR;
                    updated = true;
                    continue;
                }
            }
        }
        if updated {
            // if any fd is updated, break
            break;
        }
        if !block {
            if timeout == 0 {
                // timeout == 0 means no wait
                break;
            } else {
                let elapsed = axhal::time::monotonic_time_nanos() - now;
                if elapsed >= timeout {
                    break;
                }
            }
        }
        yield_now();
    }
    let mut updated_count = 0;
    for fd in fds.iter() {
        if fd.revents != 0 {
            updated_count += 1;
        }
    }
    Ok(updated_count)
}

/// Represents a file descriptor being monitored, mirroring C's `struct pollfd`.
///
/// # Memory Layout
/// Uses `#[repr(C)]` to ensure compatibility with libc. All fields directly map to
/// their C counterparts.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollFd {
    /// File descriptor to monitor.
    ///
    /// - Negative values cause `events` to be ignored and `revents` to be zeroed.
    /// - To temporarily ignore a descriptor, set to negative (e.g., `!fd` via bitwise complement).
    pub fd: i32,

    /// Requested events (input parameter), constructed via bitwise OR of [`PollFlags`].
    pub events: i16,

    /// Returned events (output parameter), set by kernel. May contain [`PollFlags`]
    /// values even if not requested.
    pub revents: i16,
}

/// Nanosecond-precision timeout specification, equivalent to C's `struct timespec`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSpec {
    /// Seconds component.
    pub tv_sec: i64,

    /// Nanoseconds component (0 to 999,999,999 inclusive).
    pub tv_nsec: i64,
}

#[allow(dead_code)]
pub mod poll_flags {
    //! Input events for [`PollFd`].

    /// There is data to read.
    pub const POLLIN: i16 = 0x0001;
    /// There is urgent data to read.
    pub const POLLPRI: i16 = 0x0002;
    /// Writing is now possible, though a write larger than
    /// the available space in a socket or pipe will still block
    pub const POLLOUT: i16 = 0x0004;
    /// Error condition.
    pub const POLLERR: i16 = 0x0008;
    /// Hang up.
    pub const POLLHUP: i16 = 0x0010;
    /// Invalid request: fd not open.
    pub const POLLNVAL: i16 = 0x0020;
}
