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

use alloc::{borrow::Cow, collections::VecDeque, format, sync::Arc, vec::Vec};
use core::{
    future::poll_fn,
    sync::atomic::{AtomicU32, Ordering},
    task::{Context, Poll},
    time::Duration,
};

use axerrno::{AxError, AxResult};
use axhal::mem::phys_to_virt;
use axio::{IoBuf, IoBufMut, Read, Write};
use axpoll::{IoEvents, Pollable};
use axsync::Mutex;
use axtask::{WaitQueue, current, future::block_on, spawn_with_name, yield_now};
use lazy_static::lazy_static;
use linux_raw_sys::io_uring as iouring;
use memory_addr::{PhysAddr, VirtAddr};

use crate::{file::get_file_like, mm::SharedPages, task::AsThread};

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
const OP_READ: u8 = iouring::io_uring_op::IORING_OP_READ as u8;
const OP_WRITE: u8 = iouring::io_uring_op::IORING_OP_WRITE as u8;
const OP_READ_FIXED: u8 = iouring::io_uring_op::IORING_OP_READ_FIXED as u8;
const OP_WRITE_FIXED: u8 = iouring::io_uring_op::IORING_OP_WRITE_FIXED as u8;
const OP_POLL_ADD: u8 = iouring::io_uring_op::IORING_OP_POLL_ADD as u8;

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
    /// Resolved user buffer (READ/WRITE). `None` for NOP.
    buf: Option<PinnedUserBuf>,
    /// Requested poll events for POLL_ADD (0 otherwise).
    poll_events: u32,
    /// This op's CQ ring view + the pages that back it (keepalive: the pages
    /// stay pinned until the CQE is posted, even if the fd is already closed).
    cq: CqRing,
    cq_pages: Arc<SharedPages>,
}

/// Race-free blocking queue: submitter pushes, worker pops.
struct OpQueue {
    inner: Mutex<VecDeque<Op>>,
    wq: WaitQueue,
}

