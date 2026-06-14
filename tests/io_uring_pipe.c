// io_uring READ/WRITE on a pipe (Phase 2 / M1).
// Submit a WRITE (known bytes -> pipe write end) then a READ (pipe read end ->
// buffer) through io_uring; verify the bytes round-trip via the worker that
// drives the pipe fd asynchronously. Build:
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_pipe.c -o io_uring_pipe
#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <unistd.h>
#include <string.h>
#include <sched.h>
#include <sys/syscall.h>
#include <sys/mman.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426

#define IORING_OFF_SQ_RING 0u
#define IORING_OFF_CQ_RING 0x8000000u
#define IORING_OFF_SQES    0x10000000u

#define IORING_OP_READ  22
#define IORING_OP_WRITE 23

struct io_uring_sqe {
    uint8_t  opcode; uint8_t flags; uint16_t ioprio; int32_t fd;
    uint64_t off; uint64_t addr; uint32_t len; uint32_t rw_flags;
    uint64_t user_data; uint16_t buf_index; uint16_t personality;
    uint32_t __pad2[3];
} __attribute__((packed));
struct io_uring_cqe { uint64_t user_data; int32_t res; uint32_t flags; };
struct io_sqring_offsets { uint32_t head, tail, ring_mask, ring_entries, flags, dropped, array, resv1; uint64_t user_addr; };
struct io_cqring_offsets { uint32_t head, tail, ring_mask, ring_entries, overflow, cqes, flags, resv1; uint64_t user_addr; };
struct io_uring_params {
    uint32_t sq_entries, cq_entries, flags, sq_thread_cpu, sq_thread_idle;
    uint32_t features, wq_fd, resv[3];
    struct io_sqring_offsets sq_off; struct io_cqring_offsets cq_off;
};

static inline long sys_setup(uint32_t e, struct io_uring_params *p) { return syscall(__NR_io_uring_setup, e, p); }
static inline long sys_enter(int fd, uint32_t n) { return syscall(__NR_io_uring_enter, fd, n, 0, 0, 0, 0); }

int main(void) {
    int pfd[2];
    if (pipe(pfd) < 0) { perror("pipe"); return 1; }
    int pipe_rd = pfd[0], pipe_wr = pfd[1];

    struct io_uring_params p; memset(&p, 0, sizeof p);
    int fd = (int)sys_setup(8, &p);
    if (fd < 0) { perror("io_uring_setup"); return 1; }

    size_t page = 4096;
    char *sq_ring = mmap(NULL, page, PROT_READ|PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQ_RING);
    char *cq_ring = mmap(NULL, page, PROT_READ|PROT_WRITE, MAP_SHARED, fd, IORING_OFF_CQ_RING);
    char *sqes    = mmap(NULL, page, PROT_READ|PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQES);
    if (sq_ring==MAP_FAILED || cq_ring==MAP_FAILED || sqes==MAP_FAILED) { perror("mmap"); return 1; }

    uint32_t *sq_tail = (uint32_t*)(sq_ring + p.sq_off.tail);
    uint32_t *sq_array = (uint32_t*)(sq_ring + p.sq_off.array);
    // NOTE: read the mask *value* from the ring, not the field offset.
    uint32_t mask = *(uint32_t*)(sq_ring + p.sq_off.ring_mask);
    uint32_t *cq_tail = (uint32_t*)(cq_ring + p.cq_off.tail);
    uint32_t *cq_head = (uint32_t*)(cq_ring + p.cq_off.head);
    struct io_uring_cqe *cqes = (struct io_uring_cqe*)(cq_ring + p.cq_off.cqes);
    uint32_t cqmask = *(uint32_t*)(cq_ring + p.cq_off.ring_mask);

    enum { N = 4096 };
    static unsigned char wbuf[N], rbuf[N];
    for (int i = 0; i < N; i++) wbuf[i] = (unsigned char)(i * 7 + 3);
    memset(rbuf, 0, N);

    // SQE[0] = WRITE (wbuf -> pipe_wr), SQE[1] = READ (pipe_rd -> rbuf).
    uint32_t tail = *sq_tail;
    struct io_uring_sqe *s0 = (struct io_uring_sqe*)(sqes + (tail & mask) * 64);
    memset(s0, 0, 64);
    s0->opcode = IORING_OP_WRITE; s0->fd = pipe_wr; s0->addr = (uint64_t)wbuf; s0->len = N; s0->user_data = 0xA1;
    sq_array[tail & mask] = tail & mask;

    struct io_uring_sqe *s1 = (struct io_uring_sqe*)(sqes + ((tail + 1) & mask) * 64);
    memset(s1, 0, 64);
    s1->opcode = IORING_OP_READ; s1->fd = pipe_rd; s1->addr = (uint64_t)rbuf; s1->len = N; s1->user_data = 0xA2;
    sq_array[(tail + 1) & mask] = (tail + 1) & mask;

    __atomic_store_n(sq_tail, tail + 2, __ATOMIC_RELEASE);
    if (sys_enter(fd, 2) < 0) { perror("io_uring_enter"); return 1; }

    // Reap 2 CQEs.
    int got_wr = -1, got_rd = -1;
    for (int want = 0; want < 2; want++) {
        for (;;) {
            uint32_t h = __atomic_load_n(cq_head, __ATOMIC_ACQUIRE);
            uint32_t t = __atomic_load_n(cq_tail, __ATOMIC_ACQUIRE);
            if (t != h) {
                struct io_uring_cqe *cqe = &cqes[h & cqmask];
                printf("CQE: user_data=0x%lx res=%d\n", (unsigned long)cqe->user_data, cqe->res);
                if (cqe->user_data == 0xA1) got_wr = cqe->res;
                if (cqe->user_data == 0xA2) got_rd = cqe->res;
                __atomic_store_n(cq_head, h + 1, __ATOMIC_RELEASE);
                break;
            }
            sched_yield(); // SMP=1: let the global worker run to post the CQE
        }
    }

    int ok = (got_wr == N) && (got_rd == N) && (memcmp(wbuf, rbuf, N) == 0);
    printf("WRITE res=%d  READ res=%d  bytes_match=%d\n", got_wr, got_rd, memcmp(wbuf, rbuf, N) == 0);
    if (ok) { printf("PASS: io_uring READ/WRITE pipe round-trip (%d bytes)\n", N); return 0; }
    printf("FAIL: io_uring READ/WRITE\n"); return 1;
}
