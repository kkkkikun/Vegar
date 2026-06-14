// io_uring POLL_ADD on a pipe (Phase 2 / M2).
// Submit POLL_ADD on the pipe read-end for POLLIN; the worker parks until the
// fd is readable. We then write to the write-end, which makes the read-end
// readable -> the worker wakes and posts a CQE carrying the ready event mask.
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_poll.c -o io_uring_poll
#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <unistd.h>
#include <sched.h>
#include <poll.h>
#include <sys/syscall.h>
#include <sys/mman.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426
#define IORING_OP_POLL_ADD 6

struct io_uring_sqe {
    uint8_t opcode; uint8_t flags; uint16_t ioprio; int32_t fd;
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

int main(void) {
    int pfd[2];
    if (pipe(pfd) < 0) { perror("pipe"); return 1; }
    int rd = pfd[0], wr = pfd[1];

    struct io_uring_params p; __builtin_memset(&p, 0, sizeof p);
    int fd = (int)syscall(__NR_io_uring_setup, 8, &p);
    if (fd < 0) { perror("io_uring_setup"); return 1; }

    size_t page = 4096;
    char *sq_ring = mmap(NULL, page, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0u);
    char *cq_ring = mmap(NULL, page, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0x8000000u);
    char *sqes    = mmap(NULL, page, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0x10000000u);
    if (sq_ring==MAP_FAILED || cq_ring==MAP_FAILED || sqes==MAP_FAILED) { perror("mmap"); return 1; }

    uint32_t *sq_tail = (uint32_t*)(sq_ring + p.sq_off.tail);
    uint32_t *sq_array = (uint32_t*)(sq_ring + p.sq_off.array);
    uint32_t mask = *(uint32_t*)(sq_ring + p.sq_off.ring_mask);
    uint32_t *cq_tail = (uint32_t*)(cq_ring + p.cq_off.tail);
    uint32_t *cq_head = (uint32_t*)(cq_ring + p.cq_off.head);
    struct io_uring_cqe *cqes = (struct io_uring_cqe*)(cq_ring + p.cq_off.cqes);
    uint32_t cqmask = *(uint32_t*)(cq_ring + p.cq_off.ring_mask);

    // Submit POLL_ADD on the read-end for POLLIN.
    uint32_t tail = *sq_tail;
    struct io_uring_sqe *s = (struct io_uring_sqe*)(sqes + (tail & mask) * 64);
    __builtin_memset(s, 0, 64);
    s->opcode = IORING_OP_POLL_ADD;
    s->fd = rd;
    s->rw_flags = POLLIN;          // poll mask lives in the same union (offset 28)
    s->user_data = 0xB1;
    sq_array[tail & mask] = tail & mask;
    __atomic_store_n(sq_tail, tail + 1, __ATOMIC_RELEASE);

    if (syscall(__NR_io_uring_enter, fd, 1, 0, 0, 0, 0) < 0) { perror("io_uring_enter"); return 1; }

    // Pipe is empty -> POLL_ADD is parked. Write one byte to make it readable.
    char c = 'X';
    if (write(wr, &c, 1) != 1) { perror("write"); return 1; }

    // Reap the POLL_ADD CQE.
    int got = -1;
    for (;;) {
        uint32_t h = __atomic_load_n(cq_head, __ATOMIC_ACQUIRE);
        uint32_t t = __atomic_load_n(cq_tail, __ATOMIC_ACQUIRE);
        if (t != h) {
            struct io_uring_cqe *cqe = &cqes[h & cqmask];
            got = cqe->res;
            printf("CQE: user_data=0x%lx res=0x%x\n", (unsigned long)cqe->user_data, got);
            __atomic_store_n(cq_head, h + 1, __ATOMIC_RELEASE);
            break;
        }
        sched_yield();
    }

    int ok = (got & POLLIN) != 0;
    if (ok) { printf("PASS: io_uring POLL_ADD woke on POLLIN (res=0x%x)\n", got); return 0; }
    printf("FAIL: io_uring POLL_ADD (res=0x%x)\n", got); return 1;
}
