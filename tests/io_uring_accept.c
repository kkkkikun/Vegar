// io_uring ACCEPT + loopback TCP echo test (Stage 1 / S1.6 + S1.8 core).
// Parent listens on 127.0.0.1, forks a client that connects + sends "ping" +
// reads the echo. Parent drives the whole server side through io_uring:
// ACCEPT (get client fd) -> RECV "ping" -> SEND "pong". Exercises the
// submitter-context ACCEPT path (complete_accept: poll-wait IN, accept(),
// install fd, post CQE res=fd).
//
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_accept.c -o io_uring_accept
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <sys/syscall.h>
#include <sys/mman.h>
#include <sys/wait.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426
#define IORING_OFF_SQ_RING 0u
#define IORING_OFF_CQ_RING 0x8000000u
#define IORING_OFF_SQES    0x10000000u
#define IORING_OP_ACCEPT 13
#define IORING_OP_RECV 27
#define IORING_OP_SEND 26
#define IORING_ENTER_GETEVENTS 1u
#define PORT 7321u

struct io_uring_sqe {
    uint8_t opcode, flags; uint16_t ioprio; int32_t fd;
    uint64_t off, addr; uint32_t len, rw_flags;
    uint64_t user_data; uint16_t buf_index, personality; uint32_t __pad2[3];
} __attribute__((packed));
struct io_uring_cqe { uint64_t user_data; int32_t res; uint32_t flags; };
struct io_sqring_offsets { uint32_t head, tail, ring_mask, ring_entries, flags, dropped, array, resv1; uint64_t resv2; };
struct io_cqring_offsets { uint32_t head, tail, ring_mask, ring_entries, overflow, cqes, flags, resv1; uint64_t resv2; };
struct io_uring_params {
    uint32_t sq_entries, cq_entries, flags, sq_thread_cpu, sq_thread_idle, features, wq_fd, resv[3];
    struct io_sqring_offsets sq_off;
    struct io_cqring_offsets cq_off;
};

static int reap(uint32_t *cq_head, uint32_t *cq_tail, struct io_uring_cqe *cqes, uint32_t mask) {
    for (int spin = 0; spin < 2000000; spin++) {
        uint32_t h = __atomic_load_n(cq_head, __ATOMIC_ACQUIRE);
        uint32_t t = __atomic_load_n(cq_tail, __ATOMIC_ACQUIRE);
        if (t != h) {
            struct io_uring_cqe *c = &cqes[h & mask];
            int res = c->res;
            __atomic_store_n(cq_head, h + 1, __ATOMIC_RELEASE);
            return res;
        }
    }
    return -999;
}

static void submit_one(char *sqes, uint32_t *sq_tail, uint32_t *sq_array, uint32_t mask,
                       uint8_t op, int sfd, void *addr, uint32_t len, uint64_t ud) {
    uint32_t tail = *sq_tail, idx = tail & mask;
    struct io_uring_sqe *s = (struct io_uring_sqe *)(sqes + idx * 64);
    __builtin_memset(s, 0, 64);
    s->opcode = op; s->fd = sfd; s->addr = (uint64_t)addr; s->len = len; s->user_data = ud;
    sq_array[idx] = idx;
    __atomic_store_n(sq_tail, tail + 1, __ATOMIC_RELEASE);
}

