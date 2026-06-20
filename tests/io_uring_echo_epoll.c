// io_uring vs epoll — apples-to-apples comparison.
//
// Same single-threaded event-driven echo server as io_uring_echo.c, but using
// epoll + non-blocking I/O instead of io_uring. The ONLY variable is the I/O
// submission mechanism:
//
//   io_uring: submit_and_wait (1 syscall for N ops) + ring-mapped CQE harvest
//   epoll:    epoll_wait (1 syscall for N events) + per-fd read/write syscalls
//
// Both are single-threaded, both handle K concurrent clients, both use the SAME
// client code (fork N, connect, send unique payload, verify echo). This isolates
// the structural difference: batched ring submission vs per-op syscalls.
//
// Strategy: all client fds are registered with EPOLLIN|EPOLLOUT from the start.
// The handler decides what to do based on connection state. No EPOLL_CTL_MOD —
// avoids potential StarryOS epoll MOD bugs.
//
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_echo_epoll.c -o io_uring_echo_epoll
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
#include <netinet/in.h>
#include <arpa/inet.h>
#include <sys/wait.h>

#define PORT      7798
#ifndef NCLIENTS
#define NCLIENTS  24
#endif
#define PAYLOAD   64
#define MAXEVENTS 64
#define MAXCONN   (NCLIENTS + 8)

#define ST_RECV 0
#define ST_SEND 1
#define ST_DONE 2

struct conn {
    int fd;
    int state;
    int recvd;
    int send_rem;
    int send_total;
};

static struct conn conns[MAXCONN];
static char   bufpool[MAXCONN][PAYLOAD];

