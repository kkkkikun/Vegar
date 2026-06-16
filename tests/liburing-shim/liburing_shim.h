// Minimal static liburing shim for StarryOS's io_uring.
//
// Implements just enough of the liburing API to compile and run third-party
// io_uring tests (perf-test-for-io_uring) and simple apps (echo server) against
// our kernel, WITHOUT depending on the real liburing. Built as a static C lib
// with riscv64-linux-musl-gcc and linked into test binaries.
//
// Kernel ABI requirements (all satisfied as of Stage 1):
//   - io_uring_setup writes sq_off/cq_off, power-of-two entries.
//   - mmap offsets SQ_RING=0 / CQ_RING=0x8000000 / SQES=0x10000000.
//   - io_uring_enter honors IORING_ENTER_GETEVENTS + min_complete (S1.1).
//   - CQ overflow counter at cq_off.overflow (S1.2).
//   - READV/WRITEV opcodes (S1.3).
#ifndef LIBURING_SHIM_H
#define LIBURING_SHIM_H

#include <stdint.h>
#include <stddef.h>
#include <sys/uio.h>      /* struct iovec (for readv/writev/register_buffers) */
#include <sys/socket.h>   /* sockaddr / socklen_t for io_uring_prep_accept */

#define IORING_OFF_SQ_RING 0u
#define IORING_OFF_CQ_RING 0x8000000u
#define IORING_OFF_SQES    0x10000000u

#define IORING_ENTER_GETEVENTS 1u

/* io_uring opcodes we support. */
#define IORING_OP_NOP        0
#define IORING_OP_READV      1
#define IORING_OP_WRITEV     2
#define IORING_OP_READ_FIXED 4
#define IORING_OP_WRITE_FIXED 5
#define IORING_OP_POLL_ADD   6
#define IORING_OP_READ       22
#define IORING_OP_WRITE      23
#define IORING_OP_ACCEPT     13
#define IORING_OP_SEND       26
#define IORING_OP_RECV       27

struct io_uring_sqe {
    uint8_t  opcode;
    uint8_t  flags;
    uint16_t ioprio;
    int32_t  fd;
    uint64_t off;
    uint64_t addr;
    uint32_t len;
    union {
        uint32_t rw_flags;
        uint32_t poll32_events;
        uint32_t sync_range_flags;
        uint32_t msg_flags;
        uint32_t timeout_flags;
        uint32_t accept_flags;
    };
    uint64_t user_data;
    union {
        struct {
            uint16_t buf_index;
            uint16_t personality;
        };
        uint64_t __pad2_64;
    };
    union {
        struct {
            uint32_t addr3;
            uint16_t __pad3_1;
            uint16_t file_index;
        };
        uint64_t __pad3;
    };
    /* Tail padding to reach the 64-byte io_uring_sqe stride the kernel uses
     * (kernel SQE is 64 bytes; we index sqes[] by sizeof, so this must match). */
    uint64_t __pad4;
} __attribute__((packed));

struct io_uring_cqe {
    uint64_t user_data;
    int32_t  res;
    uint32_t flags;
};

struct io_sqring_offsets {
    uint32_t head, tail, ring_mask, ring_entries, flags, dropped, array, resv1;
    uint64_t resv2;
};
struct io_cqring_offsets {
    uint32_t head, tail, ring_mask, ring_entries, overflow, cqes, flags, resv1;
    uint64_t resv2;
};
struct io_uring_params {
    uint32_t sq_entries, cq_entries, flags, sq_thread_cpu, sq_thread_idle;
    uint32_t features, wq_fd, resv[3];
    struct io_sqring_offsets sq_off;
    struct io_cqring_offsets cq_off;
};

/* liburing-style ring handles. */
struct io_uring_sq {
    unsigned *head, *tail, *ring_mask, *ring_entries, *flags, *dropped, *array;
    struct io_uring_sqe *sqes;
    unsigned sqe_head, sqe_tail;
    unsigned ring_sz, sqe_sz;
    void *ring_ptr;
};
struct io_uring_cq {
    unsigned *head, *tail, *ring_mask, *ring_entries, *overflow;
    struct io_uring_cqe *cqes;
    unsigned ring_sz;
    void *ring_ptr;
};
struct io_uring {
    struct io_uring_sq sq;
    struct io_uring_cq cq;
    unsigned flags;
    int ring_fd;
};

