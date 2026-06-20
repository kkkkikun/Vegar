//! io_uring: asynchronous syscall interface (Phase 2).
//!
//! M0 scope: ring skeleton + `IORING_OP_NOP` round-trip.
//!
//! Design (see docs/io-uring-feasibility.md + the Phase 2 plan):
//! - The SQ/CQ rings and the SQE array live in three [`SharedPages`] regions that
//!   the user `mmap`s and the kernel accesses via `phys_to_virt` — same physical
//!   pages, zero copy.
//! - `io_uring_enter` runs in the submitter's task context, so it resolves the
//!   fd table and reads the user-filled SQEs directly from the shared ring.
//! - A dedicated worker task drives the operations and posts CQEs. For M0 it
//!   only handles NOP.

pub mod syscall;

use alloc::{
    borrow::Cow, boxed::Box, collections::VecDeque, format, sync::Arc, task::Wake, vec::Vec,
};
use core::{
    future::{poll_fn, Future},
    pin::Pin,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
    time::Duration,
};

use axerrno::{AxError, AxResult};
use axhal::mem::phys_to_virt;
use axio::{IoBuf, IoBufMut, Read, Write};
use axnet::SocketOps;
use axpoll::{IoEvents, Pollable};
use axsync::Mutex;
use hashbrown::HashMap;
use axtask::{
    WaitQueue, current,
    future::{block_on, sleep},
    spawn_with_name, yield_now,
};
use lazy_static::lazy_static;
use linux_raw_sys::io_uring as iouring;
use memory_addr::{PhysAddr, VirtAddr};

use crate::{file::{get_file_like, net::Socket}, mm::SharedPages, task::AsThread};

use super::FileLike;

/// Size of one `io_uring_sqe` (matches the Linux ABI).
const SQE_SIZE: usize = 64;
/// Size of one `io_uring_cqe` (we treat it as the 16-byte `user_data/res/flags`).
const CQE_SIZE: usize = 16;

// ---- Self-consistent ring layout (offsets reported honestly via params) ----
const SQ_OFF_HEAD: usize = 0x00;
const SQ_OFF_TAIL: usize = 0x04;
const SQ_OFF_MASK: usize = 0x08;
const SQ_OFF_ENTRIES: usize = 0x0c;
const SQ_OFF_FLAGS: usize = 0x10;
const SQ_OFF_DROPPED: usize = 0x14;
const SQ_OFF_ARRAY: usize = 0x18;

const CQ_OFF_HEAD: usize = 0x00;
const CQ_OFF_TAIL: usize = 0x04;
const CQ_OFF_MASK: usize = 0x08;
const CQ_OFF_ENTRIES: usize = 0x0c;
const CQ_OFF_OVERFLOW: usize = 0x10;
const CQ_OFF_FLAGS: usize = 0x14;
const CQ_OFF_CQES: usize = 0x18;

/// IORING_OP_* values we support.
const OP_NOP: u8 = iouring::io_uring_op::IORING_OP_NOP as u8;
const OP_READV: u8 = iouring::io_uring_op::IORING_OP_READV as u8;
const OP_WRITEV: u8 = iouring::io_uring_op::IORING_OP_WRITEV as u8;
const OP_READ: u8 = iouring::io_uring_op::IORING_OP_READ as u8;
const OP_WRITE: u8 = iouring::io_uring_op::IORING_OP_WRITE as u8;
const OP_READ_FIXED: u8 = iouring::io_uring_op::IORING_OP_READ_FIXED as u8;
const OP_WRITE_FIXED: u8 = iouring::io_uring_op::IORING_OP_WRITE_FIXED as u8;
const OP_POLL_ADD: u8 = iouring::io_uring_op::IORING_OP_POLL_ADD as u8;
const OP_SEND: u8 = iouring::io_uring_op::IORING_OP_SEND as u8;
const OP_RECV: u8 = iouring::io_uring_op::IORING_OP_RECV as u8;
const OP_ACCEPT: u8 = iouring::io_uring_op::IORING_OP_ACCEPT as u8;
const OP_TIMEOUT: u8 = iouring::io_uring_op::IORING_OP_TIMEOUT as u8;
/// Internal marker (not a real io_uring opcode): a READ_FIXED/WRITE_FIXED whose
/// registered-buffer index was unfilled/sparse. Completes with -EFAULT.
const OP_FIXED_FAULT: u8 = 0xfd;
/// CQE result for a fired relative timer (matches Linux io_uring semantics).
const ENETIME: i32 = -62;

/// `MSG_DONTWAIT` — recv/send without blocking (skip the readiness wait).
const MSG_DONTWAIT: u32 = 0x40;

/// A user buffer resolved to its physical pages while the submitter's address
/// space was active (at `enter` time). The worker accesses the bytes through
/// `phys_to_virt` — no address-space switch, no per-op copy.
pub(crate) struct PinnedUserBuf {
    phys: Vec<PhysAddr>, // one entry per page the buffer spans
    page_off: usize,     // offset of byte 0 within the first page
    len: usize,          // logical length
    pos: usize,          // current read/write cursor
}

impl PinnedUserBuf {
    /// Resolve `[start, start+len)` from the CURRENT task's address space (must
    /// be the submitter's). Walks the page table page-by-page.
    pub(crate) fn resolve(start: usize, len: usize) -> AxResult<Self> {
        if len == 0 {
            return Ok(Self {
                phys: Vec::new(),
                page_off: 0,
                len: 0,
                pos: 0,
            });
        }
        let curr = current();
        let aspace = curr.as_thread().proc_data.aspace.lock();
        let pt = aspace.page_table();
        let first_page = start & !0xfff;
        let last_page = (start + len - 1) & !0xfff;
        let mut phys = Vec::new();
        let mut va = first_page;
        while va <= last_page {
            let (paddr, _flags, _size) = pt
                .query(VirtAddr::from_usize(va))
                .map_err(|_| AxError::BadAddress)?;
            phys.push(paddr);
            va += 4096;
        }
        Ok(Self {
            phys,
            page_off: start - first_page,
            len,
            pos: 0,
        })
    }

    /// Kernel virtual address of the byte at logical offset `off`.
    fn byte_ptr(&self, off: usize) -> *mut u8 {
        let abs = self.page_off + off;
        let page_idx = abs / 4096;
        let byte_in_page = abs % 4096;
        (phys_to_virt(self.phys[page_idx]).as_usize() + byte_in_page) as *mut u8
    }

    /// Copy `src` into the phys buffer at the cursor; returns bytes copied.
    fn copy_in(&mut self, src: &[u8]) -> usize {
        let n = src.len().min(self.len.saturating_sub(self.pos));
        let mut done = 0;
        while done < n {
            let abs = self.page_off + self.pos + done;
            let chunk = (n - done).min(4096 - (abs % 4096));
            unsafe {
                core::ptr::copy_nonoverlapping(src.as_ptr().add(done), self.byte_ptr(self.pos + done), chunk);
            }
            done += chunk;
        }
        self.pos += done;
        done
    }

    /// Copy from the phys buffer at the cursor into `dst`; returns bytes copied.
    fn copy_out(&mut self, dst: &mut [u8]) -> usize {
        let n = dst.len().min(self.len.saturating_sub(self.pos));
        let mut done = 0;
        while done < n {
            let abs = self.page_off + self.pos + done;
            let chunk = (n - done).min(4096 - (abs % 4096));
            unsafe {
                core::ptr::copy_nonoverlapping(self.byte_ptr(self.pos + done), dst.as_mut_ptr().add(done), chunk);
            }
            done += chunk;
        }
        self.pos += done;
        done
    }
}

