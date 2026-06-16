// io_uring iodepth-scaling benchmark via the liburing shim (Stage 1 / S1.5).
// Registered buffers + READ_FIXED on /dev/zero, swept over batch size D=1..32.
// Submits D ops in ONE io_uring_enter (submit_and_wait) vs D separate read()
// syscalls — the structural O(1)-vs-O(D) syscall batching, measured in cycles.
//
//   riscv64-linux-musl-gcc -static -O2 -Itests/liburing-shim \
//       tests/io_uring_iodepth.c tests/liburing-shim/liburing_shim.c -o io_uring_iodepth
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include "liburing_shim.h"

#define BUF 4096
#define MAXD 32

static inline uint64_t rdtime(void) { uint64_t v; asm volatile("rdtime %0" : "=r"(v)); return v; }

int main(void) {
    /* Registered buffer pool: MAXD x 4KB. */
    char *pool = mmap(NULL, BUF * MAXD, PROT_READ | PROT_WRITE,
                      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (pool == MAP_FAILED) { perror("mmap"); return 1; }
    for (int i = 0; i < MAXD * BUF; i++) pool[i] = 0; /* populate (avoid CoW-pending pin fail) */

    struct iovec iovs[MAXD];
    for (int i = 0; i < MAXD; i++) { iovs[i].iov_base = pool + i * BUF; iovs[i].iov_len = BUF; }

    int zfd = open("/dev/zero", O_RDONLY);
    if (zfd < 0) { perror("open /dev/zero"); return 1; }

    struct io_uring ring;
    if (io_uring_queue_init(64, &ring, 0) < 0) { printf("FAIL: queue_init\n"); return 1; }
    if (io_uring_register_buffers(&ring, iovs, MAXD) < 0) {
        printf("FAIL: register_buffers\n"); return 1;
    }

    printf("=== iodepth scaling: io_uring (1 enter, registered bufs) vs sync read() ===\n");
    printf("%-8s %14s %12s %14s %12s\n", "D", "iouring cyc", "cyc/op", "sync cyc", "cyc/op");

    for (int D = 1; D <= MAXD; D = D * 2) {
        /* --- io_uring: submit D READ_FIXED, submit_and_wait(D), reap D --- */
        for (int i = 0; i < D; i++) {
            struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
            sqe->opcode = IORING_OP_READ_FIXED;
            sqe->fd = zfd;
            sqe->addr = (uint64_t)(pool + i * BUF);
            sqe->len = BUF;
            sqe->buf_index = i;
            sqe->user_data = i;
        }
        uint64_t t0 = rdtime();
        io_uring_submit_and_wait(&ring, D);
        struct io_uring_cqe *cqe;
        int bytes = 0;
        for (int i = 0; i < D; i++) {
            io_uring_wait_cqe(&ring, &cqe);
            bytes += cqe->res > 0 ? cqe->res : 0;
            io_uring_cqe_seen(&ring, cqe);
        }
        uint64_t t_ring = rdtime() - t0;

        /* --- sync: D x read() into stack buffers --- */
        char tmp[BUF];
        t0 = rdtime();
        int sbytes = 0;
        for (int i = 0; i < D; i++) { int n = read(zfd, tmp, BUF); sbytes += n > 0 ? n : 0; }
        uint64_t t_sync = rdtime() - t0;

        (void)bytes; (void)sbytes;
        printf("%-8d %14llu %12llu %14llu %12llu\n", D,
               (unsigned long long)t_ring, (unsigned long long)(t_ring / D),
               (unsigned long long)t_sync, (unsigned long long)(t_sync / D));
    }

    io_uring_queue_exit(&ring);
    printf("PASS: io_uring iodepth scaling bench (registered buffers + READ_FIXED)\n");
    return 0;
}
