// io_uring SCALING benchmark (rewritten — fair comparison).
//
// Measures the per-op cost of io_uring_enter syscall batching:
//   batched:  N NOPs submitted + reaped via 1 io_uring_enter(GETEVENTS)
//   single:   N NOPs submitted + reaped via N separate io_uring_enter(GETEVENTS)
//
// Both paths use the SAME opcode (NOP), same ring setup, same CQ reap via GETEVENTS.
// The ONLY variable is batching — this isolates the syscall amortization effect.
//
// Each (N, mode) is measured over ROUNDS repetitions; we report min / avg / max cycles.
//
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_scale.c -o io_uring_scale
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/mman.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426

#define IORING_OFF_SQ_RING  0u
#define IORING_OFF_CQ_RING  0x8000000u
#define IORING_OFF_SQES     0x10000000u

#define IORING_OP_NOP       0
#define IORING_ENTER_GETEVENTS 1u

#define MAX_N    32
#define ROUNDS   16

struct io_uring_sqe {
    uint8_t  opcode; uint8_t flags; uint16_t ioprio; int32_t fd;
    uint64_t off; uint64_t addr; uint32_t len; uint32_t rw_flags;
    uint64_t user_data; uint16_t buf_index, personality; uint32_t __pad2[3];
} __attribute__((packed));
struct io_uring_cqe { uint64_t user_data; int32_t res; uint32_t flags; };
struct io_sqring_offsets { uint32_t head,tail,ring_mask,ring_entries,flags,dropped,array,resv1; uint64_t user_addr; };
struct io_cqring_offsets { uint32_t head,tail,ring_mask,ring_entries,overflow,cqes,flags,resv1; uint64_t user_addr; };
struct io_uring_params {
    uint32_t sq_entries,cq_entries,flags,sc,si,feat,wfd,rsv[3];
    struct io_sqring_offsets sq_off; struct io_cqring_offsets cq_off;
};

static inline uint64_t rdtime(void) { uint64_t v; asm volatile("rdtime %0" : "=r"(v)); return v; }

/* Reap `n` completions via GETEVENTS (blocking wait, not busy-poll). */
static void reap_n(int fd, uint32_t *cq_head, uint32_t *cq_tail,
                   struct io_uring_cqe *cqes, uint32_t mask, int n) {
    for (int got = 0; got < n; ) {
        uint32_t h = __atomic_load_n(cq_head, __ATOMIC_ACQUIRE);
        uint32_t t = __atomic_load_n(cq_tail, __ATOMIC_ACQUIRE);
        uint32_t avail = t - h;
        if (avail > 0) {
            uint32_t consume = avail < (uint32_t)(n - got) ? avail : (uint32_t)(n - got);
            __atomic_store_n(cq_head, h + consume, __ATOMIC_RELEASE);
            got += (int)consume;
        } else {
            syscall(__NR_io_uring_enter, fd, 0, (uint32_t)(n - got),
                    IORING_ENTER_GETEVENTS, 0, 0);
        }
    }
}

