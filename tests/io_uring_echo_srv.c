// Native io_uring-echo-server port from:
//   https://github.com/frevib/io_uring-echo-server
//
// Adapted for StarryOS: liburing_shim instead of real liburing, slot-based
// connection tracking (avoid fd-as-index), FAST_POLL native, FIXED_FILE
// supported but optional.
//
// Per-connection state machine: ACCEPT → RECV → SEND → (loop back to RECV).
// The server spawns NCLIENTS internally, runs them to completion, and reports
// wall-clock throughput for direct comparison with the epoll equivalent.
//
//   riscv64-linux-musl-gcc -static -O2 -Itests/liburing-shim \
//       tests/io_uring_echo_srv.c tests/liburing-shim/liburing_shim.c \
//       -o io_uring_echo_srv
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <time.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include "liburing_shim.h"

#define PORT        7799
#define NCLIENTS    32
#define MSG_LEN     128
#define ENTRIES     512
#define MAXEVENTS   512
#define BACKLOG     1024
#define MAXCONN     256

enum { T_ACCEPT = 0, T_RECV = 1, T_SEND = 2 };

struct conn {
    int fd;
    int type;    /* T_RECV or T_SEND */
    int slot;    /* index into conns[] */
    int remaining;  /* T_RECV: bytes recv'd so far; T_SEND: bytes sent so far */
    int send_total; /* bytes to send this echo (= msglen once fully received) */
};

static struct conn   conns[MAXCONN];
static unsigned char bufpool[MAXCONN][MSG_LEN];

static int find_slot(void) {
    for (int i = 0; i < MAXCONN; i++)
        if (conns[i].fd == 0) return i;
    return -1;
}

