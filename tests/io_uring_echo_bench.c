// io_uring vs epoll — sustained throughput benchmark.
//
// N concurrent clients, each sends R echo rounds (send → recv → verify).
// Measures wall-clock req/s, directly comparable to reference benchmarks.
//
// Usage: io_uring_echo_bench <iouring|epoll> <N clients> <R rounds> <msg_len>
//
//   riscv64-linux-musl-gcc -static -O2 -Itests/liburing-shim \
//       tests/io_uring_echo_bench.c tests/liburing-shim/liburing_shim.c \
//       -o io_uring_echo_bench
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
#include <sys/epoll.h>
#include <sys/wait.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include "liburing_shim.h"

#define PORT        7800
#define MAXCONN     512
#define BACKLOG     512
#define MAXEVENTS   256
#define ENTRIES     512

enum { T_ACCEPT = 0, T_RECV = 1, T_SEND = 2 };

struct conn { int fd, type, slot, remaining, send_total, rounds_left; };
static struct conn   conns[MAXCONN];
static unsigned char bufpool[MAXCONN][4096];  // max msg_len = 4096

static void set_nonblocking(int fd) { int fl = fcntl(fd, F_GETFL, 0); if (fl >= 0) fcntl(fd, F_SETFL, fl | O_NONBLOCK); }

static int find_slot(void) {
    for (int i = 0; i < MAXCONN; i++)
        if (conns[i].fd == 0) return i;
    return -1;
}

/* ── io_uring server ── */
static int server_iouring(int lfd, int N, int R, int msglen) {
    struct io_uring ring;
    struct io_uring_params params;
    memset(&params, 0, sizeof params);
    if (io_uring_queue_init_params(ENTRIES, &ring, &params) < 0) return -1;

    int accept_queued = 0;
    for (int i = 0; i < 8 && accept_queued < N; i++) {
        struct io_uring_sqe *s = io_uring_get_sqe(&ring);
        if (!s) break;
        io_uring_prep_accept(s, lfd, NULL, NULL, 0);
        s->user_data = T_ACCEPT;
        accept_queued++;
    }

    int accepted = 0, completed = 0, failed = 0, total_rounds = N * R;
    struct timespec prog;
    clock_gettime(CLOCK_MONOTONIC, &prog);  /* last-progress wall-clock */

    while (completed + failed < total_rounds) {
        io_uring_submit_and_wait(&ring, 1);

        struct io_uring_cqe *cqes[MAXEVENTS];
        int nr = io_uring_peek_batch_cqe(&ring, cqes, MAXEVENTS);
        if (nr == 0) {
            /* Wall-clock no-progress guard (not an iteration counter): on SMP=1
             * TCG the server is descheduled for seconds while clients run, so a
             * counter would false-trigger. Break only on a genuine 30s of ZERO
             * CQEs. */
            struct timespec now;
            clock_gettime(CLOCK_MONOTONIC, &now);
            double idle = (now.tv_sec - prog.tv_sec) + (now.tv_nsec - prog.tv_nsec) / 1e9;
            if (idle > 30.0) break;
            continue;
        }
        clock_gettime(CLOCK_MONOTONIC, &prog);
        for (int i = 0; i < nr; i++) {
            struct io_uring_cqe *c = cqes[i];
            uint64_t ud = c->user_data;
            int res = c->res;

            if (ud == T_ACCEPT) {
                if (res < 0) { io_uring_cqe_seen(&ring, c); continue; }
                int slot = find_slot();
                if (slot < 0) { close(res); io_uring_cqe_seen(&ring, c); continue; }
                conns[slot].fd = res; conns[slot].type = T_RECV;
                conns[slot].slot = slot; conns[slot].rounds_left = R;
                conns[slot].remaining = 0;   // bytes recv'd so far this round
                accepted++;
                struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                if (s) { io_uring_prep_recv(s, res, bufpool[slot], msglen, 0); io_uring_sqe_set_data(s, &conns[slot]); }
                if (accept_queued < N) {
                    struct io_uring_sqe *sa = io_uring_get_sqe(&ring);
                    if (sa) { io_uring_prep_accept(sa, lfd, NULL, NULL, 0); sa->user_data = T_ACCEPT; accept_queued++; }
                }
            } else {
                struct conn *cn = (struct conn *)(uintptr_t)ud;
                int slot = cn->slot;
                // `remaining` is overloaded: in T_RECV it counts bytes recv'd
                // this round; in T_SEND it counts bytes sent. Accumulate a full
                // msglen before echoing (mirrors the epoll server) — a single
                // RECV may return a partial read on loopback under load, and
                // echoing <msglen would deadlock the client's read loop.
                if (cn->type == T_RECV) {
                    if (res <= 0) { shutdown(cn->fd, SHUT_RDWR); close(cn->fd); cn->fd = 0; failed += cn->rounds_left; }
                    else {
                        cn->remaining += res;
                        if (cn->remaining >= msglen) {
                            cn->type = T_SEND; cn->send_total = msglen; cn->remaining = 0;
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) { io_uring_prep_send(s, cn->fd, bufpool[slot], msglen, 0); io_uring_sqe_set_data(s, cn); }
                        } else {
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) { io_uring_prep_recv(s, cn->fd, bufpool[slot] + cn->remaining, msglen - cn->remaining, 0); io_uring_sqe_set_data(s, cn); }
                        }
                    }
                } else {
                    if (res <= 0) { shutdown(cn->fd, SHUT_RDWR); close(cn->fd); cn->fd = 0; failed += cn->rounds_left; }
                    else {
                        cn->remaining += res;
                        if (cn->remaining >= cn->send_total) {
                            completed++; cn->rounds_left--;
                            if (cn->rounds_left > 0) {
                                cn->type = T_RECV; cn->remaining = 0;
                                struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                                if (s) { io_uring_prep_recv(s, cn->fd, bufpool[slot], msglen, 0); io_uring_sqe_set_data(s, cn); }
                            } else { shutdown(cn->fd, SHUT_RDWR); close(cn->fd); cn->fd = 0; }
                        } else {
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) { io_uring_prep_send(s, cn->fd, bufpool[slot] + cn->remaining, cn->send_total - cn->remaining, 0); io_uring_sqe_set_data(s, cn); }
                        }
                    }
                }
            }
            io_uring_cqe_seen(&ring, c);
        }
    }
    io_uring_queue_exit(&ring);
    return completed;
}

