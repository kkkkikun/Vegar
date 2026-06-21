// io_uring vs epoll — FAIR epoll variant (diagnostic for the 96-client failure).
//
// The original io_uring_echo_epoll.c registers every client fd with
// EPOLLIN|EPOLLOUT from the start and never EPOLL_CTL_MOD's. Because a TCP
// socket is almost always writable, EPOLLOUT is perpetually ready, so
// epoll_wait returns immediately every call → the server busy-loops
// (24940 wakeups at 96 clients). This file tests the hypothesis that the
// busy-loop (not epoll per se) is what breaks the 96-client case.
//
// Fair design: register EPOLLIN ONLY. When a full payload is received, echo
// it with an inline non-blocking write() right there (loopback + 64B → the
// write essentially always completes synchronously, so no EPOLLOUT needed).
// This is the standard level-triggered echo pattern and does NOT busy-loop.
// If this variant passes 96/96 while the EPOLLOUT-always-on variant fails,
// the cause is confirmed as the test's epoll registration design.
//
//   riscv64-linux-musl-gcc -static -O2 [-DNCLIENTS=96] tests/io_uring_echo_epoll_fair.c -o io_uring_echo_epoll_fair
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

#define PORT      7797
#ifndef NCLIENTS
#define NCLIENTS  24
#endif
#define PAYLOAD   64
#define MAXEVENTS 64
#define MAXCONN   (NCLIENTS + 8)

/* state: 0 = reading request, 1 = echoed + shutdown(SHUT_WR), draining to EOF */
struct conn { int fd; int recvd; int state; };
static struct conn conns[MAXCONN];
static char   bufpool[MAXCONN][PAYLOAD];