// READ direction: be a WriteBuf (receive bytes from the file).
impl Write for PinnedUserBuf {
    fn write(&mut self, src: &[u8]) -> axio::Result<usize> {
        Ok(self.copy_in(src))
    }
    fn flush(&mut self) -> axio::Result<()> {
        Ok(())
    }
}
impl IoBufMut for PinnedUserBuf {
    fn remaining_mut(&self) -> usize {
        self.len.saturating_sub(self.pos)
    }
}

// WRITE direction: be a ReadBuf (feed bytes to the file).
impl Read for PinnedUserBuf {
    fn read(&mut self, dst: &mut [u8]) -> axio::Result<usize> {
        Ok(self.copy_out(dst))
    }
}
impl IoBuf for PinnedUserBuf {
    fn remaining(&self) -> usize {
        self.len.saturating_sub(self.pos)
    }
}

// Manual Clone: clone the physical-page list, reset the read/write cursor to
// zero so each fixed-buffer op starts from the beginning of the buffer.
impl Clone for PinnedUserBuf {
    fn clone(&self) -> Self {
        Self {
            phys: self.phys.clone(),
            page_off: self.page_off,
            len: self.len,
            pos: 0,
        }
    }
}

/// One in-flight operation handed from the submitter to the global worker.
struct Op {
    user_data: u64,
    opcode: u8,
    /// Resolved in the submitter's context (worker cannot resolve fds itself:
    /// FD_TABLE is scope-local and the worker has no Thread extension).
    file: Option<Arc<dyn FileLike>>,
    /// Resolved user buffer(s). A `Vec` so READV/WRITEV can carry multiple
    /// iovecs; single-buffer ops (READ/WRITE/FIXED) wrap one entry. `None` for
    /// NOP / ACCEPT.
    buf: Option<Vec<PinnedUserBuf>>,
    /// Requested poll events for POLL_ADD (0 otherwise).
    poll_events: u32,
    /// `msg_flags` for SEND/RECV (e.g. MSG_DONTWAIT). 0 otherwise.
    msg_flags: u32,
    /// File offset for READ/WRITE (sqe.off). u64::MAX means "use current position".
    offset: u64,
    /// For OP_TIMEOUT: the relative duration parsed from the sqe timespec
    /// (`Duration::ZERO` for all other opcodes).
    duration: Duration,
    /// This op's CQ ring view + the pages that back it (keepalive: the pages
    /// stay pinned until the CQE is posted, even if the fd is already closed).
    cq: CqRing,
    cq_pages: Arc<SharedPages>,
    /// Per-ring completion wait queue + overflow list, so the worker can post a
    /// CQE (or buffer it) and wake any submitter parked in GETEVENTS.
    cq_wq: Arc<WaitQueue>,
    overflow: Arc<Mutex<VecDeque<(u64, i32)>>>,
}

/// A pollable in-flight operation driven by the multiplexing worker. This is a
/// hand-rolled state machine (`impl Future`, NOT `async fn`) so it is `Unpin` —
/// the worker stores many of these in a `VecDeque` and re-polls them across
/// iterations without `Pin<Box>`. Each field is owned, so the future registers
/// the worker's waker (via `Pollable::register`) and completes when its
/// resource becomes ready, letting the worker multiplex N pending ops on one
/// task — the single-core io_uring value (event-driven concurrency, O(1)
/// memory).
struct OpFuture {
    user_data: u64,
    opcode: u8,
    file: Option<Arc<dyn FileLike>>,
    buf: Option<Vec<PinnedUserBuf>>,
    poll_events: u32,
    msg_flags: u32,
    offset: u64,
    duration: Duration,
    /// For OP_TIMEOUT: the backing `sleep` future, lazily created on first poll
    /// (in the worker context — `wall_time()` is read there). `Pin<Box<…>>` is
    /// `Unpin`, so `OpFuture` stays storable in a `VecDeque` without re-pinning.
    timer: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    cq: CqRing,
    cq_pages: Arc<SharedPages>,
    cq_wq: Arc<WaitQueue>,
    overflow: Arc<Mutex<VecDeque<(u64, i32)>>>,
    /// Cumulative bytes across re-polls for READ/WRITE (resumes after EAGAIN).
    total: i32,
    /// Which iovec to resume from on the next poll (after EAGAIN mid-readv).
    buf_offset: usize,
}

impl From<Op> for OpFuture {
    fn from(op: Op) -> Self {
        let Op {
            user_data,
            opcode,
            file,
            buf,
            poll_events,
            msg_flags,
            offset,
            duration,
            cq,
            cq_pages,
            cq_wq,
            overflow,
        } = op;
        Self {
            user_data,
            opcode,
            file,
            buf,
            poll_events,
            msg_flags,
            offset,
            duration,
            timer: None,
            cq,
            cq_pages,
            cq_wq,
            overflow,
            total: 0,
            buf_offset: 0,
        }
    }
}

impl OpFuture {
    /// Completion metadata: what the worker needs to post the CQE once this
    /// future is `Ready`. `cq_pages` is included so the CQ ring pages stay
    /// pinned until `post_cqe` finishes (the IoRing fd may already be closed).
    fn meta(
        &self,
    ) -> (
        CqRing,
        Arc<WaitQueue>,
        Arc<Mutex<VecDeque<(u64, i32)>>>,
        Arc<SharedPages>,
        u64,
    ) {
        (
            self.cq,
            self.cq_wq.clone(),
            self.overflow.clone(),
            self.cq_pages.clone(),
            self.user_data,
        )
    }

