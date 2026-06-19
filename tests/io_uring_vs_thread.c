// Head-to-head benchmark: io_uring echo (1 worker, O(1) memory) vs
// thread/task-per-connection echo (K handlers, O(K) memory).
//
// Same clients, same payload, same network (loopback) — the ONLY variable is
// the server's concurrency model. Measures, per mode x K:
//   - wall-clock for all K clients to complete their echo (throughput proxy)
//   - PEAK kernel memory (sum of /proc/meminfo2 categories, sampled live)
//   - PEAK task count (numeric /proc entries, sampled live)
//
// The structural claim — io_uring serves K connections on 1 worker task
// (constant ~256KB stack + ring pages), while thread-per-connection spawns K
// handler tasks (K x 256KB stacks) — is turned into measured numbers here.
//
//   riscv64-linux-musl-gcc -static -O2 -Itests/liburing-shim \
//       tests/io_uring_vs_thread.c tests/liburing-shim/liburing_shim.c -o io_uring_vs_thread
//
//   ./io_uring_vs_thread iouring 128 1024   # mode, K clients, M payload bytes
//   ./io_uring_vs_thread thread  128 1024
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <poll.h>
#include <dirent.h>
#include <time.h>
#include <errno.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include "liburing_shim.h"

#define PORT      7790
#define MAXCONN   512
#define PAYLOAD   4096

enum { T_ACCEPT = 0, T_RECV = 1, T_SEND = 2 };

struct conn {
    int used;
    int fd;
    int state;
    int recvd;    /* bytes received so far in the RECV phase */
    int send_rem; /* bytes still to send in the SEND phase */
};

/* ---- live memory / task sampling (monitor child) ---- */

/* Sum every integer token in /proc/meminfo2 → total kernel-tracked bytes. */
static size_t meminfo2_total(void) {
    int fd = open("/proc/meminfo2", O_RDONLY);
    if (fd < 0) return 0;
    char buf[512];
    ssize_t n = read(fd, buf, sizeof(buf) - 1);
    close(fd);
    if (n <= 0) return 0;
    buf[n] = 0;
    size_t total = 0, val = 0;
    int indigit = 0;
    for (ssize_t i = 0; i <= n; i++) {
        char c = buf[i];
        if (c >= '0' && c <= '9') {
            val = val * 10 + (size_t)(c - '0');
            indigit = 1;
        } else {
            if (indigit) { total += val; val = 0; indigit = 0; }
        }
    }
    return total;
}

/* Count numeric entries in /proc (one per kernel task). */
static int proc_task_count(void) {
    DIR *d = opendir("/proc");
    if (!d) return 0;
    int cnt = 0;
    struct dirent *e;
    while ((e = readdir(d))) {
        const char *s = e->d_name;
        if (*s >= '0' && *s <= '9') cnt++;
    }
    closedir(d);
    return cnt;
}

/* Monitor: sample peaks until parent writes to signal_fd (closes = done). */
static void run_monitor(int signal_fd) {
    size_t peak_mem = 0;
    int peak_tasks = 0;
    for (;;) {
        size_t m = meminfo2_total();
        int t = proc_task_count();
        if (m > peak_mem) peak_mem = m;
        if (t > peak_tasks) peak_tasks = t;
        struct pollfd p = { .fd = signal_fd, .events = POLLIN, .revents = 0 };
        int r = poll(&p, 1, 2); /* wait up to 2ms; 0 = timeout, >0 = parent done */
        if (r > 0) break;
    }
    printf("PEAK mem_used_bytes=%zu tasks=%d\n", peak_mem, peak_tasks);
    fflush(stdout);
    _exit(0);
}

/* ---- client ---- */

static void run_client(int i, int K, int M) {
    int cfd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in sa = {0};
    sa.sin_family = AF_INET;
    sa.sin_port = htons(PORT);
    sa.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    for (int t = 0; t < 20000 && connect(cfd, (struct sockaddr *)&sa, sizeof sa) != 0; t++)
        usleep(200);
    static unsigned char out[PAYLOAD];
    memset(out, 'A' + (i % 26), M);
    out[M - 1] = '0' + (i % 10);
    int sent = 0;
    while (sent < M) {
        ssize_t w = write(cfd, out + sent, M - sent);
        if (w <= 0) _exit(2);
        sent += w;
    }
    static unsigned char in[PAYLOAD];
    int got = 0;
    while (got < M) {
        ssize_t r = read(cfd, in + got, M - got);
        if (r <= 0) break;
        got += r;
    }
    int ok = (got == M && memcmp(in, out, M) == 0);
    close(cfd);
    _exit(ok ? 0 : 1);
}

