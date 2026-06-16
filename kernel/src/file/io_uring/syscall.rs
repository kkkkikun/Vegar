//! `io_uring_setup` / `io_uring_enter` / `io_uring_register` syscall handlers.
//!
//! M0: `setup` + `enter` (NOP only); `register` is a stub.

use axerrno::{AxError, AxResult};
use linux_raw_sys::io_uring::io_uring_params;

use crate::file::FileLike;
use crate::mm::UserPtr;

use super::IoRing;

/// `io_uring_setup(entries, params)` -> ring fd.
pub fn sys_io_uring_setup(entries: u32, params_ptr: UserPtr<io_uring_params>) -> AxResult<isize> {
    debug!("sys_io_uring_setup <= entries: {entries}");
    let ring = IoRing::new(entries)?;
    let n = ring.entries;
    {
        let params = params_ptr.get_as_mut()?;
        params.sq_entries = n;
        params.cq_entries = n;
        params.flags = 0;
        params.sq_thread_cpu = 0;
        params.sq_thread_idle = 0;
        params.features = 0;
        params.wq_fd = 0;
        IoRing::fill_offsets(&mut params.sq_off, &mut params.cq_off);
    }
    let fd = ring.add_to_fd_table(false)?;
    debug!("sys_io_uring_setup => fd: {fd}, entries: {n}");
    Ok(fd as isize)
}

/// `IORING_ENTER_GETEVENTS`: block the caller until at least `min_complete`
/// CQEs are available (ring + overflow replay buffer).
const IORING_ENTER_GETEVENTS: u32 = 1;

/// `io_uring_enter(fd, to_submit, min_complete, flags, arg, sz)` -> #submitted.
///
/// Runs in the submitter's task context, so the SQ ring is read directly from
/// the shared pages. Each consumed SQE is turned into an `Op` and handed to the
/// worker; the SQ head is advanced with Release so the user sees the slots freed.
///
/// With `IORING_ENTER_GETEVENTS`, the caller blocks until `min_complete` CQEs are
/// available — the worker posts them and wakes this task via the ring's wait
/// queue (this is the path liburing's `io_uring_submit_and_wait` /
/// `io_uring_wait_cqe` rely on).
pub fn sys_io_uring_enter(
    fd: i32,
    to_submit: u32,
    min_complete: u32,
    flags: u32,
    _arg: usize,
    _sz: usize,
) -> AxResult<isize> {
    let ring = IoRing::from_fd(fd)?;
    // Drain all available SQEs (to_submit is treated as a hint). fd and user
    // buffer resolution happen here, in the submitter's context. ACCEPT requests
    // are returned separately (they must run in this context to install fds).
    let (n, accepts) = ring.submit_pending();
    // Run ACCEPTs here — after the other ops are queued for the worker, so the
    // worker can drive RECV/SEND on existing connections while we park waiting
    // for a new one. Each ACCEPT posts its own CQE (res = new fd).
    for (ud, listener) in accepts {
        ring.complete_accept(ud, &listener);
    }

    if flags & IORING_ENTER_GETEVENTS != 0 {
        // Block this task until min_complete CQEs land. The worker (a separate
        // task) posts them and wakes us; on SMP=1 parking here is what lets the
        // worker get scheduled and run.
        ring.wait_for_completions(min_complete);
    } else if n > 0 {
        // No GETEVENTS: the user reaps the shared ring themselves. On SMP=1
        // yield once so the worker gets a turn before we return.
        axtask::yield_now();
    }

    debug!(
        "sys_io_uring_enter <= fd: {fd}, to_submit: {to_submit}, min_complete: {min_complete}, \
         flags: {flags:#x}, submitted: {n}"
    );
    Ok(n as isize)
}

/// `io_uring_register(fd, opcode, arg, nr_args)`.
pub fn sys_io_uring_register(fd: i32, opcode: u32, arg: usize, nr_args: u32) -> AxResult<isize> {
    debug!("sys_io_uring_register <= opcode: {opcode}, nr_args: {nr_args}");
    // IORING_REGISTER_BUFFERS = 0 in the io_uring_register_op enum.
    if opcode == 0 {
        let ring = IoRing::from_fd(fd)?;
        ring.register_buffers(arg, nr_args)?;
        Ok(0)
    } else {
        Err(AxError::Unsupported)
    }
}