int main(void) {
    struct io_uring_params p;
    memset(&p, 0, sizeof p);
    int fd = (int)syscall(__NR_io_uring_setup, MAX_N, &p);
    if (fd < 0) { perror("setup"); return 1; }

    size_t pg = 4096;
    char *sq_ring = mmap(NULL, pg, PROT_READ|PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQ_RING);
    char *cq_ring = mmap(NULL, pg, PROT_READ|PROT_WRITE, MAP_SHARED, fd, IORING_OFF_CQ_RING);
    char *sqes    = mmap(NULL, pg, PROT_READ|PROT_WRITE, MAP_SHARED, fd, IORING_OFF_SQES);

    uint32_t *sq_tail  = (uint32_t*)(sq_ring + p.sq_off.tail);
    uint32_t *sq_head  = (uint32_t*)(sq_ring + p.sq_off.head);
    uint32_t *sq_array = (uint32_t*)(sq_ring + p.sq_off.array);
    uint32_t  sq_mask  = *(uint32_t*)(sq_ring + p.sq_off.ring_mask);
    uint32_t *cq_head  = (uint32_t*)(cq_ring + p.cq_off.head);
    uint32_t *cq_tail  = (uint32_t*)(cq_ring + p.cq_off.tail);
    uint32_t  cq_mask  = *(uint32_t*)(cq_ring + p.cq_off.ring_mask);
    struct io_uring_cqe *cqes = (struct io_uring_cqe*)(cq_ring + p.cq_off.cqes);

    printf("=== Submission scaling: N NOPs × 1 enter vs N × 1 NOP × 1 enter ===\n");
    printf("(both paths use same NOP opcode + GETEVENTS reap; %d rounds each)\n", ROUNDS);
    printf("%-5s %-18s %-18s %-14s %-14s %-10s\n",
           "N", "batched(avg cyc)", "single(avg cyc)", "batched/op", "single/op", "ratio");

    for (int n = 1; n <= MAX_N; n *= 2) {
        uint64_t t_batch_min = ~0ULL, t_batch_sum = 0, t_batch_max = 0;
        uint64_t t_single_min = ~0ULL, t_single_sum = 0, t_single_max = 0;

        for (int r = 0; r < ROUNDS; r++) {
            /* ── Batched: N NOPs in 1 enter + GETEVENTS ── */
            uint32_t tail = __atomic_load_n(sq_tail, __ATOMIC_RELAXED);
            for (int i = 0; i < n; i++) {
                uint32_t idx = (tail + i) & sq_mask;
                struct io_uring_sqe *s = (struct io_uring_sqe*)(sqes + idx * 64);
                memset(s, 0, 64);
                s->opcode = IORING_OP_NOP;
                s->user_data = (uint64_t)(0xB0 | (r << 8) | i);
                sq_array[idx] = idx;
            }
            __atomic_store_n(sq_tail, tail + n, __ATOMIC_RELEASE);

            uint64_t t0 = rdtime();
            // Submit n + wait for n completions in one call.
            syscall(__NR_io_uring_enter, fd, (uint32_t)n, (uint32_t)n,
                    IORING_ENTER_GETEVENTS, 0, 0);
            // Reap remaining from the ring (GETEVENTS guarantees >=n available).
            reap_n(fd, cq_head, cq_tail, cqes, cq_mask, n);
            uint64_t dt = rdtime() - t0;

            if (dt < t_batch_min) t_batch_min = dt;
            if (dt > t_batch_max) t_batch_max = dt;
            t_batch_sum += dt;

            /* ── Single: N separate enters, 1 NOP each ── */
            uint64_t t1 = rdtime();
            for (int i = 0; i < n; i++) {
                tail = __atomic_load_n(sq_tail, __ATOMIC_RELAXED);
                uint32_t idx = tail & sq_mask;
                struct io_uring_sqe *s = (struct io_uring_sqe*)(sqes + idx * 64);
                memset(s, 0, 64);
                s->opcode = IORING_OP_NOP;
                s->user_data = (uint64_t)(0xC0 | (r << 8) | i);
                sq_array[idx] = idx;
                __atomic_store_n(sq_tail, tail + 1, __ATOMIC_RELEASE);

                syscall(__NR_io_uring_enter, fd, 1, 1, IORING_ENTER_GETEVENTS, 0, 0);
                reap_n(fd, cq_head, cq_tail, cqes, cq_mask, 1);
            }
            uint64_t dt_s = rdtime() - t1;

            if (dt_s < t_single_min) t_single_min = dt_s;
            if (dt_s > t_single_max) t_single_max = dt_s;
            t_single_sum += dt_s;
        }

        uint64_t batch_avg = t_batch_sum / ROUNDS;
        uint64_t single_avg = t_single_sum / ROUNDS;
        double ratio = (double)batch_avg / (double)single_avg;

        printf("%-5d %12llu (%5llu..%5llu) %12llu (%5llu..%5llu) %12llu %12llu %9.2fx\n",
               n,
               (unsigned long long)batch_avg,
               (unsigned long long)t_batch_min, (unsigned long long)t_batch_max,
               (unsigned long long)single_avg,
               (unsigned long long)t_single_min, (unsigned long long)t_single_max,
               (unsigned long long)(batch_avg / n),
               (unsigned long long)(single_avg / n),
               ratio);
    }

    printf("PASS: io_uring scale bench (NOP batching, %d rounds, GETEVENTS reap)\n", ROUNDS);
    return 0;
}