int main(int argc, char **argv) {
    int nclients = NCLIENTS;
    int msglen   = MSG_LEN;
    if (argc > 1) nclients = atoi(argv[1]);
    if (argc > 2) msglen   = atoi(argv[2]);
    if (nclients > MAXCONN - 16) nclients = MAXCONN - 16;
    if (msglen > (int)sizeof(bufpool[0])) msglen = (int)sizeof(bufpool[0]);

    int lfd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof addr);
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(PORT);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int opt = 1;
    setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof opt);
    if (bind(lfd, (struct sockaddr *)&addr, sizeof addr) < 0) { perror("bind"); return 1; }
    if (listen(lfd, BACKLOG) < 0) { perror("listen"); return 1; }

    printf("io_uring native echo: port=%d clients=%d msglen=%d\n",
           PORT, nclients, msglen);

    /* ── Fork N clients (identical pattern to all our echo tests) ── */
    for (int i = 0; i < nclients; i++) {
        pid_t pid = fork();
        if (pid == 0) {
            int cfd = socket(AF_INET, SOCK_STREAM, 0);
            struct sockaddr_in sa;
            memset(&sa, 0, sizeof sa);
            sa.sin_family = AF_INET;
            sa.sin_port   = htons(PORT);
            sa.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
            for (int t = 0; t < 10000 && connect(cfd, (struct sockaddr *)&sa, sizeof sa) != 0; t++)
                usleep(500);
            unsigned char *out = malloc(msglen);
            for (int j = 0; j < msglen; j++) out[j] = (unsigned char)('A' + (i + j) % 26);
            out[msglen - 1] = '0' + (i % 10);
            int sent = 0;
            while (sent < msglen) { int n = write(cfd, out + sent, msglen - sent); if (n <= 0) _exit(2); sent += n; }
            unsigned char *in = malloc(msglen);
            int got = 0;
            while (got < msglen) { int n = read(cfd, in + got, msglen - got); if (n <= 0) break; got += n; }
            int ok = (got == msglen && memcmp(in, out, msglen) == 0);
            free(out); free(in);
            close(cfd);
            _exit(ok ? 0 : 1);
        }
    }

    /* ── io_uring setup ── */
    struct io_uring ring;
    struct io_uring_params params;
    memset(&params, 0, sizeof params);
    if (io_uring_queue_init_params(ENTRIES, &ring, &params) < 0) {
        printf("FAIL: io_uring_queue_init_params\n"); return 1;
    }
    printf("  features=0x%x sq_entries=%d\n", params.features, params.sq_entries);

    /* ── Queue initial ACCEPTs (track count to avoid over-queuing) ── */
    int accept_queued = 0;
    for (int i = 0; i < 4 && accept_queued < nclients; i++) {
        struct io_uring_sqe *s = io_uring_get_sqe(&ring);
        if (!s) break;
        io_uring_prep_accept(s, lfd, NULL, NULL, 0);
        s->user_data = T_ACCEPT;
        accept_queued++;
    }

    int accepted = 0, completed = 0, failed = 0;

    struct timespec t0, t1, prog;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    prog = t0;  /* last time we made progress (reaped >=1 CQE) */

    while (completed + failed < nclients) {
        io_uring_submit_and_wait(&ring, 1);

        struct io_uring_cqe *cqes[MAXEVENTS];
        int nr = io_uring_peek_batch_cqe(&ring, cqes, MAXEVENTS);
        if (nr == 0) {
            /* No CQE this round. On SMP=1 TCG the server can be descheduled
             * for many seconds while N client processes run, so an iteration
             * counter would false-trigger. Break only on a genuine 30s of
             * ZERO completions (true stall), measured by wall-clock. */
            struct timespec now;
            clock_gettime(CLOCK_MONOTONIC, &now);
            double idle = (now.tv_sec - prog.tv_sec)
                        + (now.tv_nsec - prog.tv_nsec) / 1e9;
            if (idle > 30.0) {
                printf("  stall: no progress for %.0fs (accepted=%d completed=%d failed=%d)\n",
                       idle, accepted, completed, failed);
                break;
            }
            continue;
        }
        clock_gettime(CLOCK_MONOTONIC, &prog);  /* progress → reset idle timer */
        for (int i = 0; i < nr; i++) {
            struct io_uring_cqe *c = cqes[i];
            uint64_t ud = c->user_data;
            int res = c->res;

            if (ud == T_ACCEPT) {
                if (res < 0) { io_uring_cqe_seen(&ring, c); continue; }
                int slot = find_slot();
                if (slot < 0) { close(res); io_uring_cqe_seen(&ring, c); continue; }
                conns[slot].fd   = res;
                conns[slot].type = T_RECV;
                conns[slot].slot = slot;
                conns[slot].remaining = 0;
                accepted++;
                /* Queue RECV for the new connection */
                struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                if (s) {
                    io_uring_prep_recv(s, res, bufpool[slot], msglen, 0);
                    io_uring_sqe_set_data(s, &conns[slot]);
                }
                /* Re-queue ACCEPT if more clients expected */
                if (accept_queued < nclients) {
                    struct io_uring_sqe *sa = io_uring_get_sqe(&ring);
                    if (sa) {
                        io_uring_prep_accept(sa, lfd, NULL, NULL, 0);
                        sa->user_data = T_ACCEPT;
                        accept_queued++;
                    }
                }
            } else {
                struct conn *cn = (struct conn *)(uintptr_t)ud;
                int slot = cn->slot;
                if (cn->type == T_RECV) {
                    if (res <= 0) {
                        shutdown(cn->fd, SHUT_RDWR);
                        close(cn->fd); cn->fd = 0; failed++;
                    } else {
                        /* Accumulate a full msglen before echoing — a single
                         * RECV may return a partial read (common at 128B on
                         * loopback under load), and echoing <msglen would
                         * deadlock the client's read loop. */
                        cn->remaining += res;
                        if (cn->remaining >= msglen) {
                            cn->type = T_SEND;
                            cn->send_total = msglen;
                            cn->remaining = 0;
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) {
                                io_uring_prep_send(s, cn->fd, bufpool[slot], msglen, 0);
                                io_uring_sqe_set_data(s, cn);
                            }
                        } else {
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) {
                                io_uring_prep_recv(s, cn->fd, bufpool[slot] + cn->remaining,
                                                   msglen - cn->remaining, 0);
                                io_uring_sqe_set_data(s, cn);
                            }
                        }
                    }
                } else { /* T_SEND */
                    if (res <= 0) {
                        shutdown(cn->fd, SHUT_RDWR);
                        close(cn->fd); cn->fd = 0; failed++;
                    } else {
                        cn->remaining += res;
                        if (cn->remaining >= cn->send_total) {
                            /* Single-echo: client sends once, we echo once, done. */
                            completed++;
                            shutdown(cn->fd, SHUT_RDWR);
                            close(cn->fd); cn->fd = 0;
                        } else {
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) {
                                io_uring_prep_send(s, cn->fd, bufpool[slot] + cn->remaining,
                                                   cn->send_total - cn->remaining, 0);
                                io_uring_sqe_set_data(s, cn);
                            }
                        }
                    }
                }
            }
            io_uring_cqe_seen(&ring, c);
        }
    }

    clock_gettime(CLOCK_MONOTONIC, &t1);

    /* ── Reap clients ── */
    int client_ok = 0;
    for (int i = 0; i < nclients; i++) {
        int st;
        if (waitpid(-1, &st, 0) > 0 && WIFEXITED(st) && WEXITSTATUS(st) == 0)
            client_ok++;
    }

    io_uring_queue_exit(&ring);
    close(lfd);

    double dt_ms = (t1.tv_sec - t0.tv_sec) * 1000.0 + (t1.tv_nsec - t0.tv_nsec) / 1e6;
    double total_bytes = (double)nclients * (double)msglen * 2.0;
    double mbytes = total_bytes / (1024.0 * 1024.0);
    double thr_mbps = (dt_ms > 0) ? mbytes / (dt_ms / 1000.0) : 0;

    printf("  accepted=%d completed=%d failed=%d  clients_ok=%d/%d\n",
           accepted, completed, failed, client_ok, nclients);
    printf("  time=%.1f ms  throughput=%.2f MB/s  (%.0f req/s)\n",
           dt_ms, thr_mbps, dt_ms > 0 ? (double)completed / (dt_ms / 1000.0) : 0);

    if (completed == nclients && client_ok == nclients) {
        printf("PASS: io_uring native echo (%d clients, %d bytes)\n", nclients, msglen);
        return 0;
    }
    printf("FAIL: io_uring native echo (completed=%d client_ok=%d)\n", completed, client_ok);
    return 1;
}
