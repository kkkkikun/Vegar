use crate::imp::fs::poll::{PollEntry, PollFlags, sys_poll_impl};
use crate::ptr::{PtrWrapper, UserInOutPtr, UserInPtr};
use alloc::vec::Vec;
use axerrno::{LinuxError, LinuxResult};
use axhal::time::{MICROS_PER_SEC, NANOS_PER_MICROS, NANOS_PER_SEC};
use bit_field::BitArray;
use core::cmp::{max, min};
use core::ffi::{c_int, c_ulong};
use linux_raw_sys::general::{timespec, timeval};
use syscall_trace::syscall_trace;

/// Represents a file descriptor being monitored, mirroring C's `struct pollfd`.
///
/// # Memory Layout
/// Uses `#[repr(C)]` to ensure compatibility with libc. All fields directly map to
/// their C counterparts.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserPollFd {
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

impl Into<PollEntry> for UserPollFd {
    fn into(self) -> PollEntry {
        PollEntry {
            fd: self.fd,
            events: PollFlags::from_bits_truncate(self.events),
            results: PollFlags::from_bits_truncate(self.revents),
        }
    }
}

impl From<PollEntry> for UserPollFd {
    fn from(entry: PollEntry) -> Self {
        UserPollFd {
            fd: entry.fd,
            events: entry.events.bits(),
            revents: entry.results.bits(),
        }
    }
}

#[syscall_trace]
pub fn sys_poll(
    fds: UserInOutPtr<UserPollFd>,
    n_fds: c_ulong,
    timeout: c_int,
) -> LinuxResult<isize> {
    // get params
    let fds = fds.get_as_array(n_fds as _)?;
    let fds_slice: &mut [UserPollFd] = unsafe { core::slice::from_raw_parts_mut(fds, n_fds as _) };
    let mut entries: Vec<PollEntry> = fds_slice.iter().map(|&fd| fd.into()).collect();

    // preform syscall
    let block = timeout < 0;
    let timeout = if timeout < 0 {
        0
    } else {
        timeout as u64 * NANOS_PER_MICROS
    };
    let result = sys_poll_impl(&mut entries, timeout, block)?;

    // copy results back
    let entries: Vec<UserPollFd> = entries.into_iter().map(|fd| fd.into()).collect();
    unsafe { fds.copy_from_nonoverlapping(entries.as_ptr(), n_fds as usize) };
    Ok(result)
}

#[syscall_trace]
pub fn sys_ppoll(
    fds: UserInOutPtr<UserPollFd>,
    n_fds: c_ulong,
    timeout: UserInPtr<timespec>,
    _sigmask: UserInOutPtr<c_ulong>,
) -> LinuxResult<isize> {
    // get params
    let fds = fds.get_as_array(n_fds as _)?;
    let fds_slice: &mut [UserPollFd] = unsafe { core::slice::from_raw_parts_mut(fds, n_fds as _) };
    let mut entries: Vec<PollEntry> = fds_slice.iter().map(|&fd| fd.into()).collect();
    let mut block = false;
    let mut timeout_nanos: u64 = 0;
    if timeout.is_null() {
        block = true;
    } else {
        let t = timeout.get_as_ref()?;
        if t.tv_sec < 0 || t.tv_nsec < 0 || t.tv_nsec > 999_999_999 {
            return Err(LinuxError::EINVAL);
        }
        timeout_nanos = (t.tv_sec as u64 * NANOS_PER_SEC) + t.tv_nsec as u64;
    }

    // preform syscall
    // TODO: implement signal mask
    // if _sigmask should be modified? use UserInPtr?
    // pthread_sigmask(SIG_SETMASK, &sigmask, &origmask);
    let result = sys_poll_impl(&mut entries, timeout_nanos, block)?;
    // pthread_sigmask(SIG_SETMASK, &origmask, NULL);

    // copy results back
    let entries: Vec<UserPollFd> = entries.into_iter().map(|fd| fd.into()).collect();
    unsafe { fds.copy_from_nonoverlapping(entries.as_ptr(), n_fds as usize) };
    Ok(result)
}

const FD_SETSIZE: usize = 1024;
const FD_SET_LEN: usize = FD_SETSIZE / 8 / size_of::<usize>();

#[syscall_trace]
pub fn sys_select(
    n_fds: c_int,
    read_fds: UserInOutPtr<c_ulong>,
    write_fds: UserInOutPtr<c_ulong>,
    except_fds: UserInOutPtr<c_ulong>,
    timeout: UserInPtr<timeval>,
) -> LinuxResult<isize> {
    let mut block = false;
    let mut timeout_nanos: u64 = 0;
    if timeout.is_null() {
        block = true;
    } else {
        let t = timeout.get_as_ref()?;
        if t.tv_sec < 0 || t.tv_usec < 0 {
            return Err(LinuxError::EINVAL);
        }
        let timeout_micros = (t.tv_sec as u64 * MICROS_PER_SEC) + t.tv_usec as u64;
        timeout_nanos = timeout_micros * NANOS_PER_MICROS;
    }
    sys_select_(
        n_fds,
        read_fds.clone(),
        write_fds.clone(),
        except_fds.clone(),
        timeout_nanos,
        block,
    )
}