/* Core lifecycle + submission/completion API (liburing-compatible subset). */
int  io_uring_queue_init(unsigned entries, struct io_uring *ring, unsigned flags);
void io_uring_queue_exit(struct io_uring *ring);

struct io_uring_sqe *io_uring_get_sqe(struct io_uring *ring);
int  io_uring_submit(struct io_uring *ring);
int  io_uring_submit_and_wait(struct io_uring *ring, unsigned wait_nr);
int  io_uring_wait_cqe_nr(struct io_uring *ring, struct io_uring_cqe **cqe_ptr,
                          unsigned wait_nr);
int  io_uring_wait_cqe(struct io_uring *ring, struct io_uring_cqe **cqe_ptr);
int  io_uring_peek_batch_cqe(struct io_uring *ring, struct io_uring_cqe **cqes,
                             unsigned nr);
void io_uring_cqe_seen(struct io_uring *ring, struct io_uring_cqe *cqe);

unsigned io_uring_sq_ready(struct io_uring *ring);
unsigned io_uring_sq_space_left(struct io_uring *ring);
unsigned io_uring_cq_ready(struct io_uring *ring);

int io_uring_register_buffers(struct io_uring *ring, const struct iovec *iovs,
                              unsigned nr_iovs);

/* SQE preparation helpers (no syscall, just field fills). */
static inline void io_uring_prep_rw(struct io_uring_sqe *sqe, uint8_t op,
                                    int fd, const void *addr, unsigned len,
                                    uint64_t off) {
    sqe->opcode = op; sqe->flags = 0; sqe->ioprio = 0; sqe->fd = fd;
    sqe->off = off; sqe->addr = (uint64_t)(uintptr_t)addr; sqe->len = len;
    sqe->rw_flags = 0; sqe->user_data = 0; sqe->buf_index = 0;
}
static inline void io_uring_prep_read(struct io_uring_sqe *sqe, int fd,
                                      void *buf, unsigned nbytes, uint64_t off) {
    io_uring_prep_rw(sqe, IORING_OP_READ, fd, buf, nbytes, off);
}
static inline void io_uring_prep_write(struct io_uring_sqe *sqe, int fd,
                                       const void *buf, unsigned nbytes, uint64_t off) {
    io_uring_prep_rw(sqe, IORING_OP_WRITE, fd, buf, nbytes, off);
}
static inline void io_uring_prep_readv(struct io_uring_sqe *sqe, int fd,
                                       const struct iovec *iovecs, unsigned nr,
                                       uint64_t off) {
    io_uring_prep_rw(sqe, IORING_OP_READV, fd, iovecs, nr, off);
}
static inline void io_uring_prep_writev(struct io_uring_sqe *sqe, int fd,
                                        const struct iovec *iovecs, unsigned nr,
                                        uint64_t off) {
    io_uring_prep_rw(sqe, IORING_OP_WRITEV, fd, iovecs, nr, off);
}
static inline void io_uring_prep_recv(struct io_uring_sqe *sqe, int fd,
                                      void *buf, unsigned nbytes, unsigned flags) {
    io_uring_prep_rw(sqe, IORING_OP_RECV, fd, buf, nbytes, 0);
    sqe->msg_flags = flags;
}
static inline void io_uring_prep_send(struct io_uring_sqe *sqe, int fd,
                                      const void *buf, unsigned nbytes, unsigned flags) {
    io_uring_prep_rw(sqe, IORING_OP_SEND, fd, buf, nbytes, 0);
    sqe->msg_flags = flags;
}
static inline void io_uring_prep_accept(struct io_uring_sqe *sqe, int fd,
                                        struct sockaddr *addr, socklen_t *addrlen,
                                        unsigned flags) {
    io_uring_prep_rw(sqe, IORING_OP_ACCEPT, fd, addr, addrlen ? *addrlen : 0, 0);
    sqe->accept_flags = flags;
}
static inline void io_uring_prep_poll_add(struct io_uring_sqe *sqe, int fd,
                                          unsigned poll_mask) {
    /* poll_mask (e.g. POLLIN=1) goes in the poll32_events union slot. */
    io_uring_prep_rw(sqe, IORING_OP_POLL_ADD, fd, NULL, 0, 0);
    sqe->poll32_events = poll_mask;
}

#endif /* LIBURING_SHIM_H */
