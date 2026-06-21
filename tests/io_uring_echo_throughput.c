// io_uring vs epoll — sustained throughput benchmark.
//
// Each of N clients sends R echo rounds (send → recv → verify) to a single
// server (io_uring or epoll, selectable). Measures wall-clock req/s.
// Designed to mirror the io_uring-echo-server / rust_echo_bench methodology
// but integrated (server + clients in one binary) for the QEMU batch setup.
//
// Lessons from the broken io_uring_echo_bench.c:
//   - 4 initial accepts (not 8 — 8 caused accept-pipeline issues).
//   - Graceful close: io_uring = shutdown(SHUT_WR)+close; epoll = shutdown+drain.
//   - Accumulate partial reads to msglen before echoing.
//   - Accumulate partial writes.
//   - Wall-clock no-progress guard (not iteration counter).
//   - Fair epoll: EPOLLIN-only + inline write (no EPOLLOUT busy-loop).
//
// Usage: io_uring_echo_throughput <iouring|epoll> <N> <R> <msg_len>
//
//   riscv64-linux-musl-gcc -static -O2 -Itests/liburing-shim \
//       tests/io_uring_echo_throughput.c tests/liburing-shim/liburing_shim.c \
//       -o io_uring_echo_throughput
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
#include "liburing_shim.h"

#define PORT        7810
#define MAXCONN     2048
#define BACKLOG     2048
#define MAXEVENTS   256
#define ENTRIES     512

enum { T_ACCEPT = 0, T_RECV = 1, T_SEND = 2, T_DRAIN = 3 };

struct conn { int fd, state, slot, remaining, send_total, rounds_left, msglen; };
static struct conn   *conns;
static unsigned char **bufs;   // per-slot buffers (dynamic, msglen)
static int            max_conn = 256;
static int            payload;

static void set_nonblocking(int fd) {
    int fl = fcntl(fd, F_GETFL, 0);
    if (fl >= 0) fcntl(fd, F_SETFL, fl | O_NONBLOCK);
}
static int find_slot(void) {
    for (int i = 0; i < max_conn; i++) if (conns[i].fd == 0) return i;
    return -1;
}