/* ---- io_uring server (1 worker multiplexes all K connections) ---- */

static int server_iouring(int lfd, int K, int M) {
    /* Cap entries so the kernel's 1-page CQ ring (cqes at offset 0x18) holds:
     * pow2(entries) ≤ 128 (256 would overflow). Each conn has ≤1 outstanding
     * data op + a few ACCEPTs, so K + 16 is ample headroom. */
    int entries = K + 16;
    if (entries > 128) entries = 128;
    struct io_uring ring;
    if (io_uring_queue_init(entries, &ring, 0) < 0) return -1;

    /* Heap-allocated, K-sized buffers (not a giant static BSS): conns[] tracks
     * per-connection state; bufrow(ci) is the echo scratch for connection ci. */
    int nslots = K + 8;
    struct conn *conns = calloc(nslots, sizeof(struct conn));
    unsigned char *bufpool = calloc((size_t)nslots, M);
    if (!conns || !bufpool) {
        io_uring_queue_exit(&ring);
        return -1;
    }
    /* Force-fault every page: PinnedUserBuf::resolve walks the page table
     * without faulting, so untouched (COW-zero) pages fail to pin. */
    memset(bufpool, 0, (size_t)nslots * M);

#define BUFROW(ci) (bufpool + (size_t)(ci) * M)

    int echoed = 0, failed = 0, accept_queued = 0;
    for (int i = 0; i < 8 && accept_queued < K; i++) {
        struct io_uring_sqe *s = io_uring_get_sqe(&ring);
        io_uring_prep_accept(s, lfd, NULL, NULL, 0);
        s->user_data = T_ACCEPT;
        accept_queued++;
    }

    while (echoed + failed < K) {
        io_uring_submit_and_wait(&ring, 1);
        struct io_uring_cqe *cqes[128];
        int nr = io_uring_peek_batch_cqe(&ring, cqes, entries);
        for (int i = 0; i < nr; i++) {
            struct io_uring_cqe *c = cqes[i];
            uint64_t ud = c->user_data;
            int res = c->res;
            if (ud == T_ACCEPT) {
                if (res < 0) { io_uring_cqe_seen(&ring, c); continue; }
                int ci = -1;
                for (int j = 1; j < nslots; j++) if (!conns[j].used) { ci = j; break; }
                if (ci < 0) { close(res); io_uring_cqe_seen(&ring, c); continue; }
                conns[ci].used = 1;
                conns[ci].fd = res;
                conns[ci].state = T_RECV;
                conns[ci].recvd = 0;
                struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                if (s) { io_uring_prep_recv(s, res, BUFROW(ci), M, 0); s->user_data = ci; }
                if (accept_queued < K) {
                    struct io_uring_sqe *sa = io_uring_get_sqe(&ring);
                    if (sa) { io_uring_prep_accept(sa, lfd, NULL, NULL, 0); sa->user_data = T_ACCEPT; }
                    accept_queued++;
                }
            } else {
                int ci = (int)ud;
                if (ci < 1 || ci >= nslots) { io_uring_cqe_seen(&ring, c); continue; }
                struct conn *cn = &conns[ci];
                if (cn->state == T_RECV) {
                    if (res <= 0) { close(cn->fd); cn->used = 0; failed++; }
                    else {
                        cn->recvd += res;
                        if (cn->recvd < M) {
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) { io_uring_prep_recv(s, cn->fd, BUFROW(ci) + cn->recvd,
                                                        M - cn->recvd, 0); s->user_data = ci; }
                        } else {
                            cn->state = T_SEND;
                            cn->send_rem = M;
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) { io_uring_prep_send(s, cn->fd, BUFROW(ci), M, 0); s->user_data = ci; }
                        }
                    }
                } else { /* T_SEND */
                    if (res <= 0) { close(cn->fd); cn->used = 0; failed++; }
                    else {
                        cn->send_rem -= res;
                        if (cn->send_rem <= 0) {
                            echoed++;
                            close(cn->fd);
                            cn->used = 0;
                        } else {
                            int off = M - cn->send_rem;
                            struct io_uring_sqe *s = io_uring_get_sqe(&ring);
                            if (s) { io_uring_prep_send(s, cn->fd, BUFROW(ci) + off,
                                                        cn->send_rem, 0); s->user_data = ci; }
                        }
                    }
                }
            }
            io_uring_cqe_seen(&ring, c);
        }
    }
#undef BUFROW
    io_uring_queue_exit(&ring);
    free(conns);
    free(bufpool);
    return echoed;
}