impl OpQueue {
    const fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            wq: WaitQueue::new(),
        }
    }

    fn push(&self, op: Op) {
        self.inner.lock().push_back(op);
        self.wq.notify_one(true);
    }

    fn pop(&self) -> Op {
        // Bounded wait: wake immediately on `notify` for normal ops, and every
        // 50ms as a fallback. The fallback matters for shutdown — the worker
        // must notice `shutdown=true` even if the notify fired from a context
        // (e.g. `IoRing::drop` in the gc task) that can't reschedule us.
        loop {
            if let Some(op) = self.inner.lock().pop_front() {
                return op;
            }
            self.wq.wait_timeout(Duration::from_millis(50));
        }
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
    cqes: *mut iouring::io_uring_cqe, // at CQ_OFF_CQES in the CQ ring page
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
    registered_bufs: Mutex<Vec<Option<PinnedUserBuf>>>,
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
        })
    }

    /// Register a batch of user buffers (`IORING_REGISTER_BUFFERS`).
    /// `arg` points to an array of iovec; each is resolved to phys pages via
    /// `PinnedUserBuf::resolve` (submitter's page table is live).
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
                table[i] = Some(PinnedUserBuf::resolve(iov.base, iov.len)?);
            }
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
    /// Read pending SQEs from the shared SQ ring and enqueue them as ops for
    /// the worker. Advances the SQ head with Release so the user sees the slots
    /// consumed. Shared by `io_uring_enter` and the boot-time self-test.
    ///
    /// Runs in the submitter's task context, so fd resolution (`get_file_like`,
    /// which uses the scope-local FD_TABLE) and user-buffer pinning
    /// (`PinnedUserBuf::resolve`, which walks the live page table) happen here —
    /// the worker only ever touches the already-resolved `Arc<dyn FileLike>` and
    /// phys pages.
    pub(crate) fn submit_pending(&self) -> usize {
        let sq = &self.sq;
        let mask = sq.mask as usize;
        let tail = unsafe { (*sq.tail).load(Ordering::Acquire) };
        let head = unsafe { (*sq.head).load(Ordering::Relaxed) };
        let n = (tail.wrapping_sub(head) as usize).min(sq.entries as usize);
        for i in 0..n {
            let slot = (head.wrapping_add(i as u32) as usize) & mask;
            let sqe_idx = unsafe { (*sq.array.add(slot)).load(Ordering::Acquire) } as usize & mask;
            let sqe = unsafe { &*sq.sqes.add(sqe_idx) };

            // Resolve fd + user buffer / poll mask in this (submitter) context.
            // For FIXED variants, clone a pre-registered buffer (no page walk).
            let mut poll_events = 0u32;
            let (file, buf, use_opcode) = match sqe.opcode {
                OP_READ | OP_WRITE => {
                    let file = get_file_like(sqe.fd).ok();
                    let addr = unsafe { sqe.__bindgen_anon_2.addr as usize };
                    let buf = PinnedUserBuf::resolve(addr, sqe.len as usize).ok();
                    (file, buf, sqe.opcode)
                }
                OP_READ_FIXED | OP_WRITE_FIXED => {
                    let file = get_file_like(sqe.fd).ok();
                    let idx = unsafe { sqe.__bindgen_anon_4.buf_index as usize };
                    let buf = self
                        .registered_bufs
                        .lock()
                        .get(idx)
                        .and_then(Option::as_ref)
                        .map(PinnedUserBuf::clone);
                    let oc = if sqe.opcode == OP_READ_FIXED {
                        OP_READ
                    } else {
                        OP_WRITE
                    };
                    (file, buf, oc)
                }
                OP_POLL_ADD => {
                    poll_events = unsafe { sqe.__bindgen_anon_3.poll32_events };
                    (get_file_like(sqe.fd).ok(), None, sqe.opcode)
                }
                _ => (None, None, sqe.opcode),
            };
            GLOBAL_QUEUE.push(Op {
                user_data: sqe.user_data,
                opcode: use_opcode,
                file,
                buf,
                poll_events,
                cq: self.cq,
                cq_pages: self.cq_ring_pages.clone(),
            });
        }
        unsafe { (*sq.head).store(head.wrapping_add(n as u32), Ordering::Release) };
        n
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

/// Post a completion. Only the worker calls this, so `cq.tail` is single-writer.
fn post_cqe(cq: &CqRing, user_data: u64, res: i32) {
    let tail = unsafe { (*cq.tail).load(Ordering::Relaxed) };
    let idx = (tail & cq.mask) as usize;
    unsafe {
        let cqe = cq.cqes.add(idx);
        (*cqe).user_data = user_data;
        (*cqe).res = res;
        (*cqe).flags = 0;
    }
    unsafe { (*cq.tail).store(tail.wrapping_add(1), Ordering::Release) };
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

/// The single, permanent io_uring worker. Processes ops sequentially within
/// this task: each op blocks via `block_on` / `poll_io` inside the fd's own
/// `read`/`write` impl, parking the *worker* (not a separate task). This is a
/// simple cooperative-coroutine model — the existing axtask async runtime
/// (`block_on` / `AxWaker` / `poll_fn`) is the coroutine executor. No per-op
/// task allocation means near-zero scheduling overhead.
fn global_worker(queue: Arc<OpQueue>) {
    loop {
        let op = queue.pop();
        let res: i32 = match op.opcode {
            OP_NOP => 0,
            OP_READ => match (op.file, op.buf) {
                (Some(file), Some(mut buf)) => {
                    file.read(&mut buf).map(|n| n as i32).unwrap_or(-5)
                }
                _ => -9,
            },
            OP_WRITE => match (op.file, op.buf) {
                (Some(file), Some(mut buf)) => {
                    file.write(&mut buf).map(|n| n as i32).unwrap_or(-5)
                }
                _ => -9,
            },
            // POLL_ADD: park the worker on this fd until the requested events
            // fire. The worker is the coroutine — no separate task needed.
            OP_POLL_ADD => match op.file {
                Some(file) => {
                    let events = IoEvents::from_bits_truncate(op.poll_events);
                    block_on(poll_fn(move |cx| {
                        let cur = file.poll();
                        if cur.intersects(events) {
                            Poll::Ready(cur.bits() as i32)
                        } else {
                            file.register(cx, events);
                            Poll::Pending
                        }
                    }))
                }
                None => -9,
            },
            _ => -38,
        };
        // op.cq_pages 保持存活直至 CQE 写入
        post_cqe(&op.cq, op.user_data, res);
    }
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
    let submitted = ring.submit_pending();

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
