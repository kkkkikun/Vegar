// Minimal liburing shim implementation — see liburing_shim.h.
//   riscv64-linux-musl-gcc -static -O2 -c liburing_shim.c -o liburing_shim.o
#include "liburing_shim.h"

#include <unistd.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>

/* The kernel indexes the shared SQE array with a 64-byte stride; the CQE array
 * with 16 bytes. The shim indexes by sizeof, so these MUST match exactly. */
_Static_assert(sizeof(struct io_uring_sqe) == 64, "io_uring_sqe must be 64 bytes");
_Static_assert(sizeof(struct io_uring_cqe) == 16, "io_uring_cqe must be 16 bytes");

#define __NR_io_uring_setup   425
#define __NR_io_uring_enter   426
#define __NR_io_uring_register 427

static long io_setup(unsigned entries, struct io_uring_params *p) {
    return syscall(__NR_io_uring_setup, entries, p);
}
static long io_enter(int fd, unsigned to_submit, unsigned min_complete,
                     unsigned flags) {
    return syscall(__NR_io_uring_enter, fd, to_submit, min_complete, flags, 0, 0);
}

static void *ring_mmap(int fd, size_t sz, unsigned long offset) {
    void *p = mmap(NULL, sz, PROT_READ | PROT_WRITE, MAP_SHARED, fd, offset);
    return (p == MAP_FAILED) ? NULL : p;
}

int io_uring_queue_init(unsigned entries, struct io_uring *ring, unsigned flags) {
    memset(ring, 0, sizeof *ring);
    struct io_uring_params p;
    memset(&p, 0, sizeof p);
    /* We do not yet honor setup flags (SQPOLL/IOPOLL) — pass 0. */
    (void)flags;

    int fd = (int)io_setup(entries, &p);
    if (fd < 0) return fd;
    ring->ring_fd = fd;

    size_t page = 4096;
    char *sq_ring = ring_mmap(fd, page, IORING_OFF_SQ_RING);
    if (!sq_ring) return -1;

    /* ── CQ ring: kernel may give us more cq_entries than we asked for
     *    (io_uring §4.2 mandates CQ ≥ 2× SQ). Compute the correct mmap
     *    size from the kernel-reported cq_entries BEFORE mapping. ── */
    {
        /* Read cq_entries from the SQ ring page (sq_off is there but we need
         * cq_entries from the io_uring_params we already have). */
        unsigned cq_ents = p.cq_entries;
        if (cq_ents == 0) cq_ents = p.sq_entries * 2;
        size_t cq_sz = ((size_t)p.cq_off.cqes + (size_t)cq_ents * 16 + page - 1) & ~(page - 1);
        if (cq_sz < page) cq_sz = page;
        char *cq_ring = ring_mmap(fd, cq_sz, IORING_OFF_CQ_RING);
        if (!cq_ring) return -1;
        ring->cq.ring_ptr = cq_ring;
        ring->cq.head        = (unsigned *)(cq_ring + p.cq_off.head);
        ring->cq.tail        = (unsigned *)(cq_ring + p.cq_off.tail);
        ring->cq.ring_mask   = (unsigned *)(cq_ring + p.cq_off.ring_mask);
        ring->cq.ring_entries= (unsigned *)(cq_ring + p.cq_off.ring_entries);
        ring->cq.overflow    = (unsigned *)(cq_ring + p.cq_off.overflow);
        ring->cq.cqes        = (struct io_uring_cqe *)(cq_ring + p.cq_off.cqes);
    }

    ring->sq.ring_ptr = sq_ring;

    ring->sq.head        = (unsigned *)(sq_ring + p.sq_off.head);
    ring->sq.tail        = (unsigned *)(sq_ring + p.sq_off.tail);
    ring->sq.ring_mask   = (unsigned *)(sq_ring + p.sq_off.ring_mask);
    ring->sq.ring_entries= (unsigned *)(sq_ring + p.sq_off.ring_entries);
    ring->sq.flags       = (unsigned *)(sq_ring + p.sq_off.flags);
    ring->sq.dropped     = (unsigned *)(sq_ring + p.sq_off.dropped);
    ring->sq.array       = (unsigned *)(sq_ring + p.sq_off.array);

    /* The SQE array is sized by the KERNEL's actual (power-of-two) entries, not
     * our request — mmap that many bytes (rounded to a page). The kernel rounds
     * entries up to a power of two, so a 1-page mmap (64 SQEs) overflows for any
     * larger ring and get_sqe writes past the mapping → SIGSEGV. */
    unsigned kentries = *ring->sq.ring_entries;
    size_t sqes_sz = ((size_t)kentries * 64 + page - 1) & ~(page - 1);
    if (sqes_sz < page) sqes_sz = page;
    char *sqes = ring_mmap(fd, sqes_sz, IORING_OFF_SQES);
    if (!sqes) return -1;
    ring->sq.sqes = (struct io_uring_sqe *)sqes;

    ring->sq.sqe_head = ring->sq.sqe_tail = 0;
    return 0;
}

