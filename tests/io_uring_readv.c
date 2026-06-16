// io_uring READV / WRITEV test (Stage 1 / S1.3).
// Writes 3 iovec segments to a pipe via WRITEV, reads them back via READV, and
// verifies the bytes round-trip into the correct segments. Exercises the
// Op.buf = Vec<PinnedUserBuf> path (multiple iovecs per op).
//
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_readv.c -o io_uring_readv
#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/mman.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426

#define IORING_OFF_SQ_RING 0u
#define IORING_OFF_CQ_RING 0x8000000u
#define IORING_OFF_SQES    0x10000000u

#define IORING_OP_READV  1
#define IORING_OP_WRITEV 2
#define IORING_ENTER_GETEVENTS 1u

struct iovec { void *base; size_t len; };

struct io_uring_sqe {
    uint8_t opcode, flags; uint16_t ioprio; int32_t fd;
    uint64_t off, addr; uint32_t len, rw_flags;
    uint64_t user_data; uint16_t buf_index, personality; uint32_t __pad2[3];
} __attribute__((packed));
struct io_uring_cqe { uint64_t user_data; int32_t res; uint32_t flags; };
struct io_sqring_offsets { uint32_t head, tail, ring_mask, ring_entries, flags, dropped, array, resv1; uint64_t user_addr; };
struct io_cqring_offsets { uint32_t head, tail, ring_mask, ring_entries, overflow, cqes, flags, resv1; uint64_t user_addr; };
struct io_uring_params {
    uint32_t sq_entries, cq_entries, flags, sq_thread_cpu, sq_thread_idle, features, wq_fd, resv[3];
    struct io_sqring_offsets sq_off;
    struct io_cqring_offsets cq_off;
};

static inline long sys_enter(int fd, uint32_t to_submit, uint32_t min_complete, uint32_t flags) {
    return syscall(__NR_io_uring_enter, fd, to_submit, min_complete, flags, 0, 0);
}

int main(void) {
    // Three write segments of distinct content/length.
    char w0[] = "AAAA", w1[] = "BB", w2[] = "CCCCC";
    char r0[4], r1[2], r2[5];
    memset(r0, '.', sizeof r0); memset(r1, '.', sizeof r1); memset(r2, '.', sizeof r2);

    struct iovec wiov[3] = { {w0, 4}, {w1, 2}, {w2, 5} };   // total 11 bytes
    struct iovec riov[3] = { {r0, 4}, {r1, 2}, {r2, 5} };

    int pfd[2];
    if (pipe(pfd) < 0) { perror("pipe"); return 1; }

    struct io_uring_params par; __builtin_memset(&par, 0, sizeof par);
    int fd = (int)syscall(__NR_io_uring_setup, 8, &par);
    if (fd < 0) { perror("setup"); return 1; }

    size_t page = 4096;
    char *sq_ring = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQ_RING);
    char *cq_ring = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_CQ_RING);
    char *sqes    = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQES);
    if (sq_ring == MAP_FAILED || cq_ring == MAP_FAILED || sqes == MAP_FAILED) { perror("mmap"); return 1; }

    uint32_t *sq_tail  = (uint32_t *)(sq_ring + par.sq_off.tail);
    uint32_t *sq_array = (uint32_t *)(sq_ring + par.sq_off.array);
    uint32_t mask = *(uint32_t *)(sq_ring + par.sq_off.ring_mask);
    uint32_t *cq_tail = (uint32_t *)(cq_ring + par.cq_off.tail);
    uint32_t *cq_head = (uint32_t *)(cq_ring + par.cq_off.head);
    struct io_uring_cqe *cqes = (struct io_uring_cqe *)(cq_ring + par.cq_off.cqes);

    // WRITEV: addr = iovec array, len = iov count (3).
    // READV:  same layout, on the read end.
    uint32_t tail = *sq_tail;
    uint32_t wi = tail & mask, ri = (tail + 1) & mask;
    struct io_uring_sqe *sw = (struct io_uring_sqe *)(sqes + wi * 64);
    __builtin_memset(sw, 0, 64);
    sw->opcode = IORING_OP_WRITEV; sw->fd = pfd[1]; sw->addr = (uint64_t)wiov; sw->len = 3; sw->user_data = 0xCAFE;
    struct io_uring_sqe *sr = (struct io_uring_sqe *)(sqes + ri * 64);
    __builtin_memset(sr, 0, 64);
    sr->opcode = IORING_OP_READV; sr->fd = pfd[0]; sr->addr = (uint64_t)riov; sr->len = 3; sr->user_data = 0xBEEF;
    sq_array[wi] = wi; sq_array[ri] = ri;
    __atomic_store_n(sq_tail, tail + 2, __ATOMIC_RELEASE);

    long n = sys_enter(fd, 2, 2, IORING_ENTER_GETEVENTS);
    if (n < 0) { perror("enter"); return 1; }

    // Reap 2 CQEs (order may vary: WRITEV likely completes before READV).
    int got_w = 0, got_r = 0, wres = -1, rres = -1;
    for (int g = 0; g < 2; g++) {
        uint32_t h, t;
        for (int spin = 0; spin < 1000000; spin++) {
            h = __atomic_load_n(cq_head, __ATOMIC_ACQUIRE);
            t = __atomic_load_n(cq_tail, __ATOMIC_ACQUIRE);
            if (t != h) break;
        }
        if (t == h) { printf("FAIL: only %d/2 CQEs\n", g); return 1; }
        struct io_uring_cqe *cqe = &cqes[h & mask];
        if (cqe->user_data == 0xCAFE) { got_w = 1; wres = cqe->res; }
        else if (cqe->user_data == 0xBEEF) { got_r = 1; rres = cqe->res; }
        __atomic_store_n(cq_head, h + 1, __ATOMIC_RELEASE);
    }

    printf("WRITEV res=%d READV res=%d\n", wres, rres);
    int ok = got_w && got_r && wres == 11 && rres == 11;
    ok = ok && memcmp(w0, r0, 4) == 0 && memcmp(w1, r1, 2) == 0 && memcmp(w2, r2, 5) == 0;
    printf("r0=%.4s r1=%.2s r2=%.5s\n", r0, r1, r2);

    if (!ok) { printf("FAIL: readv/writev content mismatch\n"); return 1; }
    printf("PASS: io_uring READV/WRITEV 3-segment round-trip (11 bytes)\n");
    return 0;
}
