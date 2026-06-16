// io_uring shim test (Stage 1 / S1.4).
// Exercises the minimal liburing shim (tests/liburing-shim/) against the
// kernel: a 4-NOP batch via submit_and_wait + a READV/WRITEV pipe round-trip.
// Proves the shim correctly wraps io_uring_setup/enter + mmap + the GETEVENTS
// wait path, so third-party liburing code (perf-test, echo server) can link.
//
//   riscv64-linux-musl-gcc -static -O2 -Itests/liburing-shim \
//       tests/io_uring_shim_test.c tests/liburing-shim/liburing_shim.c \
//       -o io_uring_shim_test
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include "liburing_shim.h"

static int do_nops(struct io_uring *ring) {
    enum { N = 4 };
    for (int i = 0; i < N; i++) {
        struct io_uring_sqe *sqe = io_uring_get_sqe(ring);
        if (!sqe) { printf("FAIL: get_sqe\n"); return 1; }
        io_uring_prep_rw(sqe, IORING_OP_NOP, -1, NULL, 0, 0);
        sqe->user_data = 0xD000 + i;
    }
    int r = io_uring_submit_and_wait(ring, N);
    if (r < 0) { printf("FAIL: submit_and_wait=%d\n", r); return 1; }

    struct io_uring_cqe *cqe;
    for (int i = 0; i < N; i++) {
        if (io_uring_wait_cqe(ring, &cqe) < 0 || !cqe) {
            printf("FAIL: wait_cqe %d\n", i); return 1;
        }
        if (cqe->res != 0) { printf("FAIL: NOP res=%d\n", cqe->res); return 1; }
        io_uring_cqe_seen(ring, cqe);
    }
    printf("  shim NOP batch: %d CQEs reaped via wait_cqe OK\n", N);
    return 0;
}

static int do_readv(struct io_uring *ring) {
    char w0[] = "HELLO ", w1[] = "WORLD ", w2[] = "12345";
    char r0[6], r1[6], r2[6];
    memset(r0, '.', 6); memset(r1, '.', 6); memset(r2, '.', 6);

    struct iovec wiov[3] = { {w0, 6}, {w1, 6}, {w2, 5} };  // 17 bytes
    struct iovec riov[3] = { {r0, 6}, {r1, 6}, {r2, 5} };

    int pfd[2];
    if (pipe(pfd) < 0) { perror("pipe"); return 1; }

    struct io_uring_sqe *sw = io_uring_get_sqe(ring);
    struct io_uring_sqe *sr = io_uring_get_sqe(ring);
    io_uring_prep_writev(sw, pfd[1], wiov, 3, 0); sw->user_data = 0x1;
    io_uring_prep_readv (sr, pfd[0], riov, 3, 0); sr->user_data = 0x2;

    int r = io_uring_submit_and_wait(ring, 2);
    if (r < 0) { printf("FAIL: readv submit_and_wait=%d\n", r); return 1; }

    int wr = -1, rd = -1;
    struct io_uring_cqe *cqe;
    for (int i = 0; i < 2; i++) {
        if (io_uring_wait_cqe(ring, &cqe) < 0) { printf("FAIL: wait\n"); return 1; }
        if (cqe->user_data == 0x1) wr = cqe->res; else rd = cqe->res;
        io_uring_cqe_seen(ring, cqe);
    }
    int ok = (wr == 17 && rd == 17 &&
              memcmp(w0, r0, 6) == 0 && memcmp(w1, r1, 6) == 0 && memcmp(w2, r2, 5) == 0);
    printf("  shim READV/WRITEV: write=%d read=%d r0=%.6s r1=%.6s r2=%.5s\n",
           wr, rd, r0, r1, r2);
    return ok ? 0 : 1;
}

int main(void) {
    struct io_uring ring;
    if (io_uring_queue_init(8, &ring, 0) < 0) { perror("queue_init"); return 1; }
    printf("=== io_uring shim test ===\n");

    int rc = do_nops(&ring);
    if (rc == 0) rc = do_readv(&ring);

    io_uring_queue_exit(&ring);
    if (rc) { printf("FAIL: io_uring shim\n"); return 1; }
    printf("PASS: io_uring shim (queue_init/get_sqe/submit_and_wait/wait_cqe/readv)\n");
    return 0;
}
