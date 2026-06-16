// io_uring SEND / RECV test (Stage 1 / S1.7).
// Uses an AF_UNIX socketpair (no ACCEPT needed). Verifies:
//   (1) RECV with MSG_DONTWAIT on an empty socket returns -EAGAIN (-11)
//       immediately — proving the readiness-wait is skipped and the serial
//       worker is NOT wedged.
//   (2) SEND to one end + RECV from the other round-trips the bytes.
//
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_send_recv.c -o io_uring_send_recv
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/mman.h>
#include <sys/socket.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426
#define IORING_OFF_SQ_RING 0u
#define IORING_OFF_CQ_RING 0x8000000u
#define IORING_OFF_SQES    0x10000000u
#define IORING_OP_SEND 26
#define IORING_OP_RECV 27
#define IORING_ENTER_GETEVENTS 1u
#define MSG_DONTWAIT 0x40

struct io_uring_sqe {
    uint8_t opcode, flags; uint16_t ioprio; int32_t fd;
    uint64_t off, addr; uint32_t len, rw_flags;   /* rw_flags == msg_flags (union @28) */
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

static inline long sys_enter(int fd, uint32_t to_submit, uint32_t min_complete, uint32_t flags) {
    return syscall(__NR_io_uring_enter, fd, to_submit, min_complete, flags, 0, 0);
}

static int reap_one(uint32_t *cq_head, uint32_t *cq_tail, struct io_uring_cqe *cqes, uint32_t mask) {
    for (int spin = 0; spin < 1000000; spin++) {
        uint32_t h = __atomic_load_n(cq_head, __ATOMIC_ACQUIRE);
        uint32_t t = __atomic_load_n(cq_tail, __ATOMIC_ACQUIRE);
        if (t != h) {
            struct io_uring_cqe *c = &cqes[h & mask];
            int res = c->res;
            __atomic_store_n(cq_head, h + 1, __ATOMIC_RELEASE);
            return res;
        }
    }
    return -999; /* timeout */
}

int main(void) {
    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) < 0) { perror("socketpair"); return 1; }

    struct io_uring_params p; __builtin_memset(&p, 0, sizeof p);
    int fd = (int)syscall(__NR_io_uring_setup, 8, &p);
    if (fd < 0) { perror("setup"); return 1; }
    size_t pg = 4096;
    char *sq_ring = mmap(NULL, pg, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQ_RING);
    char *cq_ring = mmap(NULL, pg, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_CQ_RING);
    char *sqes    = mmap(NULL, pg, PROT_READ | PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQES);
    uint32_t *sq_tail  = (uint32_t *)(sq_ring + p.sq_off.tail);
    uint32_t *sq_array = (uint32_t *)(sq_ring + p.sq_off.array);
    uint32_t mask = *(uint32_t *)(sq_ring + p.sq_off.ring_mask);
    uint32_t *cq_tail = (uint32_t *)(cq_ring + p.cq_off.tail);
    uint32_t *cq_head = (uint32_t *)(cq_ring + p.cq_off.head);
    struct io_uring_cqe *cqes = (struct io_uring_cqe *)(cq_ring + p.cq_off.cqes);

    /* (1) MSG_DONTWAIT RECV on the empty socket -> expect -EAGAIN (-11). */
    char rbuf[16]; memset(rbuf, '.', 16);
    uint32_t tail = *sq_tail, idx = tail & mask;
    struct io_uring_sqe *s = (struct io_uring_sqe *)(sqes + idx * 64);
    __builtin_memset(s, 0, 64);
    s->opcode = IORING_OP_RECV; s->fd = sv[1]; s->addr = (uint64_t)rbuf; s->len = 16;
    s->rw_flags = MSG_DONTWAIT; s->user_data = 0xE1;
    sq_array[idx] = idx;
    __atomic_store_n(sq_tail, tail + 1, __ATOMIC_RELEASE);
    sys_enter(fd, 1, 1, IORING_ENTER_GETEVENTS);
    int eagain_res = reap_one(cq_head, cq_tail, cqes, mask);
    printf("  DONTWAIT RECV (empty): res=%d (want -11)\n", eagain_res);

    /* (2) SEND then RECV round-trip. */
    const char *msg = "hello-uring-net"; /* 15 bytes */
    size_t mlen = strlen(msg);
    char rbuf2[32]; memset(rbuf2, 0, 32);
    tail = *sq_tail;
    uint32_t si = tail & mask, ri = (tail + 1) & mask;
    struct io_uring_sqe *ss = (struct io_uring_sqe *)(sqes + si * 64);
    __builtin_memset(ss, 0, 64);
    ss->opcode = IORING_OP_SEND; ss->fd = sv[0]; ss->addr = (uint64_t)msg; ss->len = mlen; ss->user_data = 0xE2;
    struct io_uring_sqe *sr = (struct io_uring_sqe *)(sqes + ri * 64);
    __builtin_memset(sr, 0, 64);
    sr->opcode = IORING_OP_RECV; sr->fd = sv[1]; sr->addr = (uint64_t)rbuf2; sr->len = 32; sr->user_data = 0xE3;
    sq_array[si] = si; sq_array[ri] = ri;
    __atomic_store_n(sq_tail, tail + 2, __ATOMIC_RELEASE);
    sys_enter(fd, 2, 2, IORING_ENTER_GETEVENTS);
    int r1 = reap_one(cq_head, cq_tail, cqes, mask);
    int r2 = reap_one(cq_head, cq_tail, cqes, mask);
    int send_res = -1, recv_res = -1;
    /* Order may vary; identify by re-deriving: SEND wrote mlen, RECV got the bytes. */
    if (r1 == (int)mlen && r2 == (int)mlen) { send_res = r1; recv_res = r2; }
    else { send_res = r1; recv_res = r2; }
    printf("  SEND res=%d RECV res=%d rbuf2=%.15s\n", send_res, recv_res, rbuf2);

    int ok = (eagain_res == -11)
          && (send_res == (int)mlen) && (recv_res == (int)mlen)
          && (memcmp(msg, rbuf2, mlen) == 0);
    if (!ok) { printf("FAIL: io_uring SEND/RECV\n"); return 1; }
    printf("PASS: io_uring SEND/RECV on socketpair (+ MSG_DONTWAIT -> EAGAIN)\n");
    return 0;
}