/* ── epoll server ── */
static int server_epoll(int lfd, int N, int R, int msglen) {
    int epfd = epoll_create1(0);
    struct epoll_event ev;
    ev.events = EPOLLIN; ev.data.fd = lfd;
    epoll_ctl(epfd, EPOLL_CTL_ADD, lfd, &ev);

    int accepted = 0, completed = 0, failed = 0, total_rounds = N * R;
    struct timespec prog;
    clock_gettime(CLOCK_MONOTONIC, &prog);  /* last-progress wall-clock */

    while (completed + failed < total_rounds) {
        struct epoll_event events[MAXEVENTS];
        int nfds = epoll_wait(epfd, events, MAXEVENTS, 50);
        if (nfds <= 0) {
            /* Wall-clock no-progress guard: break only on a genuine 30s of ZERO
             * events, not on iteration count (SMP=1 TCG deschedules the server
             * for seconds at a time while clients run). */
            struct timespec now;
            clock_gettime(CLOCK_MONOTONIC, &now);
            double idle = (now.tv_sec - prog.tv_sec) + (now.tv_nsec - prog.tv_nsec) / 1e9;
            if (idle > 30.0) break;
            continue;
        }
        clock_gettime(CLOCK_MONOTONIC, &prog);

        for (int i = 0; i < nfds; i++) {
            int fd = events[i].data.fd;
            if (fd == lfd && (events[i].events & EPOLLIN)) {
                int cfd;
                while (accepted < N && (cfd = accept(lfd, NULL, NULL)) >= 0) {
                    int slot = find_slot();
                    if (slot < 0) { close(cfd); break; }
                    set_nonblocking(cfd);
                    conns[slot].fd = cfd; conns[slot].type = T_RECV;
                    conns[slot].slot = slot; conns[slot].rounds_left = R;
                    conns[slot].remaining = 0;
                    accepted++;
                    ev.events = EPOLLIN | EPOLLOUT; ev.data.fd = cfd;
                    epoll_ctl(epfd, EPOLL_CTL_ADD, cfd, &ev);
                }
            } else {
                int ci = -1;
                for (int j = 0; j < MAXCONN; j++) if (conns[j].fd == fd) { ci = j; break; }
                if (ci < 0) continue;
                struct conn *cn = &conns[ci];
                if (cn->type == T_RECV && (events[i].events & EPOLLIN)) {
                    int n = read(fd, bufpool[ci] + cn->remaining, msglen - cn->remaining);
                    if (n <= 0) { epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL); close(fd); cn->fd = 0; failed += cn->rounds_left; }
                    else {
                        cn->remaining += n;
                        if (cn->remaining >= msglen) {
                            cn->type = T_SEND; cn->send_total = cn->remaining; cn->remaining = 0;
                        }
                    }
                } else if (cn->type == T_SEND && (events[i].events & EPOLLOUT)) {
                    int n = write(fd, bufpool[ci] + cn->remaining, cn->send_total - cn->remaining);
                    if (n <= 0) { epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL); close(fd); cn->fd = 0; failed += cn->rounds_left; }
                    else {
                        cn->remaining += n;
                        if (cn->remaining >= cn->send_total) {
                            completed++; cn->rounds_left--;
                            if (cn->rounds_left > 0) { cn->type = T_RECV; cn->remaining = 0; }
                            else { epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL); close(fd); cn->fd = 0; }
                        }
                    }
                }
            }
        }
    }
    close(epfd);
    return completed;
}

