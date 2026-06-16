// io_uring concurrent echo server (Stage 1.5) — C port of the standard
// tokio-rs `tcp_echo.rs` event loop, via the liburing shim.
//
// Single-threaded, event-driven: one io_uring ring, submit_and_wait(1) loop,
// per-connection token state machine ACCEPT -> RECV -> SEND. N clients connect
// concurrently; the server multiplexes all of them on ONE worker. This is the
// hard use case for the multiplexing worker: with the old serial worker a
// pending RECV on one connection would block all others (head-of-line
// blocking); here all N must complete concurrently and each client must get its
// OWN payload back (no cross-connection corruption).
//
//   riscv64-linux-musl-gcc -static -O2 -Itests/liburing-shim \
//       tests/io_uring_echo.c tests/liburing-shim/liburing_shim.c -o io_uring_echo
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <sys/wait.h>
#include "liburing_shim.h"

#define PORT      7788
#define NCLIENTS  24
#define PAYLOAD   64
#define ENTRIES   64
#define MAXCONN   (NCLIENTS + 8)

enum { T_ACCEPT = 0, T_RECV = 1, T_SEND = 2 };

struct conn {
    int used;
    int fd;
    int state;     /* T_RECV | T_SEND */
    int remaining; /* bytes still to send (partial-send handling) */
};

static struct conn conns[MAXCONN];
static char bufpool[MAXCONN][PAYLOAD];

static int alloc_conn(void) {
    /* index 0 is reserved for T_ACCEPT; conns start at 1. */
    for (int i = 1; i < MAXCONN; i++)
        if (!conns[i].used) return i;
    return -1;
}

int main(void) {
    int lfd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof addr);
    addr.sin_family = AF_INET;
    addr.sin_port = htons(PORT);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int opt = 1;
    setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof opt);
    if (bind(lfd, (struct sockaddr *)&addr, sizeof addr) < 0) { perror("bind"); return 1; }
    if (listen(lfd, NCLIENTS + 4) < 0) { perror("listen"); return 1; }
    printf("io_uring echo: listening 127.0.0.1:%d, %d concurrent clients\n", PORT, NCLIENTS);

    /* Fork N clients: each connects, sends a unique payload, reads the echo. */
    for (int i = 0; i < NCLIENTS; i++) {
        pid_t pid = fork();
        if (pid == 0) {
            int cfd = socket(AF_INET, SOCK_STREAM, 0);
            struct sockaddr_in sa;
            memset(&sa, 0, sizeof sa);
            sa.sin_family = AF_INET;
            sa.sin_port = htons(PORT);
            sa.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
            for (int t = 0; t < 3000 && connect(cfd, (struct sockaddr *)&sa, sizeof sa) != 0; t++)
                usleep(1000);
            char out[PAYLOAD];
            memset(out, 'A' + (i % 26), PAYLOAD);
            out[PAYLOAD - 1] = '0' + (i % 10); /* make each client's payload distinct */
            if (write(cfd, out, PAYLOAD) != PAYLOAD) _exit(2);
            char in[PAYLOAD];
            int got = 0;
            while (got < PAYLOAD) {
                int n = read(cfd, in + got, PAYLOAD - got);
                if (n <= 0) break;
                got += n;
            }
            int ok = (got == PAYLOAD && memcmp(in, out, PAYLOAD) == 0);
            close(cfd);
            _exit(ok ? 0 : 1);
        }
    }

    /* io_uring setup via the shim. */
    struct io_uring ring;
    if (io_uring_queue_init(ENTRIES, &ring, 0) < 0) { printf("FAIL: queue_init\n"); return 1; }

    int accepted = 0, echoed = 0, failed = 0;
    int accept_queued = 0; /* cap total queued ACCEPTs at NCLIENTS */
    /* Queue a few ACCEPTs up front to accept clients promptly. */
    for (int i = 0; i < 4 && accept_queued < NCLIENTS; i++) {
        struct io_uring_sqe *s = io_uring_get_sqe(&ring);
        io_uring_prep_accept(s, lfd, NULL, NULL, 0);
        s->user_data = T_ACCEPT;
        accept_queued++;
    }

    while (echoed + failed < NCLIENTS) {
        io_uring_submit_and_wait(&ring, 1); /* flush queued SQEs, wait for >=1 CQE */

        struct io_uring_cqe *cqes[ENTRIES];
        int nr = io_uring_peek_batch_cqe(&ring, cqes, ENTRIES);
        for (int i = 0; i < nr; i++) {
            struct io_uring_cqe *c = cqes[i];
            uint64_t ud = c->user_data;
            int res = c->res;

            if (ud == T_ACCEPT) {
                if (res < 0) continue; /* shouldn't happen on loopback; retry next round */
                int fd = res;
                int ci = alloc_conn();
                if (ci < 0) { close(fd); continue; }
                conns[ci].used = 1;
                conns[ci].fd = fd;
                conns[ci].state = T_RECV;
                accepted++;
                struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                io_uring_prep_recv(s, fd, bufpool[ci], PAYLOAD, 0);
                s->user_data = ci;
                /* Refill one ACCEPT, but never queue more than NCLIENTS total —
                 * an extra ACCEPT would block forever in complete_accept waiting
                 * for a client that never connects, deadlocking the submitter. */
                if (accept_queued < NCLIENTS) {
                    struct io_uring_sqe *sa = io_uring_get_sqe(&ring);
                    io_uring_prep_accept(sa, lfd, NULL, NULL, 0);
                    sa->user_data = T_ACCEPT;
                    accept_queued++;
                }
            } else {
                int ci = (int)ud;
                struct conn *cn = &conns[ci];
                if (cn->state == T_RECV) {
                    if (res <= 0) { /* client closed without sending */
                        close(cn->fd); cn->used = 0; failed++;
                    } else {
                        cn->state = T_SEND;
                        cn->remaining = res;
                        struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                        io_uring_prep_send(s, cn->fd, bufpool[ci], res, 0);
                        s->user_data = ci;
                    }
                } else { /* T_SEND */
                    if (res <= 0) { close(cn->fd); cn->used = 0; failed++; continue; }
                    cn->remaining -= res;
                    if (cn->remaining <= 0) {
                        echoed++;          /* whole payload echoed back */
                        close(cn->fd); cn->used = 0;
                    } else {
                        int off = res; /* partial send: continue (rare for small payload) */
                        (void)off;
                        struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                        io_uring_prep_send(s, cn->fd, bufpool[ci], cn->remaining, 0);
                        s->user_data = ci;
                    }
                }
            }
            io_uring_cqe_seen(&ring, c);
        }
    }

    /* Reap clients, count how many got a correct echo. */
    int client_ok = 0;
    for (int i = 0; i < NCLIENTS; i++) {
        int st;
        waitpid(-1, &st, 0);
        if (WIFEXITED(st) && WEXITSTATUS(st) == 0) client_ok++;
    }

    io_uring_queue_exit(&ring);
    printf("  accepted=%d echoed=%d failed=%d  clients_ok=%d/%d\n",
           accepted, echoed, failed, client_ok, NCLIENTS);
    if (echoed == NCLIENTS && client_ok == NCLIENTS) {
        printf("PASS: io_uring concurrent echo (event-driven, %d clients multiplexed on 1 worker)\n",
               NCLIENTS);
        return 0;
    }
    printf("FAIL: io_uring concurrent echo\n");
    return 1;
}