/* ---- thread/task-per-connection server (fork a handler per connection) ---- */

static void handler_child(int cfd, int M) {
    static unsigned char buf[PAYLOAD];
    int got = 0;
    while (got < M) {
        ssize_t r = read(cfd, buf + got, M - got);
        if (r <= 0) _exit(1);
        got += r;
    }
    int sent = 0;
    while (sent < M) {
        ssize_t w = write(cfd, buf + sent, M - sent);
        if (w <= 0) _exit(1);
        sent += w;
    }
    close(cfd);
    _exit(0);
}

static int server_thread(int lfd, int K, int M) {
    static pid_t hpids[MAXCONN];
    int accepted = 0, ok = 0;
    while (accepted < K) {
        int cfd = accept(lfd, NULL, NULL);
        if (cfd < 0) {
            if (errno == EINTR) continue;
            break;
        }
        pid_t pid = fork();
        if (pid == 0) { /* handler task: blocking recv/send */ handler_child(cfd, M); }
        close(cfd); /* parent doesn't need the conn fd */
        if (accepted < MAXCONN) hpids[accepted] = pid;
        accepted++;
    }
    /* Reap handlers by SPECIFIC pid — waitpid(-1) would race with main() reaping
     * the clients (both loops would steal each other's children). main() reaps
     * only clients afterward (handlers are already gone). */
    for (int i = 0; i < accepted && i < MAXCONN; i++) {
        int st;
        if (waitpid(hpids[i], &st, 0) == hpids[i] && WIFEXITED(st) && WEXITSTATUS(st) == 0)
            ok++;
    }
    return ok;
}

/* ---- main harness ---- */

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0); /* unbuffered: survive crashes for debug */
    if (argc < 4) {
        printf("usage: %s <iouring|thread> <K clients> <M payload bytes>\n", argv[0]);
        return 2;
    }
    const char *mode = argv[1];
    int K = atoi(argv[2]);
    int M = atoi(argv[3]);
    if (K <= 0 || K > MAXCONN) K = MAXCONN;
    if (M <= 0 || M > PAYLOAD) M = PAYLOAD;
    int is_iouring = (strcmp(mode, "iouring") == 0);
    int lfd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in addr = {0};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(PORT);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int opt = 1;
    setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof opt);
    if (bind(lfd, (struct sockaddr *)&addr, sizeof addr) < 0) { perror("bind"); return 1; }
    if (listen(lfd, K + 16) < 0) { perror("listen"); return 1; }

    /* monitor child: samples peak mem + tasks until signaled. */
    int pipefd[2];
    if (pipe(pipefd) < 0) { pipefd[0] = pipefd[1] = -1; }
    pid_t mon = -1;
    if (pipefd[0] >= 0) {
        mon = fork();
        if (mon == 0) { close(pipefd[1]); run_monitor(pipefd[0]); }
        close(pipefd[0]);
    }

    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    /* fork K clients (identical for both modes). */
    for (int i = 0; i < K; i++) {
        pid_t pid = fork();
        if (pid == 0) run_client(i, K, M);
    }

    /* serve with the selected concurrency model. */
    int ok = is_iouring ? server_iouring(lfd, K, M) : server_thread(lfd, K, M);

    /* reap clients. */
    int client_ok = 0;
    for (int i = 0; i < K; i++) {
        int st;
        if (waitpid(-1, &st, 0) > 0 && WIFEXITED(st) && WEXITSTATUS(st) == 0) client_ok++;
    }

    clock_gettime(CLOCK_MONOTONIC, &t1);
    double dt_ms = (t1.tv_sec - t0.tv_sec) * 1000.0 + (t1.tv_nsec - t0.tv_nsec) / 1e6;
    double total_bytes = (double)K * (double)M * 2.0; /* both directions */
    double mbytes = total_bytes / (1024.0 * 1024.0);
    double thr_mbps = (dt_ms > 0) ? mbytes / (dt_ms / 1000.0) : 0;

    /* signal monitor to flush peaks. */
    if (pipefd[1] >= 0) { write(pipefd[1], "x", 1); close(pipefd[1]); }
    if (mon > 0) { int st; waitpid(mon, &st, 0); }

    printf("RESULT mode=%s K=%d M=%d echoed_ok=%d clients_ok=%d/%d "
           "time_ms=%.1f throughput_MBps=%.2f total_MB=%.3f\n",
           mode, K, M, ok, client_ok, K, dt_ms, thr_mbps, mbytes);
    if (ok == K && client_ok == K) return 0;
    return 1;
}