/* ── main ── */
int main(int argc, char **argv) {
    if (argc < 5) {
        printf("Usage: %s <iouring|epoll> <N clients> <R rounds> <msg_len>\n", argv[0]);
        return 2;
    }
    int is_iouring = (strcmp(argv[1], "iouring") == 0);
    int N = atoi(argv[2]), R = atoi(argv[3]), msglen = atoi(argv[4]);
    if (N <= 0) N = 32; if (R <= 0) R = 10;
    if (msglen <= 0 || msglen > 4096) msglen = 128;

    int lfd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof addr);
    addr.sin_family = AF_INET; addr.sin_port = htons(PORT);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int opt = 1; setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof opt);
    if (is_iouring) {} else set_nonblocking(lfd);
    bind(lfd, (struct sockaddr *)&addr, sizeof addr);
    listen(lfd, BACKLOG);

    printf("echo bench: mode=%s N=%d R=%d msglen=%d ", argv[1], N, R, msglen);
    fflush(stdout);

    /* Fork N clients */
    for (int i = 0; i < N; i++) {
        pid_t pid = fork();
        if (pid == 0) {
            int cfd = socket(AF_INET, SOCK_STREAM, 0);
            struct sockaddr_in sa;
            memset(&sa, 0, sizeof sa);
            sa.sin_family = AF_INET; sa.sin_port = htons(PORT);
            sa.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
            for (int t = 0; t < 8000 && connect(cfd, (struct sockaddr *)&sa, sizeof sa) != 0; t++) usleep(500);
            unsigned char *out = malloc(msglen), *in = malloc(msglen);
            for (int r = 0; r < R; r++) {
                for (int j = 0; j < msglen; j++) out[j] = (unsigned char)('A' + (i + j + r) % 26);
                out[msglen-1] = '0' + (i % 10);
                int sent = 0; while (sent < msglen) { int n = write(cfd, out + sent, msglen - sent); if (n <= 0) _exit(2); sent += n; }
                int got = 0; while (got < msglen) { int n = read(cfd, in + got, msglen - got); if (n <= 0) _exit(1); got += n; }
                if (memcmp(in, out, msglen) != 0) _exit(1);
            }
            free(out); free(in); close(cfd);
            _exit(0);
        }
    }

    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    int ok = is_iouring ? server_iouring(lfd, N, R, msglen) : server_epoll(lfd, N, R, msglen);
    clock_gettime(CLOCK_MONOTONIC, &t1);

    int client_ok = 0;
    for (int i = 0; i < N; i++) { int st; if (waitpid(-1, &st, 0) > 0 && WIFEXITED(st) && WEXITSTATUS(st) == 0) client_ok++; }

    double dt_s = (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) / 1e9;
    int total = N * R;
    printf("completed=%d client_ok=%d/%d time=%.3fs req/s=%.0f\n",
           ok, client_ok, N, dt_s, dt_s > 0 ? (double)ok / dt_s : 0);

    close(lfd);
    return (ok == total && client_ok == N) ? 0 : 1;
}