void io_uring_queue_exit(struct io_uring *ring) {
    size_t page = 4096;
    /* Read kentries BEFORE unmapping — ring pointers point INTO the mappings. */
    unsigned kentries = ring->sq.ring_entries ? *ring->sq.ring_entries : 0;
    unsigned cq_ents = ring->cq.ring_entries ? *ring->cq.ring_entries : 0;
    if (ring->sq.ring_ptr) munmap(ring->sq.ring_ptr, page);
    if (ring->cq.ring_ptr) {
        size_t cq_sz = ((size_t)0x18 + (size_t)cq_ents * 16 + page - 1) & ~(page - 1);
        if (cq_sz < page) cq_sz = page;
        munmap(ring->cq.ring_ptr, cq_sz);
    }
    if (ring->sq.sqes) {
        size_t sqes_sz = ((size_t)kentries * 64 + page - 1) & ~(page - 1);
        if (sqes_sz < page) sqes_sz = page;
        munmap(ring->sq.sqes, sqes_sz);
    }
    close(ring->ring_fd);
    memset(ring, 0, sizeof *ring);
}

unsigned io_uring_sq_ready(struct io_uring *ring) {
    return ring->sq.sqe_head - ring->sq.sqe_tail;
}
unsigned io_uring_sq_space_left(struct io_uring *ring) {
    return *ring->sq.ring_entries - io_uring_sq_ready(ring);
}
unsigned io_uring_cq_ready(struct io_uring *ring) {
    return *ring->cq.tail - *ring->cq.head;
}

struct io_uring_sqe *io_uring_get_sqe(struct io_uring *ring) {
    if (io_uring_sq_ready(ring) >= *ring->sq.ring_entries) return NULL;
    struct io_uring_sqe *sqe = &ring->sq.sqes[ring->sq.sqe_head & *ring->sq.ring_mask];
    ring->sq.sqe_head++;
    memset(sqe, 0, sizeof *sqe);
    return sqe;
}

/* Flush locally-allocated SQEs into the shared SQ ring (array + tail), then
 * optionally call enter. Returns number submitted. */
static int io_uring_flush_and_enter(struct io_uring *ring, unsigned min_complete,
                                    unsigned flags) {
    unsigned ktail = *ring->sq.tail;
    unsigned ktail_before = ktail;
    while (ring->sq.sqe_tail < ring->sq.sqe_head) {
        unsigned idx = ktail & *ring->sq.ring_mask;
        /* Identity mapping: array slot holds the SQE index. */
        ring->sq.array[idx] = ring->sq.sqe_tail & *ring->sq.ring_mask;
        ring->sq.sqe_tail++;
        ktail++;
    }
    if (ktail != ktail_before) {
        __atomic_store_n(ring->sq.tail, ktail, __ATOMIC_RELEASE);
    }
    unsigned to_submit = ktail - ktail_before;
    if (to_submit == 0 && !(flags & IORING_ENTER_GETEVENTS)) return 0;
    long r = io_enter(ring->ring_fd, to_submit, min_complete, flags);
    return (int)r;
}

int io_uring_submit(struct io_uring *ring) {
    return io_uring_flush_and_enter(ring, 0, 0);
}
int io_uring_submit_and_wait(struct io_uring *ring, unsigned wait_nr) {
    return io_uring_flush_and_enter(ring, wait_nr, IORING_ENTER_GETEVENTS);
}

int io_uring_peek_batch_cqe(struct io_uring *ring, struct io_uring_cqe **cqes,
                            unsigned nr) {
    unsigned ready = io_uring_cq_ready(ring);
    unsigned count = ready < nr ? ready : nr;
    unsigned head = *ring->cq.head;
    for (unsigned i = 0; i < count; i++)
        cqes[i] = &ring->cq.cqes[(head + i) & *ring->cq.ring_mask];
    return (int)count;
}

int io_uring_wait_cqe_nr(struct io_uring *ring, struct io_uring_cqe **cqe_ptr,
                         unsigned wait_nr) {
    if (io_uring_cq_ready(ring) < wait_nr) {
        /* Block in the kernel until wait_nr CQEs are available. */
        long r = io_enter(ring->ring_fd, 0, wait_nr, IORING_ENTER_GETEVENTS);
        if (r < 0) return (int)r;
    }
    if (io_uring_cq_ready(ring) == 0) return -1;
    unsigned head = __atomic_load_n(ring->cq.head, __ATOMIC_ACQUIRE);
    *cqe_ptr = &ring->cq.cqes[head & *ring->cq.ring_mask];
    return 0;
}

