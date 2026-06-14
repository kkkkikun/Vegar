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

/// `io_uring_enter(fd, to_submit, min_complete, flags, arg, sz)` -> #submitted.
///
/// Runs in the submitter's task context, so the SQ ring is read directly from
/// the shared pages. Each consumed SQE is turned into an `Op` and handed to the
/// worker; the SQ head is advanced with Release so the user sees the slots freed.
pub fn sys_io_uring_enter(
    fd: i32,
    to_submit: u32,
    _min_complete: u32,
    _flags: u32,
    _arg: usize,
    _sz: usize,
) -> AxResult<isize> {
    let ring = IoRing::from_fd(fd)?;
    // M0: drain all available SQEs (to_submit is treated as a hint; GETEVENTS
    // waiting is not yet implemented — the user reaps CQEs from the shared ring).
    let n = ring.submit_pending();

    debug!("sys_io_uring_enter <= fd: {fd}, to_submit: {to_submit}, submitted: {n}");

    // 🚀 核心新增：在单核（smp=1）调度下，主动让出一次 CPU 
    // 确保刚被 submit_pending 推入全局队列的子任务线程有充足的机会被调度器切上去跑！
    if n > 0 {
        axtask::yield_now();
    }

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
