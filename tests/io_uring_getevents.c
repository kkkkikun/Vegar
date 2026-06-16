// io_uring GETEVENTS test (Stage 1 / S1.1).
// Verifies io_uring_enter(IORING_ENTER_GETEVENTS, min_complete=N) BLOCKS until
// N CQEs are available — the path liburing's io_uring_submit_and_wait /
// io_uring_wait_cqe depend on. Before S1.1, enter ignored GETEVENTS and returned
// immediately with 0 completions.
//
// We submit 4 NOPs, then enter with min_complete=4 + GETEVENTS. Immediately on
// return we read the CQ (no polling loop) — if GETEVENTS worked, all 4 CQEs are
// guaranteed present; if it didn't wait, the CQ would be empty/incomplete.
//
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_getevents.c -o io_uring_getevents
#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/mman.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426

#define IORING_OFF_SQ_RING 0u
#define IORING_OFF_CQ_RING 0x8000000u
#define IORING_OFF_SQES    0x10000000u

#define IORING_OP_NOP 0
#define IORING_ENTER_GETEVENTS 1u

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
    enum { N = 4 };
    struct io_uring_params p; __builtin_memset(&p, 0, sizeof p);
    int fd = (int)syscall(__NR_io_uring_setup, 8, &p);
    if (fd < 0) { perror("setup"); return 1; }

    size_t page = 4096;
    char *sq_ring = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQ_RING);
    char *cq_ring = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_CQ_RING);
    char *sqes    = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQES);
    if (sq_ring == MAP_FAILED || cq_ring == MAP_FAILED || sqes == MAP_FAILED) { perror("mmap"); return 1; }

    uint32_t *sq_tail  = (uint32_t *)(sq_ring + p.sq_off.tail);
    uint32_t *sq_array = (uint32_t *)(sq_ring + p.sq_off.array);
    // CORRECT mask: read the value stored at the ring_mask offset (not the offset itself).
    uint32_t mask = *(uint32_t *)(sq_ring + p.sq_off.ring_mask);

    uint32_t *cq_tail = (uint32_t *)(cq_ring + p.cq_off.tail);
    uint32_t *cq_head = (uint32_t *)(cq_ring + p.cq_off.head);
    struct io_uring_cqe *cqes = (struct io_uring_cqe *)(cq_ring + p.cq_off.cqes);

    // Submit N NOP SQEs.
    uint32_t tail = *sq_tail;
    for (int i = 0; i < N; i++) {
        uint32_t idx = (tail + i) & mask;
        struct io_uring_sqe *s = (struct io_uring_sqe *)(sqes + idx * 64);
        __builtin_memset(s, 0, 64);
        s->opcode = IORING_OP_NOP;
        s->user_data = 0xA000 + i;
        sq_array[idx] = idx;
    }
    __atomic_store_n(sq_tail, tail + N, __ATOMIC_RELEASE);

    // The decisive call: enter with GETEVENTS + min_complete=N.
    // Without GETEVENTS handling this returns instantly with the CQ empty.
    long n = sys_enter(fd, N, N, IORING_ENTER_GETEVENTS);
    if (n < 0) { perror("enter"); return 1; }

    // Immediately (no polling) the CQ must hold >= N completions.
    uint32_t h = __atomic_load_n(cq_head, __ATOMIC_ACQUIRE);
    uint32_t t = __atomic_load_n(cq_tail, __ATOMIC_ACQUIRE);
    uint32_t ready = t - h;
    printf("enter returned %ld submitted; CQ ready=%u (want %d)\n", n, ready, N);

    int ok = (ready >= N);
    for (int i = 0; i < N && ok; i++) {
        struct io_uring_cqe *cqe = &cqes[(h + i) & mask];
        if (cqe->user_data != 0xA000 + i || cqe->res != 0) {
            printf("CQE[%d] mismatch: user_data=0x%lx res=%d\n",
                   i, (unsigned long)cqe->user_data, cqe->res);
            ok = 0;
        }
    }
    if (ready < N) { printf("FAIL: GETEVENTS did not wait for %d completions\n", N); return 1; }
    if (!ok) { printf("FAIL: CQE contents wrong\n"); return 1; }
    printf("PASS: io_uring GETEVENTS waited for %d completions\n", N);
    return 0;
}