static void set_nonblocking(int fd) {
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags >= 0) fcntl(fd, F_SETFL, flags | O_NONBLOCK);
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
    set_nonblocking(lfd);
    if (bind(lfd, (struct sockaddr *)&addr, sizeof addr) < 0) { perror("bind"); return 1; }
    if (listen(lfd, NCLIENTS + 4) < 0) { perror("listen"); return 1; }

    printf("epoll echo: listening 127.0.0.1:%d, %d concurrent clients\n", PORT, NCLIENTS);

    /* ── Fork N clients (identical to io_uring_echo.c) ── */
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
            out[PAYLOAD - 1] = '0' + (i % 10);
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

    /* ── epoll event loop ── */
    int epfd = epoll_create1(0);
    printf("epoll echo: epfd=%d\n", epfd);
    if (epfd < 0) { perror("epoll_create1"); return 1; }

    struct epoll_event ev;
    ev.events = EPOLLIN;
    ev.data.fd = lfd;
    if (epoll_ctl(epfd, EPOLL_CTL_ADD, lfd, &ev) < 0) {
        perror("epoll_ctl lfd"); return 1;
    }
    printf("epoll echo: lfd=%d added to epoll\n", lfd);

    int accepted = 0, echoed = 0, failed = 0;
    int wakeups = 0, read_calls = 0, write_calls = 0;
    int stall = 0;  // consecutive epoll_wait calls with 0 events

    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    while (echoed + failed < NCLIENTS) {
        struct epoll_event events[MAXEVENTS];
        int nfds = epoll_wait(epfd, events, MAXEVENTS, 50);  // 50ms timeout for progress detection
        wakeups++;
        if (nfds < 0) { if (errno == EINTR) continue; break; }
        if (nfds == 0) {
            stall++;
            if (stall > 600) {  // 30s idle → hang detected
                printf("epoll echo: STALL detected (stall=%d, echoed=%d failed=%d accepted=%d)\n",
                       stall, echoed, failed, accepted);
                break;
            }
            continue;
        }
        stall = 0;  // reset on progress

        for (int i = 0; i < nfds; i++) {
            int fd = events[i].data.fd;
            uint32_t revents = events[i].events;

            if (fd == lfd && (revents & EPOLLIN)) {
                /* ── Accept ── */
                int cfd;
                while ((cfd = accept(lfd, NULL, NULL)) >= 0) {
                    if (accepted >= NCLIENTS) { close(cfd); break; }
                    int ci = -1;
                    for (int j = 1; j < MAXCONN; j++)
                        if (conns[j].fd == 0) { ci = j; break; }
                    if (ci < 0) { close(cfd); continue; }
                    set_nonblocking(cfd);
                    conns[ci].fd    = cfd;
                    conns[ci].state = ST_RECV;
                    conns[ci].recvd = 0;
                    // Register with BOTH IN and OUT — never MOD.
                    ev.events   = EPOLLIN | EPOLLOUT;
                    ev.data.fd  = cfd;
                    if (epoll_ctl(epfd, EPOLL_CTL_ADD, cfd, &ev) < 0) {
                        printf("epoll echo: epoll_ctl ADD fd=%d failed (errno=%d)\n", cfd, errno);
                    }
                    accepted++;
                }
            } else if (fd != lfd) {
                /* ── Client I/O ── */
                int ci = -1;
                for (int j = 1; j < MAXCONN; j++)
                    if (conns[j].fd == fd) { ci = j; break; }
                if (ci < 0 || conns[ci].state == ST_DONE) continue;

                struct conn *cn = &conns[ci];

                /* Try read if we're in RECV state and fd is readable */
                if (cn->state == ST_RECV && (revents & EPOLLIN)) {
                    int n = read(fd, bufpool[ci] + cn->recvd,
                                 PAYLOAD - cn->recvd);
                    read_calls++;
                    if (n > 0) {
                        cn->recvd += n;
                        if (cn->recvd >= PAYLOAD) {
                            cn->state     = ST_SEND;
                            cn->send_rem  = cn->recvd;
                            cn->send_total = cn->recvd;
                        }
                    } else if (n == 0 || (n < 0 && errno != EAGAIN)) {
                        epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL);
                        close(fd); cn->state = ST_DONE; cn->fd = 0; failed++;
                    }
                }

                /* Try write if we're in SEND state and fd is writable */
                if (cn->state == ST_SEND && (revents & EPOLLOUT)) {
                    int off = cn->send_total - cn->send_rem;
                    int n = write(fd, bufpool[ci] + off, cn->send_rem);
                    write_calls++;
                    if (n > 0) {
                        cn->send_rem -= n;
                        if (cn->send_rem <= 0) {
                            echoed++;
                            epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL);
                            close(fd); cn->state = ST_DONE; cn->fd = 0;
                        }
                    } else if (n == 0 || (n < 0 && errno != EAGAIN)) {
                        epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL);
                        close(fd); cn->state = ST_DONE; cn->fd = 0; failed++;
                    }
                }
            }
        }
    }

    clock_gettime(CLOCK_MONOTONIC, &t1);
    close(epfd);

    /* ── Reap clients ── */
    int client_ok = 0;
    for (int i = 0; i < NCLIENTS; i++) {
        int st;
        if (waitpid(-1, &st, 0) > 0 && WIFEXITED(st) && WEXITSTATUS(st) == 0)
            client_ok++;
    }

    double dt_ms = (t1.tv_sec - t0.tv_sec) * 1000.0
                 + (t1.tv_nsec - t0.tv_nsec) / 1e6;
    double total_bytes = (double)NCLIENTS * (double)PAYLOAD * 2.0;
    double mbytes = total_bytes / (1024.0 * 1024.0);
    double thr_mbps = (dt_ms > 0) ? mbytes / (dt_ms / 1000.0) : 0;

    printf("  accepted=%d echoed=%d failed=%d  clients_ok=%d/%d\n",
           accepted, echoed, failed, client_ok, NCLIENTS);
    printf("  epoll: %d wakeups, %d reads, %d writes, %.1f events/wake\n",
           wakeups, read_calls, write_calls,
           wakeups > 0 ? (double)(read_calls + write_calls) / (double)wakeups : 0.0);
    printf("  time=%.1f ms  throughput=%.2f MB/s\n", dt_ms, thr_mbps);

    if (echoed == NCLIENTS && client_ok == NCLIENTS) {
        printf("PASS: epoll concurrent echo (%d clients, single-threaded event loop)\n",
               NCLIENTS);
        return 0;
    }
    printf("FAIL: epoll echo (echoed=%d client_ok=%d stall=%d)\n",
           echoed, client_ok, stall);
    return 1;
}