    /// Readiness-wait + nonblocking read/write with **cross-poll resume**: if
    /// EAGAIN occurs mid-way through a multi-iovec op (e.g. READV where the
    /// second iovec's recv blocks after the first succeeded), the op saves its
    /// progress (`self.total`, `self.buf_offset`) and re-arms as Pending. On
    /// the next poll (woken when more data arrives), it resumes from the
    /// interrupted iovec — so a readv eventually completes with ALL bytes.
    ///
    /// This drives I/O **entirely through the `FileLike` async contract**
    /// (`poll_read`/`poll_write` for the park path, `try_read`/`try_write` for
    /// the `MSG_DONTWAIT` path). No `set_nonblocking` toggle lives here — each
    /// driver owns its readiness strategy. That is the architectural payoff of
    /// the `AsyncFileLike` migration: the worker talks the same async trait to
    /// files, pipes, and sockets.
    fn poll_rw(&mut self, cx: &mut Context<'_>, is_read: bool) -> Poll<i32> {
        let file = match self.file.as_ref() {
            Some(f) => f.clone(),
            None => return Poll::Ready(-9),
        };
        let bufs = match self.buf.as_mut() {
            Some(b) => b,
            None => return Poll::Ready(-9),
        };
        let dontwait = self.msg_flags & MSG_DONTWAIT != 0;
        let events = if is_read { IoEvents::IN } else { IoEvents::OUT };

        let mut hit_block = false;
        // Resume from the iovec that last got EAGAIN; earlier iovecs are full
        // (remaining_mut == 0) and get skipped.
        let mut i = self.buf_offset;
        while i < bufs.len() {
            let buf = &mut bufs[i];
            if buf.remaining_mut() == 0 {
                i += 1;
                self.buf_offset = i;
                continue;
            }

            if dontwait {
                // Single non-blocking shot: never park. Report -EAGAIN if the
                // whole op transferred nothing. (Positioned I/O only happens on
                // regular files, which never block, so the read_at fallback is
                // always ready.)
                let r = if is_read {
                    if self.offset != u64::MAX {
                        file.read_at(buf, self.offset).or_else(|_| file.try_read(buf))
                    } else {
                        file.try_read(buf)
                    }
                } else if self.offset != u64::MAX {
                    file.write_at(buf, self.offset).or_else(|_| file.try_write(buf))
                } else {
                    file.try_write(buf)
                };
                if self.offset != u64::MAX {
                    if let Ok(n) = &r {
                        self.offset += *n as u64;
                    }
                }
                match r {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        self.total = self.total.wrapping_add(n as i32);
                        self.buf_offset = i + 1;
                        i += 1;
                    }
                    Err(_) => {
                        hit_block = true;
                        self.buf_offset = i; // resume here next poll
                        break;
                    }
                }
            } else {
                // Async park path: on WouldBlock the driver registers the worker
                // waker inside poll_read/poll_write and we return Pending.
                let pr = if is_read {
                    if self.offset != u64::MAX {
                        file.poll_read_at(cx, buf, self.offset)
                    } else {
                        file.poll_read(cx, buf)
                    }
                } else if self.offset != u64::MAX {
                    file.poll_write_at(cx, buf, self.offset)
                } else {
                    file.poll_write(cx, buf)
                };
                match pr {
                    Poll::Ready(Ok(0)) => break, // EOF
                    Poll::Ready(Ok(n)) => {
                        if self.offset != u64::MAX {
                            self.offset += n as u64;
                        }
                        self.total = self.total.wrapping_add(n as i32);
                        self.buf_offset = i + 1;
                        i += 1;
                    }
                    Poll::Ready(Err(_)) => {
                        hit_block = true;
                        self.buf_offset = i; // resume here next poll
                        break;
                    }
                    Poll::Pending => {
                        // Re-armed by the driver. Resume from this iovec next poll.
                        self.buf_offset = i;
                        return Poll::Pending;
                    }
                }
            }
        }

        if hit_block {
            if self.total == 0 && dontwait {
                Poll::Ready(-11) // EAGAIN: nothing transferred, caller asked DONTWAIT
            } else {
                // More data may arrive (WouldBlock) or a transient error; re-arm.
                file.register(cx, events);
                Poll::Pending
            }
        } else {
            // All iovecs processed (or EOF). Return cumulative total.
            Poll::Ready(self.total)
        }
    }
}

impl Future for OpFuture {
    type Output = i32;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<i32> {
        match self.opcode {
            OP_NOP => Poll::Ready(0),
            OP_POLL_ADD => match self.file.as_ref() {
                Some(file) => {
                    let events = IoEvents::from_bits_truncate(self.poll_events);
                    let cur = file.poll();
                    if cur.intersects(events) {
                        Poll::Ready(cur.bits() as i32)
                    } else {
                        file.register(cx, events);
                        Poll::Pending
                    }
                }
                None => Poll::Ready(-9),
            },
            OP_READ | OP_RECV => self.poll_rw(cx, true),
            OP_WRITE | OP_SEND => self.poll_rw(cx, false),
            OP_FIXED_FAULT => Poll::Ready(-14), // EFAULT: fixed buffer index unfilled
            OP_TIMEOUT => {
                // Lazily arm the backing sleep future on first poll (worker
                // context), then drive it. When it fires, complete with -ETIME.
                // The TimerFuture registered the worker's waker with the kernel
                // timer wheel, so the worker is re-poled when the deadline hits.
                if self.timer.is_none() {
                    self.timer = Some(Box::pin(sleep(self.duration)));
                }
                match self.timer.as_mut().unwrap().as_mut().poll(cx) {
                    Poll::Ready(()) => Poll::Ready(ENETIME),
                    Poll::Pending => Poll::Pending,
                }
            }
            _ => Poll::Ready(-38),
        }
    }
}

/// Race-free op queue: the submitter pushes, the multiplexing worker pops.
struct OpQueue {
    inner: Mutex<VecDeque<Op>>,
    /// The worker registers its waker here before parking, so a `push` wakes it
    /// (alongside any readiness the in-flight ops registered on their own).
    worker_waker: Mutex<Option<Waker>>,
}

impl OpQueue {
    const fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            worker_waker: Mutex::new(None),
        }
    }

    fn push(&self, op: Op) {
        self.inner.lock().push_back(op);
        if let Some(w) = self.worker_waker.lock().take() {
            w.wake();
        }
    }

    /// Non-blocking pop; the worker calls this each pass of its event loop.
    fn try_pop(&self) -> Option<Op> {
        self.inner.lock().pop_front()
    }

    /// Register the worker's waker so the next `push` wakes it. Called right
    /// before the worker parks.
    fn register_worker(&self, cx: &mut Context<'_>) {
        *self.worker_waker.lock() = Some(cx.waker().clone());
    }
}

/// Kernel-side view over the shared SQ ring + SQE array.
#[derive(Clone, Copy)]
struct SqRing {
    head: *const AtomicU32,
    tail: *const AtomicU32,
    array: *mut AtomicU32, // sq.array[entries]
    sqes: *mut iouring::io_uring_sqe,
    mask: u32,
    entries: u32,
}

/// Kernel-side view over the shared CQ ring.
#[derive(Clone, Copy)]
struct CqRing {
    head: *const AtomicU32,
    tail: *const AtomicU32,
    cqes: *mut iouring::io_uring_cqe,    // at CQ_OFF_CQES in the CQ ring page
    overflow_ctr: *mut AtomicU32,        // user-visible overflow counter (CQ_OFF_OVERFLOW)
    mask: u32,
    entries: u32,
}

// Raw pointers over kernel-mapped shared pages: Send + Sync by construction
// (the pages are pinned for the lifetime of the ring and accessed under the
// ring's producer/consumer discipline).
unsafe impl Send for SqRing {}
unsafe impl Sync for SqRing {}
unsafe impl Send for CqRing {}
unsafe impl Sync for CqRing {}

pub struct IoRing {
    entries: u32,
    sq_ring_pages: Arc<SharedPages>,
    cq_ring_pages: Arc<SharedPages>,
    sqes_pages: Arc<SharedPages>,
    sq: SqRing,
    cq: CqRing,
    registered_bufs: Mutex<Vec<Option<(PinnedUserBuf, u64)>>>,
    registered_files: Mutex<Vec<Option<Arc<dyn FileLike>>>>,
    /// Waiters blocked in `io_uring_enter(GETEVENTS)` are parked here; the
    /// worker wakes them from `post_cqe` once CQEs appear.
    cq_wq: Arc<WaitQueue>,
    /// CQEs that didn't fit the ring (the user isn't reaping fast enough) are
    /// buffered here and replayed oldest-first as ring slots free. Prevents the
    /// silent overwrite the old `post_cqe` performed when the ring was full.
    overflow: Arc<Mutex<VecDeque<(u64, i32)>>>,
}

