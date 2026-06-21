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

    // Linux rejects entries=0 with EINVAL (our clamp would silently accept 0→1).
    if entries == 0 {
        return Err(AxError::InvalidInput);
    }

    // Validate params before creating the ring.
    const UNSUPPORTED_FLAGS: u32 = (1 << 0)  // IORING_SETUP_IOPOLL
                                 | (1 << 1)  // IORING_SETUP_SQPOLL
                                 | (1 << 2); // IORING_SETUP_SQ_AFF
    {
        let params = params_ptr.get_as_mut()?;

        // Reject unsupported flags. Use InvalidInput (EINVAL) rather than
        // Unsupported (ENOSYS): real apps probe for SQPOLL by checking EINVAL.
        if params.flags & UNSUPPORTED_FLAGS != 0 {
            warn!("io_uring_setup: unsupported flags {:#x}", params.flags);
            return Err(AxError::InvalidInput);
        }

        // Reject non-zero reserved fields (future-proofing).
        if params.resv.iter().any(|&r| r != 0) {
            warn!("io_uring_setup: non-zero reserved field");
            return Err(AxError::InvalidInput);
        }
    }

    let ring = IoRing::new(entries)?;
    let n = ring.entries;
    {
        let params = params_ptr.get_as_mut()?;
        params.sq_entries = n;
        // §4.2: "the CQ ring is twice the size of the SQ ring" — required by the
        // io_uring ABI so liburing can budget CQE slots correctly.
        params.cq_entries = n * 2;
        params.flags = 0;
        params.sq_thread_cpu = 0;
        params.sq_thread_idle = 0;
        // Advertise IORING_FEAT_NODROP: our CQ overflow is replay-buffered
        // (post_cqe never silently drops), so a userspace liburing that checks
        // this feature knows it can rely on no-drop semantics.
        //
        // IORING_FEAT_RSRC_TAGS (0x400): we honor per-buffer tags and post a
        // resource-released CQE (user_data = old tag) when a tagged registered
        // buffer is replaced via REGISTER_BUFFERS_UPDATE. Required for
        // test_register_buffers_update to run.
        // IORING_FEAT_FAST_POLL(1<<5): StarryOS I/O is inherently polling-based
        // (smoltcp + global_worker poll loop). There is no interrupt→wakeup
        // latency to "bypass", so FAST_POLL is the natural mode of operation.
        const IORING_FEAT_RSRC_TAGS: u32 = 1024;
        const IORING_FEAT_FAST_POLL: u32 = 1 << 5;
        params.features =
            linux_raw_sys::io_uring::IORING_FEAT_NODROP
            | IORING_FEAT_RSRC_TAGS
            | IORING_FEAT_FAST_POLL;
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
    // Reject unknown flags (we only support IORING_ENTER_GETEVENTS).
    const IORING_ENTER_GETEVENTS: u32 = 1;
    const IORING_ENTER_KNOWN_MASK: u32 = IORING_ENTER_GETEVENTS;
    if flags & !IORING_ENTER_KNOWN_MASK != 0 {
        return Err(AxError::InvalidInput);
    }

    let ring = match IoRing::from_fd(fd) {
        Ok(r) => r,
        Err(_) => {
            // Distinguish invalid fd (EBADF) from valid-but-not-ring fd (EOPNOTSUPP).
            if fd >= 0 && crate::file::get_file_like(fd).is_ok() {
                return Err(AxError::OperationNotSupported);
            }
            return Err(AxError::BadFileDescriptor);
        }
    };
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
    // Constants from io_uring_register_op (we don't link the enum to avoid the
    // sparse/optional variants tripping the build).
    const IORING_REGISTER_BUFFERS: u32 = 0;
    const IORING_UNREGISTER_BUFFERS: u32 = 1;
    const IORING_REGISTER_FILES: u32 = 2;
    const IORING_REGISTER_PROBE: u32 = 8;
    const IORING_REGISTER_BUFFERS2: u32 = 15;
    const IORING_REGISTER_BUFFERS_UPDATE: u32 = 16;
    const IORING_REGISTER_FILES_UPDATE: u32 = 18;
    match opcode {
        IORING_REGISTER_BUFFERS => {
            let ring = IoRing::from_fd(fd)?;
            ring.register_buffers(arg, nr_args)?;
            Ok(0)
        }
        IORING_UNREGISTER_BUFFERS => {
            // liburing's `unregister_buffers()` — required cleanup for
            // test_register_buffers (returns ENOSYS otherwise).
            let ring = IoRing::from_fd(fd)?;
            ring.unregister_buffers()?;
            Ok(0)
        }
        IORING_REGISTER_BUFFERS2 => {
            // Tagged registration; supports sparse tables
            // (IORING_RSRC_REGISTER_SPARSE) used by test_register_buffers_update.
            let ring = IoRing::from_fd(fd)?;
            ring.register_buffers2(arg, nr_args)?;
            Ok(0)
        }
        IORING_REGISTER_PROBE => {
            // Report supported opcodes so liburing consumers (tokio-rs
            // io-uring-test) run their opcode tests instead of skipping.
            let ring = IoRing::from_fd(fd)?;
            ring.register_probe(arg, nr_args)?;
            Ok(0)
        }
        IORING_REGISTER_BUFFERS_UPDATE => {
            // Replace a range of the registered-buffer table (rsrc_update2);
            // posts a tag CQE when a tagged slot is replaced.
            let ring = IoRing::from_fd(fd)?;
            ring.register_buffers_update(arg, nr_args)?;
            Ok(0)
        }
        IORING_REGISTER_FILES => {
            let ring = IoRing::from_fd(fd)?;
            ring.register_files(arg, nr_args)?;
            Ok(0)
        }
        IORING_REGISTER_FILES_UPDATE => {
            let ring = IoRing::from_fd(fd)?;
            ring.register_files_update(arg, nr_args)?;
            Ok(0)
        }
        _ => Err(AxError::Unsupported),
    }
}