int io_uring_wait_cqe(struct io_uring *ring, struct io_uring_cqe **cqe_ptr) {
    return io_uring_wait_cqe_nr(ring, cqe_ptr, 1);
}

void io_uring_cqe_seen(struct io_uring *ring, struct io_uring_cqe *cqe) {
    (void)cqe;
    unsigned head = __atomic_load_n(ring->cq.head, __ATOMIC_ACQUIRE);
    __atomic_store_n(ring->cq.head, head + 1, __ATOMIC_RELEASE);
}

int io_uring_register_buffers(struct io_uring *ring, const struct iovec *iovs,
                              unsigned nr_iovs) {
    return (int)syscall(__NR_io_uring_register, ring->ring_fd,
                        0 /* IORING_REGISTER_BUFFERS */, iovs, nr_iovs);
}

/* ── queue_init_params: like queue_init but passes user-provided params ── */
int io_uring_queue_init_params(unsigned entries, struct io_uring *ring,
                               struct io_uring_params *p) {
    memset(ring, 0, sizeof *ring);
    int fd = (int)io_setup(entries, p);
    if (fd < 0) return fd;
    ring->ring_fd = fd;

    size_t page = 4096;
    char *sq_ring = ring_mmap(fd, page, IORING_OFF_SQ_RING);
    if (!sq_ring) return -1;

    /* CQ ring mmap size from kernel-reported cq_entries (may be 2× SQ). */
    {
        unsigned cq_ents = p->cq_entries ? p->cq_entries : p->sq_entries * 2;
        size_t cq_sz = ((size_t)p->cq_off.cqes + (size_t)cq_ents * 16 + page - 1) & ~(page - 1);
        if (cq_sz < page) cq_sz = page;
        char *cq_ring = ring_mmap(fd, cq_sz, IORING_OFF_CQ_RING);
        if (!cq_ring) return -1;
        ring->cq.ring_ptr = cq_ring;
        ring->cq.head        = (unsigned *)(cq_ring + p->cq_off.head);
        ring->cq.tail        = (unsigned *)(cq_ring + p->cq_off.tail);
        ring->cq.ring_mask   = (unsigned *)(cq_ring + p->cq_off.ring_mask);
        ring->cq.ring_entries= (unsigned *)(cq_ring + p->cq_off.ring_entries);
        ring->cq.overflow    = (unsigned *)(cq_ring + p->cq_off.overflow);
        ring->cq.cqes        = (struct io_uring_cqe *)(cq_ring + p->cq_off.cqes);
    }

    ring->sq.ring_ptr = sq_ring;

    ring->sq.head        = (unsigned *)(sq_ring + p->sq_off.head);
    ring->sq.tail        = (unsigned *)(sq_ring + p->sq_off.tail);
    ring->sq.ring_mask   = (unsigned *)(sq_ring + p->sq_off.ring_mask);
    ring->sq.ring_entries= (unsigned *)(sq_ring + p->sq_off.ring_entries);
    ring->sq.flags       = (unsigned *)(sq_ring + p->sq_off.flags);
    ring->sq.dropped     = (unsigned *)(sq_ring + p->sq_off.dropped);
    ring->sq.array       = (unsigned *)(sq_ring + p->sq_off.array);

    unsigned kentries = *ring->sq.ring_entries;
    size_t sqes_sz = ((size_t)kentries * 64 + page - 1) & ~(page - 1);
    if (sqes_sz < page) sqes_sz = page;
    char *sqes = ring_mmap(fd, sqes_sz, IORING_OFF_SQES);
    if (!sqes) return -1;
    ring->sq.sqes = (struct io_uring_sqe *)sqes;

    ring->sq.sqe_head = ring->sq.sqe_tail = 0;
    return 0;
}

/* ── register files (IORING_REGISTER_FILES = 2) ── */
int io_uring_register_files(struct io_uring *ring, const int *files, unsigned nr) {
    return (int)syscall(__NR_io_uring_register, ring->ring_fd,
                        2 /* IORING_REGISTER_FILES */, files, nr);
}

/* ── register files update (IORING_REGISTER_FILES_UPDATE = 18) ── */
int io_uring_register_files_update(struct io_uring *ring, unsigned off,
                                   const int *files, unsigned nr) {
    struct io_uring_files_update {
        uint32_t offset;
        uint32_t resv;
        uint64_t fds;
    } up = {
        .offset = off,
        .resv = 0,
        .fds = (uint64_t)(uintptr_t)files,
    };
    return (int)syscall(__NR_io_uring_register, ring->ring_fd,
                        18 /* IORING_REGISTER_FILES_UPDATE */, &up, nr);
}