/* ── io_uring server (graceful: shutdown+close) ── */
static int server_iouring(int lfd, int N, int R, int msglen) {
    struct io_uring ring;
    if (io_uring_queue_init(ENTRIES, &ring, 0) < 0) return -1;

    int accept_queued = 0;
    for (int i = 0; i < 4 && accept_queued < N; i++) {
        struct io_uring_sqe *s = io_uring_get_sqe(&ring);
        if (!s) break;
        io_uring_prep_accept(s, lfd, NULL, NULL, 0);
        s->user_data = T_ACCEPT; accept_queued++;
    }

    int accepted = 0, completed = 0, failed = 0, total = N * R;
    struct timespec prog; clock_gettime(CLOCK_MONOTONIC, &prog);

    while (completed + failed < total) {
        io_uring_submit_and_wait(&ring, 1);
        struct io_uring_cqe *cqes[MAXEVENTS];
        int nr = io_uring_peek_batch_cqe(&ring, cqes, MAXEVENTS);
        if (nr == 0) {
            struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now);
            double idle = (now.tv_sec-prog.tv_sec) + (now.tv_nsec-prog.tv_nsec)/1e9;
            if (idle > 30.0) break;
            continue;
        }
        clock_gettime(CLOCK_MONOTONIC, &prog);
        for (int i = 0; i < nr; i++) {
            struct io_uring_cqe *c = cqes[i];
            uint64_t ud = c->user_data; int res = c->res;
            if (ud == T_ACCEPT) {
                if (res < 0) { io_uring_cqe_seen(&ring,c); continue; }
                int slot = find_slot();
                if (slot < 0) { close(res); io_uring_cqe_seen(&ring,c); continue; }
                conns[slot].fd=res; conns[slot].state=T_RECV;
                conns[slot].slot=slot; conns[slot].rounds_left=R;
                conns[slot].remaining=0; conns[slot].msglen=msglen;
                accepted++;
                struct io_uring_sqe *s=io_uring_get_sqe(&ring);
                if(s){ io_uring_prep_recv(s,res,bufs[slot],msglen,0); io_uring_sqe_set_data(s,&conns[slot]); }
                if (accept_queued < N) {
                    struct io_uring_sqe *sa=io_uring_get_sqe(&ring);
                    if(sa){ io_uring_prep_accept(sa,lfd,NULL,NULL,0); sa->user_data=T_ACCEPT; accept_queued++; }
                }
            } else {
                struct conn *cn=(struct conn*)(uintptr_t)ud; int slot=cn->slot;
                if (cn->state == T_RECV) {
                    if (res<=0){ shutdown(cn->fd,SHUT_RDWR); close(cn->fd); cn->fd=0; failed+=cn->rounds_left; }
                    else {
                        cn->remaining += res;
                        if (cn->remaining >= cn->msglen) {
                            cn->state=T_SEND; cn->send_total=cn->msglen; cn->remaining=0;
                            struct io_uring_sqe *s=io_uring_get_sqe(&ring);
                            if(s){ io_uring_prep_send(s,cn->fd,bufs[slot],cn->msglen,0); io_uring_sqe_set_data(s,cn); }
                        } else {
                            struct io_uring_sqe *s=io_uring_get_sqe(&ring);
                            if(s){ io_uring_prep_recv(s,cn->fd,bufs[slot]+cn->remaining,cn->msglen-cn->remaining,0); io_uring_sqe_set_data(s,cn); }
                        }
                    }
                } else if (cn->state == T_SEND) {
                    if (res<=0){ shutdown(cn->fd,SHUT_RDWR); close(cn->fd); cn->fd=0; failed+=cn->rounds_left; }
                    else {
                        cn->remaining += res;
                        if (cn->remaining >= cn->send_total) {
                            completed++; cn->rounds_left--;
                            if (cn->rounds_left > 0) {
                                cn->state=T_RECV; cn->remaining=0;
                                struct io_uring_sqe *s=io_uring_get_sqe(&ring);
                                if(s){ io_uring_prep_recv(s,cn->fd,bufs[slot],cn->msglen,0); io_uring_sqe_set_data(s,cn); }
                            } else {
                                shutdown(cn->fd, SHUT_WR);  /* graceful: FIN after data */
                                close(cn->fd); cn->fd=0;
                            }
                        } else {
                            struct io_uring_sqe *s=io_uring_get_sqe(&ring);
                            if(s){ io_uring_prep_send(s,cn->fd,bufs[slot]+cn->remaining,cn->send_total-cn->remaining,0); io_uring_sqe_set_data(s,cn); }
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

/* ── fair epoll server (EPOLLIN-only, inline write, graceful drain) ── */
static int server_epoll(int lfd, int N, int R, int msglen) {
    int epfd = epoll_create1(0);
    struct epoll_event ev; ev.events=EPOLLIN; ev.data.fd=lfd;
    epoll_ctl(epfd, EPOLL_CTL_ADD, lfd, &ev);

    int accepted = 0, completed = 0, failed = 0, total = N * R;
    struct timespec prog; clock_gettime(CLOCK_MONOTONIC, &prog);

    while (completed + failed < total) {
        struct epoll_event events[MAXEVENTS];
        int nfds = epoll_wait(epfd, events, MAXEVENTS, 50);
        if (nfds <= 0) {
            struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now);
            double idle = (now.tv_sec-prog.tv_sec) + (now.tv_nsec-prog.tv_nsec)/1e9;
            if (idle > 30.0) break;
            continue;
        }
        clock_gettime(CLOCK_MONOTONIC, &prog);
        for (int i=0; i<nfds; i++) {
            int fd=events[i].data.fd;
            if (fd==lfd) {
                int cfd;
                while (accepted < N && (cfd=accept(lfd,NULL,NULL))>=0) {
                    int slot=find_slot(); if(slot<0){ close(cfd); break; }
                    set_nonblocking(cfd);
                    conns[slot].fd=cfd; conns[slot].state=T_RECV; conns[slot].slot=slot;
                    conns[slot].rounds_left=R; conns[slot].remaining=0; conns[slot].msglen=msglen;
                    accepted++;
                    ev.events=EPOLLIN; ev.data.fd=cfd;
                    epoll_ctl(epfd,EPOLL_CTL_ADD,cfd,&ev);
                }
            } else {
                int ci=-1;
                for(int j=0;j<max_conn;j++) if(conns[j].fd==fd){ ci=j; break; }
                if (ci<0) continue;
                struct conn *cn=&conns[ci];
                if (cn->state == T_RECV) {
                    int n=read(fd,bufs[ci]+cn->remaining,cn->msglen-cn->remaining);
                    if (n>0) {
                        cn->remaining+=n;
                        if (cn->remaining>=cn->msglen) {
                            /* inline echo write */
                            int sent=0;
                            while(sent<cn->msglen){ int w=write(fd,bufs[ci]+sent,cn->msglen-sent); if(w>0)sent+=w; else if(w<0&&errno==EAGAIN){ usleep(1000); continue; } else break; }
                            if (sent>=cn->msglen) {
                                completed++; cn->rounds_left--;
                                if (cn->rounds_left>0){ cn->state=T_RECV; cn->remaining=0; }
                                else {
                                    /* graceful drain: shutdown write, wait client EOF */
                                    shutdown(fd,SHUT_WR); cn->state=T_DRAIN;
                                }
                            } else { epoll_ctl(epfd,EPOLL_CTL_DEL,fd,NULL); close(fd); cn->fd=0; failed+=cn->rounds_left; }
                        }
                    } else if (n==0||(n<0&&errno!=EAGAIN)) { epoll_ctl(epfd,EPOLL_CTL_DEL,fd,NULL); close(fd); cn->fd=0; failed+=cn->rounds_left; }
                } else if (cn->state == T_DRAIN) {
                    char tmp[256]; int n=read(fd,tmp,sizeof tmp);
                    if (n<=0){ epoll_ctl(epfd,EPOLL_CTL_DEL,fd,NULL); close(fd); cn->fd=0; }
                    /* n>0: ignore, keep draining */
                }
            }
        }
    }
    close(epfd);
    return completed;
}

int main(int argc, char **argv) {
    if (argc < 5) { printf("Usage: %s <iouring|epoll> <N> <R> <msg_len>\n",argv[0]); return 2; }
    int is_iouring = (strcmp(argv[1],"iouring")==0);
    int N=atoi(argv[2]), R=atoi(argv[3]), msglen=atoi(argv[4]);
    if(N<=0)N=32; if(R<=0)R=100; if(msglen<16||msglen>4096)msglen=128;
    max_conn = N + 16;
    conns = calloc(max_conn, sizeof(*conns));
    bufs  = malloc(max_conn * sizeof(*bufs));
    for (int i=0;i<max_conn;i++){ bufs[i]=malloc(msglen); memset(bufs[i],0,msglen); }
    payload = msglen;

    int lfd=socket(AF_INET,SOCK_STREAM,0);
    struct sockaddr_in addr; memset(&addr,0,sizeof addr);
    addr.sin_family=AF_INET; addr.sin_port=htons(PORT); addr.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
    int opt=1; setsockopt(lfd,SOL_SOCKET,SO_REUSEADDR,&opt,sizeof opt);
    if (is_iouring){} else set_nonblocking(lfd);
    bind(lfd,(struct sockaddr*)&addr,sizeof addr); listen(lfd,BACKLOG);

    /* Fork N clients — diagnostic exit codes:
     * 0=ok  2=write fail  3=EOF before full  4=read err(RST)  5=mismatch */
    for (int i=0;i<N;i++) {
        pid_t p=fork();
        if(p==0){
            int cfd=socket(AF_INET,SOCK_STREAM,0);
            struct sockaddr_in sa; memset(&sa,0,sizeof sa);
            sa.sin_family=AF_INET; sa.sin_port=htons(PORT); sa.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
            for(int t=0;t<8000&&connect(cfd,(struct sockaddr*)&sa,sizeof sa)!=0;t++) usleep(500);
            unsigned char *out=malloc(msglen),*in=malloc(msglen);
            for (int r=0;r<R;r++){
                for(int j=0;j<msglen;j++)out[j]=(i*R+r+j)%26+'A';
                out[msglen-1]='0'+(i%10);
                { int sent=0; while(sent<msglen){ int n=write(cfd,out+sent,msglen-sent); if(n<=0)_exit(2); sent+=n; } }
                { int got=0, eof=0; while(got<msglen){ int n=read(cfd,in+got,msglen-got); if(n>0)got+=n; else if(n==0){eof=1;break;} else _exit(4); } if(got<msglen)_exit(eof?3:4); }
                if(memcmp(in,out,msglen)!=0)_exit(5);
            }
            free(out);free(in);close(cfd);_exit(0);
        }
    }

    struct timespec t0,t1; clock_gettime(CLOCK_MONOTONIC,&t0);
    int ok=is_iouring?server_iouring(lfd,N,R,msglen):server_epoll(lfd,N,R,msglen);
    clock_gettime(CLOCK_MONOTONIC,&t1);

    int client_ok=0, tally[8]={0};
    for(int i=0;i<N;i++){int st; if(waitpid(-1,&st,0)>0&&WIFEXITED(st)){ int c=WEXITSTATUS(st); if(c==0)client_ok++; if(c>=0&&c<8)tally[c]++; else tally[1]++; }}
    double dt_s=(t1.tv_sec-t0.tv_sec)+(t1.tv_nsec-t0.tv_nsec)/1e9;
    int total=N*R;
    printf("throughput: mode=%s N=%d R=%d len=%d  echoed=%d/%d client_ok=%d/%d  time=%.3fs  req/s=%.0f  fail:W=%d E=%d RST=%d M=%d\n",
           argv[1],N,R,msglen,ok,total,client_ok,N,dt_s,dt_s>0?(double)ok/dt_s:0,
           tally[2],tally[3],tally[4],tally[5]);

    for(int i=0;i<max_conn;i++)free(bufs[i]);
    free(bufs); free(conns); close(lfd);
    return (ok==total && client_ok==N)?0:1;
}