fn page_ptr(pages: &SharedPages, idx: usize) -> *mut u8 {
    phys_to_virt(pages[idx]).as_usize() as *mut u8
}

fn zero_pages(pages: &SharedPages) {
    for i in 0..pages.len() {
        unsafe { core::ptr::write_bytes(page_ptr(pages, i), 0, 4096) };
    }
}

impl IoRing {
    /// Create a ring with `entries` slots, spawn its worker, and return the
    /// owning handle (caller registers it as an fd via `add_to_fd_table`).
    pub fn new(entries: u32) -> AxResult<Self> {
        // Clamp to a power of two within [1, 1 << 15].
        let entries = entries.clamp(1, 1 << 15).next_power_of_two();
        let entries_us = entries as usize;

        // SQ/CQ ring pages: one 4K page each is enough for entries <= ~250
        // (control fields + array[entries]*4 / cqes[entries]*16).
        let sq_ring_pages = Arc::new(SharedPages::new(4096, axhal::paging::PageSize::Size4K)?);
        let cq_ring_pages = Arc::new(SharedPages::new(4096, axhal::paging::PageSize::Size4K)?);
        // SQE array: round up to a whole number of 4K pages (SharedPages requires
        // page-aligned sizes).
        let sqes_bytes = (SQE_SIZE * entries_us).next_multiple_of(4096);
        let sqes_pages = Arc::new(SharedPages::new(
            sqes_bytes,
            axhal::paging::PageSize::Size4K,
        )?);

        zero_pages(&sq_ring_pages);
        zero_pages(&cq_ring_pages);
        zero_pages(&sqes_pages);

        // Lay out the control fields.
        let sq_base = page_ptr(&sq_ring_pages, 0);
        let cq_base = page_ptr(&cq_ring_pages, 0);
        let sqes_base = page_ptr(&sqes_pages, 0);

        unsafe {
            // SQ control
            *(sq_base.add(SQ_OFF_MASK) as *mut u32) = entries - 1;
            *(sq_base.add(SQ_OFF_ENTRIES) as *mut u32) = entries;
            // SQ array: identity mapping (array[i] = i) so a tail index maps 1:1.
            let arr = sq_base.add(SQ_OFF_ARRAY) as *mut u32;
            for i in 0..entries_us {
                *arr.add(i) = i as u32;
            }
            // CQ control
            *(cq_base.add(CQ_OFF_MASK) as *mut u32) = entries - 1;
            *(cq_base.add(CQ_OFF_ENTRIES) as *mut u32) = entries;
        }

        let sq = SqRing {
            head: unsafe { sq_base.add(SQ_OFF_HEAD) as *const AtomicU32 },
            tail: unsafe { sq_base.add(SQ_OFF_TAIL) as *const AtomicU32 },
            array: unsafe { sq_base.add(SQ_OFF_ARRAY) as *mut AtomicU32 },
            sqes: unsafe { sqes_base as *mut iouring::io_uring_sqe },
            mask: entries - 1,
            entries,
        };
        let cq = CqRing {
            head: unsafe { cq_base.add(CQ_OFF_HEAD) as *const AtomicU32 },
            tail: unsafe { cq_base.add(CQ_OFF_TAIL) as *const AtomicU32 },
            cqes: unsafe { cq_base.add(CQ_OFF_CQES) as *mut iouring::io_uring_cqe },
            overflow_ctr: unsafe { cq_base.add(CQ_OFF_OVERFLOW) as *mut AtomicU32 },
            mask: entries - 1,
            entries,
        };

        // Note: no per-ring worker. All rings share a single global worker
        // (see GLOBAL_QUEUE), spawned once from a boot/kernel context so it is
        // never tied to a user-process lifetime.
        Ok(Self {
            entries,
            sq_ring_pages,
            cq_ring_pages,
            sqes_pages,
            sq,
            cq,
            registered_bufs: Mutex::new(Vec::new()),
            registered_files: Mutex::new(Vec::new()),
            cq_wq: Arc::new(WaitQueue::new()),
            overflow: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    /// Register a batch of user buffers (`IORING_REGISTER_BUFFERS`).
    /// `arg` points to an array of iovec; each is resolved to phys pages via
    /// `PinnedUserBuf::resolve` (submitter's page table is live). No tags
    /// (tag = 0) — use `register_buffers2` for tagged registration.
    pub(crate) fn register_buffers(&self, arg: usize, nr: u32) -> AxResult<()> {
        #[repr(C)]
        struct IoVec {
            base: usize,
            len: usize,
        }
        let n = nr as usize;
        let mut table = self.registered_bufs.lock();
        table.resize_with(n, || None);
        for i in 0..n {
            let iov_ptr: crate::mm::UserConstPtr<IoVec> = (arg + i * 16).into();
            let iov = iov_ptr.get_as_ref()?;
            if iov.len > 0 {
                table[i] = Some((PinnedUserBuf::resolve(iov.base, iov.len)?, 0));
            }
        }
        Ok(())
    }

    /// `IORING_UNREGISTER_BUFFERS`: drop every registered buffer. Required by
    /// liburing's `unregister_buffers()` — the tokio-rs `test_register_buffers`
    /// calls it as a cleanup step, and the whole test fails with ENOSYS if it
    /// is missing.
    pub(crate) fn unregister_buffers(&self) -> AxResult<()> {
        self.registered_bufs.lock().clear();
        Ok(())
    }

    /// `IORING_REGISTER_BUFFERS2`: tagged buffer registration. `arg` points to
    /// `io_uring_rsrc_register { nr, flags, resv2, data, tags }` (32 bytes).
    /// With `IORING_RSRC_REGISTER_SPARSE` (flags & 1) it allocates an all-`None`
    /// table of `nr` slots (filled later via `register_buffers_update`).
    const IORING_RSRC_REGISTER_SPARSE: u32 = 1;
    pub(crate) fn register_buffers2(&self, arg: usize, _nr_args: u32) -> AxResult<()> {
        #[repr(C)]
        struct IoUringRsrcRegister {
            nr: u32,
            flags: u32,
            _resv2: u64,
            data: u64, // -> iovec array (0 for sparse)
            tags: u64, // -> u64 tag array (0 for none)
        }
        #[repr(C)]
        struct IoVec {
            base: usize,
            len: usize,
        }
        let r: crate::mm::UserConstPtr<IoUringRsrcRegister> = arg.into();
        let r = r.get_as_ref()?;
        let n = r.nr as usize;
        let mut table = self.registered_bufs.lock();
        if r.flags & Self::IORING_RSRC_REGISTER_SPARSE != 0 {
            // Sparse: pre-allocate the table, all slots unfilled.
            table.clear();
            table.resize_with(n, || None);
            return Ok(());
        }
        table.resize_with(n, || None);
        for i in 0..n {
            let iov_ptr: crate::mm::UserConstPtr<IoVec> = ((r.data as usize) + i * 16).into();
            let iov = iov_ptr.get_as_ref()?;
            let tag = if r.tags != 0 {
                let tag_ptr: crate::mm::UserConstPtr<u64> = ((r.tags as usize) + i * 8).into();
                tag_ptr.get_as_ref().map(|t| *t).unwrap_or(0)
            } else {
                0
            };
            if iov.len > 0 {
                table[i] = Some((PinnedUserBuf::resolve(iov.base, iov.len)?, tag));
            }
        }
        Ok(())
    }

    /// `IORING_REGISTER_BUFFERS_UPDATE`: replace a contiguous range of the
    /// registered-buffer table starting at `offset`. `arg` points to an
    /// `io_uring_rsrc_update2 { offset, _resv, data, tags, nr, _resv2 }`
    /// (32 bytes). Each entry's new tag comes from the `tags` array. When an
    /// already-registered slot with a **non-zero old tag** is replaced, the
    /// kernel posts a CQE with `user_data == old_tag` to signal the resource is
    /// released (this is the `IORING_FEAT_RSRC_TAGS` contract). Filling a sparse
    /// (`None`) slot posts no CQE. The table auto-grows with `None`.
    pub(crate) fn register_buffers_update(&self, arg: usize, _nr_args: u32) -> AxResult<()> {
        #[repr(C)]
        struct IoUringRsrcUpdate2 {
            offset: u32,
            _resv: u32,
            data: u64,  // -> iovec array
            tags: u64, // -> u64 tag array (0 if none)
            nr: u32,
            _resv2: u32,
        }
        #[repr(C)]
        struct IoVec {
            base: usize,
            len: usize,
        }
        let upd: crate::mm::UserConstPtr<IoUringRsrcUpdate2> = arg.into();
        let u = upd.get_as_ref()?;
        let off = u.offset as usize;
        let n = u.nr as usize;
        // Collect (old_tag) CQEs to post for replaced tagged slots. Done after
        // dropping the table lock so post_cqe (which locks `overflow`) doesn't
        // nest under registered_bufs.
        let mut released_tags: Vec<u64> = Vec::new();
        {
            let mut table = self.registered_bufs.lock();
            if off + n > table.len() {
                table.resize_with(off + n, || None);
            }
            for i in 0..n {
                let iov_ptr: crate::mm::UserConstPtr<IoVec> = ((u.data as usize) + i * 16).into();
                let iov = iov_ptr.get_as_ref()?;
                let new_tag = if u.tags != 0 {
                    let tag_ptr: crate::mm::UserConstPtr<u64> = ((u.tags as usize) + i * 8).into();
                    tag_ptr.get_as_ref().map(|t| *t).unwrap_or(0)
                } else {
                    0
                };
                // Replacing a registered slot whose old tag is non-zero posts a
                // resource-released CQE carrying the OLD tag as user_data.
                if let Some((_, old_tag)) = table[off + i] {
                    if old_tag != 0 {
                        released_tags.push(old_tag);
                    }
                }
                table[off + i] = if iov.len > 0 {
                    Some((PinnedUserBuf::resolve(iov.base, iov.len)?, new_tag))
                } else {
                    None
                };
            }
        }
        for tag in released_tags {
            post_cqe(&self.cq, &self.overflow, &self.cq_wq, tag, 0);
        }
        Ok(())
    }

    /// `IORING_REGISTER_FILES`: register an array of fds for use with
    /// `IOSQE_FIXED_FILE`. `arg` points to an array of `int` fds. On StarryOS
    /// we resolve each to an `Arc<dyn FileLike>` via `get_file_like` at register
    /// time (the submitter's fd table is live here).
    pub(crate) fn register_files(&self, arg: usize, nr: u32) -> AxResult<()> {
        let n = nr as usize;
        let fds: crate::mm::UserConstPtr<i32> = arg.into();
        let fd_slice = fds.get_as_slice(n)?;
        let mut table = self.registered_files.lock();
        table.resize_with(n, || None);
        for (i, &fd) in fd_slice.iter().enumerate() {
            if fd >= 0 {
                if let Ok(file) = get_file_like(fd) {
                    table[i] = Some(file);
                }
            }
        }
        Ok(())
    }

    /// `IORING_REGISTER_FILES_UPDATE`: update a range of the registered-files
    /// table starting at `offset`. `arg` points to `io_uring_files_update
    /// { offset, resv, fds }`, `nr` is the number of entries.
    pub(crate) fn register_files_update(&self, arg: usize, nr: u32) -> AxResult<()> {
        #[repr(C)]
        struct IoUringFilesUpdate {
            offset: u32,
            _resv: u32,
            fds: u64,  // -> int array
        }
        let upd: crate::mm::UserConstPtr<IoUringFilesUpdate> = arg.into();
        let u = upd.get_as_ref()?;
        let off = u.offset as usize;
        let n = nr as usize;
        let fds: crate::mm::UserConstPtr<i32> = (u.fds as usize).into();
        let fd_slice = fds.get_as_slice(n)?;
        let mut table = self.registered_files.lock();
        if off + n > table.len() {
            table.resize_with(off + n, || None);
        }
        for (i, &fd) in fd_slice.iter().enumerate() {
            table[off + i] = if fd >= 0 {
                get_file_like(fd).ok()
            } else {
                None
            };
        }
        Ok(())
    }

    /// `IORING_REGISTER_PROBE`: fill the user-provided `io_uring_probe` with the
    /// opcodes we support, so a liburing consumer (e.g. tokio-rs `io-uring-test`)
    /// can gate its opcode tests via `probe.is_supported(op)` instead of skipping
    /// them. Layout: 16-byte header `{last_op, ops_len, ...}` then `ops[op] =
    /// {op, _, flags, _}` (8 bytes each); `flags & IO_URING_OP_SUPPORTED(=1)`
    /// marks a supported opcode. `is_supported(op)` ⇒ `op <= last_op && ops[op].
    /// flags & 1`.
    pub(crate) fn register_probe(&self, arg: usize, nr: u32) -> AxResult<()> {
        #[repr(C)]
        struct ProbeHead {
            last_op: u8,
            ops_len: u8,
            _resv: u16,
            _resv2: [u32; 3],
        }
        #[repr(C)]
        struct ProbeOp {
            op: u8,
            _resv: u8,
            flags: u16,
            _resv2: u32,
        }
        const SUPPORTED: &[u8] = &[
            OP_NOP, OP_READV, OP_WRITEV, OP_READ_FIXED, OP_WRITE_FIXED, OP_POLL_ADD,
            OP_ACCEPT, OP_READ, OP_WRITE, OP_SEND, OP_RECV, OP_TIMEOUT,
        ];
        let last_op: u8 = *SUPPORTED.iter().max().unwrap(); // RECV = 27

        let head: crate::mm::UserPtr<ProbeHead> = arg.into();
        let h = head.get_as_mut()?;
        h.last_op = last_op;
        h.ops_len = last_op + 1;

        let n = (last_op as usize + 1).min(nr.min(256) as usize);
        let ops: crate::mm::UserPtr<ProbeOp> = (arg + 16).into();
        let ops_slice = ops.get_as_mut_slice(n)?;
        for (i, opref) in ops_slice.iter_mut().enumerate() {
            opref.op = i as u8;
            opref.flags = if SUPPORTED.contains(&(i as u8)) { 1 } else { 0 };
        }
        Ok(())
    }

    /// Select the SharedPages region for `mmap` at the given io_uring offset.
    pub fn shared_pages_for_offset(&self, offset: usize) -> AxResult<Arc<SharedPages>> {
        match offset as u32 {
            iouring::IORING_OFF_SQ_RING => Ok(self.sq_ring_pages.clone()),
            iouring::IORING_OFF_CQ_RING => Ok(self.cq_ring_pages.clone()),
            iouring::IORING_OFF_SQES => Ok(self.sqes_pages.clone()),
            _ => Err(AxError::InvalidInput),
        }
    }

    /// Fill in `sq_off` / `cq_off` so a user (or liburing) can locate the ring
    /// fields within the mmap'd pages.
    pub fn fill_offsets(sq_off: &mut iouring::io_sqring_offsets, cq_off: &mut iouring::io_cqring_offsets) {
        sq_off.head = SQ_OFF_HEAD as u32;
        sq_off.tail = SQ_OFF_TAIL as u32;
        sq_off.ring_mask = SQ_OFF_MASK as u32;
        sq_off.ring_entries = SQ_OFF_ENTRIES as u32;
        sq_off.flags = SQ_OFF_FLAGS as u32;
        sq_off.dropped = SQ_OFF_DROPPED as u32;
        sq_off.array = SQ_OFF_ARRAY as u32;

        cq_off.head = CQ_OFF_HEAD as u32;
        cq_off.tail = CQ_OFF_TAIL as u32;
        cq_off.ring_mask = CQ_OFF_MASK as u32;
        cq_off.ring_entries = CQ_OFF_ENTRIES as u32;
        cq_off.overflow = CQ_OFF_OVERFLOW as u32;
        cq_off.flags = CQ_OFF_FLAGS as u32;
        cq_off.cqes = CQ_OFF_CQES as u32;
    }
}

impl IoRing {
    /// Block (in the submitter's task context) until at least `min_complete`
    /// CQEs are available — counting both entries already in the ring and any
    /// buffered in the overflow replay list. Used by `io_uring_enter(GETEVENTS)`.
    ///
    /// The worker wakes `cq_wq` from `post_cqe`; `WaitQueue::wait_until`
    /// registers its listener before the final condition re-check, so no
    /// completion posted between the check and the registration is lost.
    pub(crate) fn wait_for_completions(&self, min_complete: u32) {
        if min_complete == 0 {
            return;
        }
        let head = self.cq.head;
        let tail = self.cq.tail;
        let overflow = &self.overflow;
        self.cq_wq.wait_until(|| {
            let in_ring = unsafe { (*tail).load(Ordering::Acquire) }
                .wrapping_sub(unsafe { (*head).load(Ordering::Acquire) });
            in_ring + overflow.lock().len() as u32 >= min_complete
        });
    }

    /// Read pending SQEs from the shared SQ ring and enqueue them as ops for
    /// the worker. Advances the SQ head with Release so the user sees the slots
    /// consumed. Shared by `io_uring_enter` and the boot-time self-test.
    ///
    /// Runs in the submitter's task context, so fd resolution (`get_file_like`,
    /// which uses the scope-local FD_TABLE) and user-buffer pinning
    /// (`PinnedUserBuf::resolve`, which walks the live page table) happen here —
    /// the worker only ever touches the already-resolved `Arc<dyn FileLike>` and
    /// phys pages.
    pub(crate) fn submit_pending(&self) -> (usize, Vec<(u64, Arc<Socket>)>) {
        let sq = &self.sq;
        let mask = sq.mask as usize;
        let tail = unsafe { (*sq.tail).load(Ordering::Acquire) };
        let head = unsafe { (*sq.head).load(Ordering::Relaxed) };
        let n = (tail.wrapping_sub(head) as usize).min(sq.entries as usize);
        // ACCEPT is handled in the submitter's context (it must install the new
        // fd via the scope-local FD_TABLE, which the worker cannot reach), so we
        // collect ACCEPT requests here and hand them back to io_uring_enter
        // rather than enqueuing them for the worker. Non-ACCEPT ops below go to
        // the worker as usual.
        let mut accepts: Vec<(u64, Arc<Socket>)> = Vec::new();

        // Resolve fd: if IOSQE_FIXED_FILE is set, `sqe.fd` is an index into
        // `registered_files`; otherwise it's a real fd number.
        fn resolve_fd(ring: &IoRing, fd_raw: i32, fixed: bool) -> Option<Arc<dyn FileLike>> {
            if fixed {
                let idx = fd_raw as usize;
                ring.registered_files.lock().get(idx).and_then(|s| s.clone())
            } else {
                get_file_like(fd_raw).ok()
            }
        }

        for i in 0..n {
            let slot = (head.wrapping_add(i as u32) as usize) & mask;
            let sqe_idx = unsafe { (*sq.array.add(slot)).load(Ordering::Acquire) } as usize & mask;
            let sqe = unsafe { &*sq.sqes.add(sqe_idx) };
            let fixed_file = sqe.flags & 1 != 0;  // IOSQE_FIXED_FILE

            if sqe.opcode == OP_ACCEPT {
                if let Some(listener) = resolve_fd(self, sqe.fd, fixed_file) {
                    if let Ok(sock) = listener.downcast_arc::<Socket>() {
                        accepts.push((sqe.user_data, sock));
                    }
                }
                continue;
            }

            // Resolve fd + user buffer(s) / poll mask in this (submitter) context.
            // For FIXED variants, clone a pre-registered buffer (no page walk).
            let mut poll_events = 0u32;
            let mut msg_flags = 0u32;
            let mut duration = Duration::ZERO;
            let (file, buf, use_opcode) = match sqe.opcode {
                OP_READ | OP_WRITE => {
                    let file = resolve_fd(self, sqe.fd, fixed_file);
                    let addr = unsafe { sqe.__bindgen_anon_2.addr as usize };
                    let buf = PinnedUserBuf::resolve(addr, sqe.len as usize)
                        .ok()
                        .map(|b| alloc::vec![b]);
                    (file, buf, sqe.opcode)
                }
                OP_SEND | OP_RECV => {
                    // Socket send/recv: same buffer layout as read/write, but the
                    // worker drives them through a readiness wait + nonblocking I/O
                    // (so a blocking RECV never wedges the single serial worker).
                    msg_flags = unsafe { sqe.__bindgen_anon_3.msg_flags };
                    let file = resolve_fd(self, sqe.fd, fixed_file);
                    let addr = unsafe { sqe.__bindgen_anon_2.addr as usize };
                    let buf = PinnedUserBuf::resolve(addr, sqe.len as usize)
                        .ok()
                        .map(|b| alloc::vec![b]);
                    (file, buf, sqe.opcode)
                }
                OP_READV | OP_WRITEV => {
                    let file = resolve_fd(self, sqe.fd, fixed_file);
                    let iov_base = unsafe { sqe.__bindgen_anon_2.addr as usize };
                    let nr = sqe.len as usize; // for readv/writev, len = iovec count
                    let mut bufs = Vec::new();
                    for i in 0..nr {
                        #[repr(C)]
                        struct IoVec {
                            base: usize,
                            len: usize,
                        }
                        let iov_ptr: crate::mm::UserConstPtr<IoVec> = (iov_base + i * 16).into();
                        if let Ok(iov) = iov_ptr.get_as_ref() {
                            if let Ok(b) = PinnedUserBuf::resolve(iov.base, iov.len) {
                                bufs.push(b);
                            }
                        }
                    }
                    let oc = if sqe.opcode == OP_READV { OP_READ } else { OP_WRITE };
                    (file, if bufs.is_empty() { None } else { Some(bufs) }, oc)
                }
                OP_READ_FIXED | OP_WRITE_FIXED => {
                    let file = resolve_fd(self, sqe.fd, fixed_file);
                    let idx = unsafe { sqe.__bindgen_anon_4.buf_index as usize };
                    let slot = self.registered_bufs.lock().get(idx).cloned();
                    match slot {
                        Some(Some((b, _tag))) => {
                            let oc = if sqe.opcode == OP_READ_FIXED {
                                OP_READ
                            } else {
                                OP_WRITE
                            };
                            (file, Some(alloc::vec![b]), oc)
                        }
                        // Sparse / unfilled slot (or out-of-range index) →
                        // -EFAULT: the user asked to use a fixed buffer that
                        // isn't registered at that index.
                        _ => (file, None, OP_FIXED_FAULT),
                    }
                }
                OP_POLL_ADD => {
                    poll_events = unsafe { sqe.__bindgen_anon_3.poll32_events };
                    (resolve_fd(self, sqe.fd, fixed_file), None, sqe.opcode)
                }
                OP_TIMEOUT => {
                    // sqe.addr -> __kernel_timespec { tv_sec: i64, tv_nsec: i64 }.
                    // Relative timer (the test form); ABS flags are ignored.
                    #[repr(C)]
                    struct Timespec {
                        tv_sec: i64,
                        tv_nsec: i64,
                    }
                    let ts_ptr: usize = unsafe { sqe.__bindgen_anon_2.addr as usize };
                    let dur = crate::mm::UserConstPtr::<Timespec>::from(ts_ptr)
                        .get_as_ref()
                        .ok()
                        .map(|ts| {
                            Duration::new(
                                ts.tv_sec.max(0) as u64,
                                ts.tv_nsec.max(0) as u32,
                            )
                        })
                        .unwrap_or(Duration::ZERO);
                    duration = dur;
                    (None, None, sqe.opcode)
                }
                _ => (None, None, sqe.opcode),
            };
            GLOBAL_QUEUE.push(Op {
                user_data: sqe.user_data,
                opcode: use_opcode,
                file,
                buf,
                poll_events,
                msg_flags,
                offset: unsafe { sqe.__bindgen_anon_1.off },
                duration,
                cq: self.cq,
                cq_pages: self.cq_ring_pages.clone(),
                cq_wq: self.cq_wq.clone(),
                overflow: self.overflow.clone(),
            });
        }
        unsafe { (*sq.head).store(head.wrapping_add(n as u32), Ordering::Release) };
        (n, accepts)
    }

    /// Drive one ACCEPT request in the submitter's task context: park until the
    /// listener is readable (a connection is pending), accept it, install the
    /// new socket as an fd, and post a CQE whose `res` is that fd (or -errno).
    pub(crate) fn complete_accept(&self, user_data: u64, listener: &Arc<Socket>) {
        let l = listener.clone();
        block_on(poll_fn(move |cx| {
            if l.poll().intersects(IoEvents::IN) {
                Poll::Ready(())
            } else {
                l.register(cx, IoEvents::IN);
                Poll::Pending
            }
        }));
        let res = match listener.accept() {
            Ok(inner) => {
                // add_to_fd_table takes ownership (like sys_accept4); it wraps the
                // socket in an Arc internally and inserts it into the fd table.
                Socket(inner)
                    .add_to_fd_table(false)
                    .map(|fd| fd as i32)
                    .unwrap_or(-9)
            }
            Err(_) => -5,
        };
        post_cqe(&self.cq, &self.overflow, &self.cq_wq, user_data, res);
    }
}

impl Drop for IoRing {
    fn drop(&mut self) {
        // Nothing to tear down: the rings are refcounted (an in-flight op's
        // `cq_pages` keeps the CQ pages alive until its CQE is posted), and the
        // worker is global/permanent. Any ops already queued for this ring will
        // still complete (posting a CQE no one reads is harmless).
    }
}

impl FileLike for IoRing {
    fn path(&self) -> Cow<'_, str> {
        format!("io_uring:[{}]", self as *const _ as usize).into()
    }
}

impl Pollable for IoRing {
    fn poll(&self) -> IoEvents {
        IoEvents::empty()
    }
    fn register(&self, _context: &mut Context<'_>, _events: IoEvents) {}
}

/// Post a completion (worker only → single writer of `cq.tail`). If the ring is
/// full, the CQE is buffered in `overflow` and replayed oldest-first as the user
/// reaps (advances `cq.head`), avoiding the silent overwrite the old version did
/// when the ring was full. Genuine backup beyond one ring's depth is dropped and
/// reported via the user-visible overflow counter. Wakes any submitter parked in
/// `io_uring_enter(GETEVENTS)`.
fn post_cqe(
    cq: &CqRing,
    overflow: &Mutex<VecDeque<(u64, i32)>>,
    wq: &WaitQueue,
    user_data: u64,
    res: i32,
) {
    let head = unsafe { (*cq.head).load(Ordering::Acquire) };
    let mut tail = unsafe { (*cq.tail).load(Ordering::Relaxed) };
    let mut pend = overflow.lock();
    pend.push_back((user_data, res));
    // Drain as many buffered entries as fit (oldest first).
    while tail.wrapping_sub(head) < cq.entries {
        let Some((ud, r)) = pend.pop_front() else { break };
        let idx = (tail & cq.mask) as usize;
        unsafe {
            let cqe = cq.cqes.add(idx);
            (*cqe).user_data = ud;
            (*cqe).res = r;
            (*cqe).flags = 0;
        }
        tail = tail.wrapping_add(1);
    }
    unsafe { (*cq.tail).store(tail, Ordering::Release) };
    // Catastrophic backup: replay buffer itself exceeded one ring's depth.
    // Drop the oldest and bump the user-visible overflow counter.
    if pend.len() > cq.entries as usize {
        pend.pop_front();
        let ov = unsafe { (*cq.overflow_ctr).load(Ordering::Relaxed) };
        unsafe { (*cq.overflow_ctr).store(ov + 1, Ordering::Release) };
    }
    drop(pend);
    wq.notify_all(false);
}

/// Global op queue + worker shared by ALL IoRing instances. Initialized on
/// first use; the worker is spawned from a kernel context (the first submit
/// comes from the boot self-test), so it is never tied to a user-process
/// lifetime. (An earlier per-ring worker spawned inside a user syscall was
/// killed at process exit, hanging the gc task — the global worker fixes that.)
lazy_static! {
    static ref GLOBAL_QUEUE: Arc<OpQueue> = {
        let queue = Arc::new(OpQueue::new());
        let q = queue.clone();
        spawn_with_name(move || global_worker(q), "io_uring-worker".into());
        queue
    };
}

/// Per-op completion-routing state shared between the worker task and every
/// in-flight op's [`OpWaker`].
struct WorkerShared {
    /// op-ids whose `OpWaker` has fired since the worker last drained. The
    /// worker re-polls only these (plus newly-arrived ops) each round.
    ready: Mutex<VecDeque<usize>>,
    /// The `block_on` task's waker. `OpWaker` reads this lazily at wake time
    /// (never captures a clone) so it always sees the freshest one.
    worker_waker: Mutex<Option<Waker>>,
}

/// A per-op waker: when a driver (socket / pipe / timer) fires it, it pushes
/// the op's id into `ready` and wakes the worker once. Modeled on epoll's
/// `InterestWaker` (`kernel/src/file/epoll.rs`). Because each in-flight op has
/// its OWN waker, a readiness event on one fd re-polls ONLY that op — there is
/// no broadcast re-scan of all in-flight ops (the O(N²) that capped the old
/// worker at ~96 connections, where it spent 6124 wakeups to reap 288 CQEs).
struct OpWaker {
    id: usize,
    shared: Arc<WorkerShared>,
}

impl Wake for OpWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        // Push the id BEFORE taking+waking, so the id is guaranteed visible to
        // the worker's next drain (no lost wakeup even if the wake races the
        // worker's park).
        self.shared.ready.lock().push_back(self.id);
        if let Some(w) = self.shared.worker_waker.lock().take() {
            w.wake();
        }
    }
}

/// Monotonic op-id allocator (globally unique across the worker's lifetime;
/// ids are never reused, so a stale `OpWaker` firing later cannot alias a new
/// op).
static NEXT_OP_ID: AtomicUsize = AtomicUsize::new(0);

/// The single, permanent io_uring worker — a **multiplexing event loop** on one
/// task (single-core friendly). Unlike the old "poll ALL in-flight every wake"
/// loop (O(N²), which thrashed at ~96 connections), this uses **per-op
/// completion routing**: each in-flight op holds its own [`OpWaker`], so a
/// readiness event re-polls only the op it belongs to. The worker drains the
/// ready set + newly-arrived ops, polls just those, and parks when both are
/// empty. O(ready) per event; O(1) task/stack memory preserved (one worker
/// task, heap-resident `OpFuture`/`OpWaker` — no per-op stack).
fn global_worker(queue: Arc<OpQueue>) {
    let shared = Arc::new(WorkerShared {
        ready: Mutex::new(VecDeque::new()),
        worker_waker: Mutex::new(None),
    });
    block_on(async move {
        // id -> (op future, its per-op waker Arc). One entry per in-flight op;
        // no per-op task, no per-op stack -> O(1) task/stack memory.
        let mut inflight: HashMap<usize, (OpFuture, Arc<OpWaker>)> = HashMap::new();
        loop {
            // ── Phase A: decide which op-ids to poll this round; park if none.
            let to_poll: Vec<usize> = poll_fn(|cx| {
                // (1) Arm BOTH wake sources BEFORE draining, so any fire during
                //     the drain still wakes us:
                //       - shared.worker_waker  : fired by OpWaker (resource ready)
                //       - OpQueue.worker_waker : fired by submitter push (new op)
                *shared.worker_waker.lock() = Some(cx.waker().clone());
                queue.register_worker(cx);

                let mut ids = Vec::new();
                // (2) Drain newly-arrived ops: assign an id, store, and schedule
                //     an INITIAL poll. A fresh op has registered nothing yet, so
                //     it must be polled once to either complete (NOP) or arm its
                //     waker on the driver.
                while let Some(op) = queue.try_pop() {
                    let id = NEXT_OP_ID.fetch_add(1, Ordering::Relaxed);
                    let opw = Arc::new(OpWaker {
                        id,
                        shared: shared.clone(),
                    });
                    inflight.insert(id, (OpFuture::from(op), opw));
                    ids.push(id);
                }
                // (3) Drain ids whose OpWaker fired since the last round.
                while let Some(id) = shared.ready.lock().pop_front() {
                    ids.push(id);
                }

                if ids.is_empty() {
                    Poll::Pending
                } else {
                    Poll::Ready(ids)
                }
            })
            .await;

            // ── Phase B: poll ONLY the candidate ids (ready + new arrivals).
            // Collect completions and post them AFTER the poll loop (mirrors the
            // old worker's deferred-batch post, so no CQE is written mid-poll —
            // important under CQ overflow, where echo at >64 concurrent ops
            // fills the 64-deep ring).
            let mut done: Vec<(CqRing, Arc<WaitQueue>, Arc<Mutex<VecDeque<(u64, i32)>>>, Arc<SharedPages>, u64, i32)> =
                Vec::new();
            for id in to_poll {
                // `remove` returns None for spurious ids: an op completed and was
                // removed, but a stale OpWaker clone still sitting in some driver
                // PollSet fired later. Monotonic ids never collide, so this is a
                // safe no-op.
                let Some((mut f, opw)) = inflight.remove(&id) else {
                    continue;
                };
                let waker = Waker::from(opw.clone());
                let mut cx = Context::from_waker(&waker);
                match Pin::new(&mut f).poll(&mut cx) {
                    Poll::Ready(res) => {
                        // `_pages` (the CQ-pages keepalive) rides in `done` until
                        // after `post_cqe` below.
                        let (cq, wq, ov, pages, ud) = f.meta();
                        done.push((cq, wq, ov, pages, ud, res));
                        // `f` and `opw` drop here. Stale `opw` clones may still
                        // live in driver PollSets; their later wake() is the
                        // harmless spurious case handled by the `remove` guard.
                    }
                    Poll::Pending => {
                        // Waker already armed during this poll; re-insert and
                        // wait for its fire to re-schedule this id.
                        inflight.insert(id, (f, opw));
                    }
                }
            }
            for (cq, wq, ov, _pages, ud, res) in done {
                post_cqe(&cq, &ov, &wq, ud, res);
            }
        }
    });
}

/// Boot-time self-test: submit one NOP through the shared SQ ring, drive the
/// worker, and verify a matching CQE appears. Exercises the SharedPages ring,
/// the Acquire/Release ordering, and the worker end-to-end (no syscall/mmap).
pub fn selftest() {
    warn!("io_uring selftest: starting");
    let ring = match IoRing::new(8) {
        Ok(r) => r,
        Err(e) => {
            warn!("io_uring selftest: setup failed: {e:?}");
            return;
        }
    };

    // Drop a NOP SQE into sqes[0] and publish it via the SQ ring.
    let sq = &ring.sq;
    unsafe {
        let sqe = &mut *sq.sqes.add(0);
        core::ptr::write_bytes(sqe as *mut _ as *mut u8, 0, 64);
        sqe.opcode = OP_NOP;
        sqe.user_data = 0x1234_5678;
        // sq.array[0] is already 0 (identity), so advance the tail.
        (*sq.tail).store(1, Ordering::Release);
    }

    // Submit (reads the SQ ring exactly like io_uring_enter does).
    let (submitted, _accepts) = ring.submit_pending();

    // Reap the CQE. SMP=1, so we must yield to let the worker run.
    let cq = &ring.cq;
    let mut ok = false;
    for _ in 0..1_000_000 {
        let head = unsafe { (*cq.head).load(Ordering::Acquire) };
        let tail = unsafe { (*cq.tail).load(Ordering::Acquire) };
        if tail != head {
            let idx = (head & cq.mask) as usize;
            let cqe = unsafe { &*cq.cqes.add(idx) };
            ok = cqe.user_data == 0x1234_5678 && cqe.res == 0;
            unsafe { (*cq.head).store(head.wrapping_add(1), Ordering::Release) };
            break;
        }
        yield_now();
    }

    if ok {
        warn!("io_uring selftest: PASS (NOP round-trip, submitted={submitted})");
    } else {
        warn!("io_uring selftest: FAIL (no/incorrect CQE, submitted={submitted})");
    }
    // `ring` drops here → worker observes shutdown and exits.
}