#[syscall_trace]
pub fn sys_pselect(
    n_fds: c_int,
    read_fds: UserInOutPtr<c_ulong>,
    write_fds: UserInOutPtr<c_ulong>,
    except_fds: UserInOutPtr<c_ulong>,
    timeout: UserInPtr<timespec>,
    _sigmask: UserInOutPtr<c_ulong>,
) -> LinuxResult<isize> {
    let mut block = false;
    let mut timeout_nanos: u64 = 0;
    if timeout.is_null() {
        block = true;
    } else {
        let t = timeout.get_as_ref()?;
        if t.tv_sec < 0 || t.tv_nsec < 0 || t.tv_nsec > 999_999_999 {
            return Err(LinuxError::EINVAL);
        }
        timeout_nanos = (t.tv_sec as u64 * NANOS_PER_SEC) + t.tv_nsec as u64;
    }
    // TODO: implement signal mask
    sys_select_(
        n_fds,
        read_fds.clone(),
        write_fds.clone(),
        except_fds.clone(),
        timeout_nanos,
        block,
    )
    // TODO: restore signal mask
}

fn sys_select_(
    n_fds: c_int,
    read_fds: UserInOutPtr<c_ulong>,
    write_fds: UserInOutPtr<c_ulong>,
    except_fds: UserInOutPtr<c_ulong>,
    timeout_nanos: u64,
    block: bool,
) -> LinuxResult<isize> {
    let n_fds = min(n_fds as usize, FD_SETSIZE);

    let mut empty_set_read: [usize; FD_SET_LEN] = [0usize; FD_SET_LEN];
    let mut empty_set_write: [usize; FD_SET_LEN] = [0usize; FD_SET_LEN];
    let mut empty_set_except: [usize; FD_SET_LEN] = [0usize; FD_SET_LEN];

    let read_fds_slice = if read_fds.is_null() {
        &mut empty_set_read
    } else {
        let read_fds = read_fds.get_as_array(FD_SET_LEN)? as *mut usize;
        unsafe { core::slice::from_raw_parts_mut(read_fds, FD_SET_LEN) }
    };
    let write_fds_slice = if write_fds.is_null() {
        &mut empty_set_write
    } else {
        let write_fds = write_fds.get_as_array(FD_SET_LEN)? as *mut usize;
        unsafe { core::slice::from_raw_parts_mut(write_fds, FD_SET_LEN) }
    };
    let except_fds_slice = if except_fds.is_null() {
        &mut empty_set_except
    } else {
        let except_fds = except_fds.get_as_array(FD_SET_LEN)? as *mut usize;
        unsafe { core::slice::from_raw_parts_mut(except_fds, FD_SET_LEN) }
    };

    let mut entries: Vec<PollEntry> = (0..n_fds)
        .map(|i| {
            let mut events = PollFlags::empty();
            if read_fds_slice.get_bit(i) {
                events |= PollFlags::POLLIN;
            }
            if write_fds_slice.get_bit(i) {
                events |= PollFlags::POLLOUT;
            }
            if except_fds_slice.get_bit(i) {
                events |= PollFlags::POLLPRI;
            }
            PollEntry {
                fd: i as i32,
                events,
                results: PollFlags::empty(),
            }
        })
        .filter(|entry| !entry.events.is_empty())
        .collect();

    // TODO: implement signal mask
    sys_poll_impl(&mut entries, timeout_nanos, block)?;
    // pthread_sigmask(SIG_SETMASK, &origmask, NULL);

    // copy results back
    let mut count = 0;
    for i in 0..n_fds {
        let entry = &entries[i];
        // if read_fds_slice.get_bit(i) != 0 {
        let mut updated = false;
        if entry.results.contains(PollFlags::POLLIN) {
            read_fds_slice.set_bit(i, true);
            updated = true;
        } else {
            read_fds_slice.set_bit(i, false);
        }
        // }
        // if write_fds_slice.get_bit(i) != 0 {
        if entry.results.contains(PollFlags::POLLOUT) {
            write_fds_slice.set_bit(i, true);
            updated = true;
        } else {
            write_fds_slice.set_bit(i, false);
        }
        // }
        // if except_fds_slice.get_bit(i) != 0 {
        if entry.results.contains(PollFlags::POLLPRI) {
            except_fds_slice.set_bit(i, true);
            updated = true;
        } else {
            except_fds_slice.set_bit(i, false);
        }
        // }
        if updated {
            count += 1;
        }
    }

    Ok(count)
}
