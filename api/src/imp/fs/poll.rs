use arceos_posix_api::{Pipe, get_file_like};
use axerrno::LinuxResult;
use axtask::yield_now;
use bitflags::bitflags;

bitflags! {
    pub struct PollFlags: i16 {
        /// There is data to read.
        const POLLIN = 0x0001;
        /// There is urgent data to read.
        const POLLPRI = 0x0002;
        /// Writing is now possible, though a writing larger than
        /// the available space in a socket or pipe will still block
        const POLLOUT = 0x0004;
        /// Error condition.
        const POLLERR = 0x0008;
        /// Hang up.
        const POLLHUP = 0x0010;
        /// Invalid request: fd not open.
        const POLLNVAL = 0x0020;
    }
}

pub struct PollEntry {
    /// File descriptor to monitor.
    /// Negative values cause `events` to be ignored and `results` to be zeroed.
    pub fd: i32,
    /// Requested events (input parameter), constructed via bitwise OR of [`PollFlags`].
    pub events: PollFlags,
    /// Returned events (output parameter), set by kernel. May contain [`PollFlags`]
    /// values even if not requested.
    pub results: PollFlags,
}

/// Poll: Monitors multiple file descriptors for event readiness, with millisecond timeout precision.
///
/// # Parameters
/// - `fds`: A mutable slice of [`PollEntry`] structures specifying file descriptors to monitor.
/// - `timeout`: Timeout in milliseconds. Negative value blocks indefinitely, zero returns immediately.
///
/// # Returns
/// - `Ok(isize)`: Number of ready file descriptors (≥0).
/// - `Err(LinuxError)`: Returns Linux errno on error.
///
/// # Safety
/// - The caller must ensure `fds` elements remain valid throughout the call.
pub fn sys_poll_impl(fds: &mut [PollEntry], timeout: u64, block: bool) -> LinuxResult<isize> {
    for entry in fds.iter_mut() {
        entry.results = PollFlags::empty();
    }
    let now = axhal::time::monotonic_time_nanos();
    loop {
        let mut updated = false;
        for entry in fds.iter_mut() {
            if entry.fd < 0 {
                // ignore it
                continue;
            }
            let f = get_file_like(entry.fd);
            if let Err(_) = f {
                // invalid request: fd isn't open
                entry.results = PollFlags::POLLNVAL;
                continue;
            }
            if let Some(pipe) = f.clone()?.into_any().downcast_ref::<Pipe>() {
                if pipe.write_end_close() {
                    entry.results |= PollFlags::POLLHUP;
                    updated = true;
                }
            }
            match f?.poll() {
                Ok(state) => {
                    if state.readable && entry.events.contains(PollFlags::POLLIN) {
                        entry.results |= PollFlags::POLLIN;
                        updated = true;
                    }
                    if state.writable && entry.events.contains(PollFlags::POLLOUT) {
                        entry.results |= PollFlags::POLLOUT;
                        updated = true;
                    }
                }
                Err(_) => {
                    // poll error, for example, pipe closed
                    entry.results = PollFlags::POLLERR;
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
        if !fd.results.is_empty() {
            updated_count += 1;
        }
    }
    Ok(updated_count)
}