static void set_nonblocking(int fd) {
    int fl = fcntl(fd, F_GETFL, 0);
    if (fl >= 0) fcntl(fd, F_SETFL, fl | O_NONBLOCK);
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

    printf("epoll-fair echo: listening 127.0.0.1:%d, %d clients (EPOLLIN-only, inline write)\n", PORT, NCLIENTS);

    /* Fork N clients — diagnostic exit codes:
     *   0 = ok, 2 = write fail, 3 = EOF before full payload,
     *   4 = read error (e.g. ECONNRESET/RST), 5 = full bytes but memcmp mismatch.
     * Parent tallies these to pinpoint WHY clients fail. */
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
            int got = 0, eof = 0;
            while (got < PAYLOAD) {
                int n = read(cfd, in + got, PAYLOAD - got);
                if (n > 0) got += n;
                else if (n == 0) { eof = 1; break; }   /* EOF before full */
                else break;                              /* error (RST/ECONNRESET) */
            }
            if (got < PAYLOAD) _exit(eof ? 3 : 4);      /* 3=EOF short, 4=RST/error */
            if (memcmp(in, out, PAYLOAD) != 0) _exit(5);
            close(cfd);
            _exit(0);
        }
    }

    int epfd = epoll_create1(0);
    struct epoll_event ev;
    ev.events = EPOLLIN; ev.data.fd = lfd;
    epoll_ctl(epfd, EPOLL_CTL_ADD, lfd, &ev);

    int accepted = 0, echoed = 0, failed = 0, wakeups = 0, read_calls = 0, write_calls = 0;
    int stall = 0;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    while (echoed + failed < NCLIENTS) {
        struct epoll_event events[MAXEVENTS];
        int nfds = epoll_wait(epfd, events, MAXEVENTS, 50);
        wakeups++;
        if (nfds < 0) { if (errno == EINTR) continue; break; }
        if (nfds == 0) {
            if (++stall > 600) { printf("  STALL\n"); break; }
            continue;
        }
        stall = 0;
        for (int i = 0; i < nfds; i++) {
            int fd = events[i].data.fd;
            if (fd == lfd) {
                int cfd;
                while ((cfd = accept(lfd, NULL, NULL)) >= 0) {
                    if (accepted >= NCLIENTS) { close(cfd); break; }
                    int ci = -1;
                    for (int j = 1; j < MAXCONN; j++) if (conns[j].fd == 0) { ci = j; break; }
                    if (ci < 0) { close(cfd); continue; }
                    set_nonblocking(cfd);
                    conns[ci].fd = cfd; conns[ci].recvd = 0; conns[ci].state = 0;
                    ev.events = EPOLLIN; ev.data.fd = cfd;   // EPOLLIN ONLY — no EPOLLOUT
                    epoll_ctl(epfd, EPOLL_CTL_ADD, cfd, &ev);
                    accepted++;
                }
            } else {
                int ci = -1;
                for (int j = 1; j < MAXCONN; j++) if (conns[j].fd == fd) { ci = j; break; }
                if (ci < 0) continue;
                if (conns[ci].state == 0) {
                    /* Reading the request. */
                    int n = read(fd, bufpool[ci] + conns[ci].recvd, PAYLOAD - conns[ci].recvd);
                    read_calls++;
                    if (n > 0) {
                        conns[ci].recvd += n;
                        if (conns[ci].recvd >= PAYLOAD) {
                            int sent = 0;
                            while (sent < PAYLOAD) {
                                int w = write(fd, bufpool[ci] + sent, PAYLOAD - sent);
                                write_calls++;
                                if (w > 0) sent += w;
                                else if (w < 0 && errno == EAGAIN) continue;
                                else break;
                            }
                            if (sent >= PAYLOAD) {
                                echoed++;
                                /* GRACEFUL close: half-close the write side so
                                 * smoltcp flushes the echo + FIN *after* the
                                 * data, and keep the fd to drain the client's
                                 * EOF before fully closing. This avoids the RST
                                 * that the immediate-close variant hits when
                                 * close races smoltcp's lazy transmit. */
                                shutdown(fd, SHUT_WR);
                                conns[ci].state = 1;
                            } else {
                                epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL);
                                close(fd); conns[ci].fd = 0; failed++;
                            }
                        }
                    } else if (n == 0 || (n < 0 && errno != EAGAIN)) {
                        epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL);
                        close(fd); conns[ci].fd = 0; failed++;
                    }
                } else {
                    /* state 1: echoed + half-closed; drain until client EOF. */
                    char tmp[64];
                    int n = read(fd, tmp, sizeof tmp);
                    if (n <= 0) {  /* EOF (client closed) or error → fully close */
                        epoll_ctl(epfd, EPOLL_CTL_DEL, fd, NULL);
                        close(fd); conns[ci].fd = 0;
                    }
                    /* n > 0: ignore unexpected late data, keep draining */
                }
            }
        }
    }

    clock_gettime(CLOCK_MONOTONIC, &t1);
    close(epfd);

    int client_ok = 0;
    int tally[8] = {0};  // index by exit code (0,2,3,4,5); 1=sig/other bucket
    for (int i = 0; i < NCLIENTS; i++) {
        int st;
        if (waitpid(-1, &st, 0) > 0) {
            if (WIFEXITED(st)) {
                int c = WEXITSTATUS(st);
                if (c == 0) client_ok++;
                if (c >= 0 && c < 8) tally[c]++; else tally[1]++;
            } else tally[1]++;  // killed by signal
        }
    }
    printf("  client fail-mode: ok=%d write_fail=%d EOF_short=%d RST_err=%d mismatch=%d other=%d\n",
           tally[0], tally[2], tally[3], tally[4], tally[5], tally[1]);

    double dt_ms = (t1.tv_sec - t0.tv_sec) * 1000.0 + (t1.tv_nsec - t0.tv_nsec) / 1e6;
    printf("  accepted=%d echoed=%d failed=%d  clients_ok=%d/%d\n", accepted, echoed, failed, client_ok, NCLIENTS);
    printf("  epoll-fair: %d wakeups, %d reads, %d writes, %.2f io/wake\n",
           wakeups, read_calls, write_calls, wakeups > 0 ? (double)(read_calls + write_calls) / wakeups : 0.0);
    printf("  time=%.1f ms\n", dt_ms);
    if (echoed == NCLIENTS && client_ok == NCLIENTS) {
        printf("PASS: epoll-fair echo (%d clients)\n", NCLIENTS);
        return 0;
    }
    printf("FAIL: epoll-fair echo (echoed=%d client_ok=%d)\n", echoed, client_ok);
    return 1;
}