int main(void) {
    int lfd = socket(AF_INET, SOCK_STREAM, 0);
    if (lfd < 0) { perror("socket"); return 1; }
    struct sockaddr_in addr; memset(&addr, 0, sizeof addr);
    addr.sin_family = AF_INET;
    addr.sin_port = htons(PORT);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int opt = 1; setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof opt);
    if (bind(lfd, (struct sockaddr *)&addr, sizeof addr) < 0) { perror("bind"); return 1; }
    if (listen(lfd, 4) < 0) { perror("listen"); return 1; }
    printf("listening on 127.0.0.1:%d (lfd=%d)\n", PORT, lfd);

    struct io_uring_params p; __builtin_memset(&p, 0, sizeof p);
    int ringfd = (int)syscall(__NR_io_uring_setup, 8, &p);
    if (ringfd < 0) { perror("setup"); return 1; }
    size_t pg = 4096;
    char *sq_ring = mmap(NULL, pg, PROT_READ | PROT_WRITE, MAP_SHARED, ringfd, IORING_OFF_SQ_RING);
    char *cq_ring = mmap(NULL, pg, PROT_READ | PROT_WRITE, MAP_SHARED, ringfd, IORING_OFF_CQ_RING);
    char *sqes    = mmap(NULL, pg, PROT_READ | PROT_WRITE, MAP_SHARED, ringfd, IORING_OFF_SQES);
    uint32_t *sq_tail  = (uint32_t *)(sq_ring + p.sq_off.tail);
    uint32_t *sq_array = (uint32_t *)(sq_ring + p.sq_off.array);
    uint32_t mask = *(uint32_t *)(sq_ring + p.sq_off.ring_mask);
    uint32_t *cq_tail = (uint32_t *)(cq_ring + p.cq_off.tail);
    uint32_t *cq_head = (uint32_t *)(cq_ring + p.cq_off.head);
    struct io_uring_cqe *cqes = (struct io_uring_cqe *)(cq_ring + p.cq_off.cqes);

    /* Fork a client: connect, send "ping", read echo, exit 0 if got "pong". */
    pid_t pid = fork();
    if (pid == 0) {
        int cfd = socket(AF_INET, SOCK_STREAM, 0);
        struct sockaddr_in sa; memset(&sa, 0, sizeof sa);
        sa.sin_family = AF_INET; sa.sin_port = htons(PORT); sa.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        int rc = -1;
        for (int i = 0; i < 2000 && rc != 0; i++) {
            rc = connect(cfd, (struct sockaddr *)&sa, sizeof sa);
            if (rc != 0) usleep(1000);
        }
        if (rc != 0) _exit(2);
        write(cfd, "ping", 4);
        char rb[8]; int n = read(cfd, rb, 4);
        int ok = (n == 4 && memcmp(rb, "pong", 4) == 0);
        close(cfd);
        _exit(ok ? 0 : 1);
    }

    /* ACCEPT via io_uring (runs in submitter context, parks until connect). */
    submit_one(sqes, sq_tail, sq_array, mask, IORING_OP_ACCEPT, lfd, NULL, 0, 0xA1);
    syscall(__NR_io_uring_enter, ringfd, 1, 1, IORING_ENTER_GETEVENTS, 0, 0);
    int cfd = reap(cq_head, cq_tail, cqes, mask);
    printf("  ACCEPT res=%d\n", cfd);
    if (cfd < 0) { printf("FAIL: accept cqe=%d\n", cfd); return 1; }

    /* RECV "ping". */
    char rbuf[8]; memset(rbuf, 0, 8);
    submit_one(sqes, sq_tail, sq_array, mask, IORING_OP_RECV, cfd, rbuf, 4, 0xA2);
    syscall(__NR_io_uring_enter, ringfd, 1, 1, IORING_ENTER_GETEVENTS, 0, 0);
    int rr = reap(cq_head, cq_tail, cqes, mask);
    printf("  RECV res=%d rbuf=%.4s\n", rr, rbuf);

    /* SEND "pong". */
    submit_one(sqes, sq_tail, sq_array, mask, IORING_OP_SEND, cfd, "pong", 4, 0xA3);
    syscall(__NR_io_uring_enter, ringfd, 1, 1, IORING_ENTER_GETEVENTS, 0, 0);
    int ss = reap(cq_head, cq_tail, cqes, mask);
    printf("  SEND res=%d\n", ss);
    close(cfd);

    int status; waitpid(pid, &status, 0);
    int child_ok = (WIFEXITED(status) && WEXITSTATUS(status) == 0);
    int ok = (cfd > 0) && (rr == 4) && (ss == 4) && (memcmp(rbuf, "ping", 4) == 0) && child_ok;
    if (!ok) { printf("FAIL: io_uring ACCEPT echo (child_ok=%d status=%d)\n", child_ok, status); return 1; }
    printf("PASS: io_uring ACCEPT + RECV + SEND loopback TCP echo\n");
    return 0;
}
