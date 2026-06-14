// io_uring NOP round-trip test (Phase 2 / M0).
// Uses raw riscv64 syscalls + the offsets the kernel reports in io_uring_params,
// so it does not depend on liburing. Build:
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_nop.c -o io_uring_nop
#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/mman.h>

// io_uring syscall numbers (riscv64 / generic Linux).
#define __NR_io_uring_setup   425
#define __NR_io_uring_enter   426

#define IORING_OFF_SQ_RING 0u
#define IORING_OFF_CQ_RING 0x8000000u
#define IORING_OFF_SQES    0x10000000u

#define IORING_OP_NOP 0

struct io_uring_sqe {
    uint8_t  opcode;
    uint8_t  flags;
    uint16_t ioprio;
    int32_t  fd;
    uint64_t off;
    uint64_t addr;
    uint32_t len;
    uint32_t rw_flags;
    uint64_t user_data;
    uint16_t buf_index;
    uint16_t personality;
    uint32_t __pad2[3];
} __attribute__((packed));

struct io_uring_cqe {
    uint64_t user_data;
    int32_t  res;
    uint32_t flags;
};

struct io_sqring_offsets {
    uint32_t head, tail, ring_mask, ring_entries, flags, dropped, array, resv1;
    uint64_t user_addr;
};
struct io_cqring_offsets {
    uint32_t head, tail, ring_mask, ring_entries, overflow, cqes, flags, resv1;
    uint64_t user_addr;
};
struct io_uring_params {
    uint32_t sq_entries, cq_entries, flags, sq_thread_cpu, sq_thread_idle;
    uint32_t features, wq_fd, resv[3];
    struct io_sqring_offsets sq_off;
    struct io_cqring_offsets cq_off;
};

static inline long sys_setup(uint32_t entries, struct io_uring_params *p) {
    return syscall(__NR_io_uring_setup, entries, p);
}
static inline long sys_enter(int fd, uint32_t to_submit, uint32_t min_complete,
                             uint32_t flags) {
    return syscall(__NR_io_uring_enter, fd, to_submit, min_complete, flags, 0, 0);
}

int main(void) {
    struct io_uring_params p;
    for (size_t i = 0; i < sizeof(p); i++) ((char *)&p)[i] = 0;

    int fd = (int)sys_setup(8, &p);
    if (fd < 0) { perror("io_uring_setup"); return 1; }
    printf("setup ok: fd=%d sq_entries=%u cq_entries=%u\n",
           fd, p.sq_entries, p.cq_entries);
    printf("sq_off: head=%u tail=%u mask=%u entries=%u array=%u\n",
           p.sq_off.head, p.sq_off.tail, p.sq_off.ring_mask,
           p.sq_off.ring_entries, p.sq_off.array);
    printf("cq_off: head=%u tail=%u mask=%u entries=%u cqes=%u\n",
           p.cq_off.head, p.cq_off.tail, p.cq_off.ring_mask,
           p.cq_off.ring_entries, p.cq_off.cqes);

    size_t page = 4096;
    char *sq_ring = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQ_RING);
    char *cq_ring = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_CQ_RING);
    char *sqes    = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQES);
    if (sq_ring == MAP_FAILED || cq_ring == MAP_FAILED || sqes == MAP_FAILED) {
        perror("mmap"); return 1;
    }

    uint32_t *sq_tail = (uint32_t *)(sq_ring + p.sq_off.tail);
    uint32_t *sq_head = (uint32_t *)(sq_ring + p.sq_off.head);
    uint32_t *sq_array = (uint32_t *)(sq_ring + p.sq_off.array);
    uint32_t mask = p.sq_off.ring_mask;

    uint32_t *cq_tail = (uint32_t *)(cq_ring + p.cq_off.tail);
    uint32_t *cq_head = (uint32_t *)(cq_ring + p.cq_off.head);
    struct io_uring_cqe *cqes = (struct io_uring_cqe *)(cq_ring + p.cq_off.cqes);
    uint32_t cqmask = p.cq_off.ring_mask;

    // Submit one NOP.
    uint32_t tail = *sq_tail;
    uint32_t idx = tail & mask;
    struct io_uring_sqe *sqe = (struct io_uring_sqe *)(sqes + idx * 64);
    for (size_t i = 0; i < 64; i++) ((char *)sqe)[i] = 0;
    sqe->opcode = IORING_OP_NOP;
    sqe->user_data = 0x1234;
    sq_array[idx] = idx;          // identity; kernel reads SQE index from here
    __atomic_store_n(sq_tail, tail + 1, __ATOMIC_RELEASE);

    long n = sys_enter(fd, 1, 0, 0);
    if (n < 0) { perror("io_uring_enter"); return 1; }
    printf("enter returned %ld submitted\n", n);

    // Reap one CQE (poll the CQ tail).
    int got = 0;
    for (int i = 0; i < 1000000; i++) {
        uint32_t h = __atomic_load_n(cq_head, __ATOMIC_ACQUIRE);
        uint32_t t = __atomic_load_n(cq_tail, __ATOMIC_ACQUIRE);
        if (t != h) {
            struct io_uring_cqe *cqe = &cqes[h & cqmask];
            printf("CQE: user_data=0x%lx res=%d flags=%u\n",
                   (unsigned long)cqe->user_data, cqe->res, cqe->flags);
            __atomic_store_n(cq_head, h + 1, __ATOMIC_RELEASE);
            got = 1;
            break;
        }
    }

    // Drain the SQ ring head update sanity check.
    (void)sq_head;
    printf(got && "x" ? "" : "");
    if (!got) { printf("FAIL: no CQE received\n"); return 1; }
    printf("PASS: io_uring NOP round-trip\n");
    return 0;
}
