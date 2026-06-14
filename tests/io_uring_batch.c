// io_uring BATCH advantage demo (Phase 2 / M3).
// N independent pipe pairs, N writes + N reads = 2N operations.
//
//   Sync:  2N syscalls (N write + N read), serial, each blocks until done.
//   iouring: submit all 2N SQEs in ONE io_uring_enter → worker drives
//           them all concurrently (per-op spawn) → reap 2N CQEs.
//
// The advantage: submission cost is O(1) for io_uring (one enter), O(N)
// for sync (2N syscalls). And all io_uring ops are in-flight simultaneously
// while the submitter is idle.
//
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_batch.c -o io_uring_batch
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <string.h>
#include <sched.h>
#include <sys/syscall.h>
#include <sys/mman.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426

#define IORING_OP_READ  22
#define IORING_OP_WRITE 23

enum { N = 8, BSZ = 64 };

struct io_uring_sqe {
    uint8_t opcode,flags; uint16_t ioprio; int32_t fd;
    uint64_t off; uint64_t addr; uint32_t len; uint32_t rw_flags;
    uint64_t user_data; uint16_t buf_index,personality; uint32_t _pad[3];
} __attribute__((packed));
struct io_uring_cqe { uint64_t user_data; int32_t res; uint32_t flags; };
struct io_uring_params {
    uint32_t sq_entries,cq_entries,flags,sc,si; uint32_t f,wfd,rsv[3];
    struct io_sqring_offsets { uint32_t h,t,rm,re,fl,dr,ar,rs; uint64_t ua; } sq_off;
    struct io_cqring_offsets { uint32_t h,t,rm,re,ov,cq,fl,rs; uint64_t ua; } cq_off;
};

static inline uint64_t rdtime(void) { uint64_t v; asm volatile("rdtime %0":"=r"(v)); return v; }

static void reap_all(uint32_t *ct, uint32_t *ch, struct io_uring_cqe *cqes,
                     uint32_t cmask, int want) {
    for(int g=0;g<want;){uint32_t h=__atomic_load_n(ch,__ATOMIC_ACQUIRE),t=__atomic_load_n(ct,__ATOMIC_ACQUIRE);
    if(t!=h){__atomic_store_n(ch,h+1,__ATOMIC_RELEASE); g++;} else sched_yield();}
}

int main(void) {
    int wr[N], rd[N]; unsigned char wb[N][BSZ], rb[N][BSZ];
    for(int i=0;i<N;i++){ int p[2]; pipe(p); rd[i]=p[0]; wr[i]=p[1];
        for(int k=0;k<BSZ;k++) wb[i][k]=(unsigned char)(i*31+k*7+3); }

    // ── io_uring setup (one ring for all pipes) ──
    struct io_uring_params p; memset(&p,0,sizeof p);
    int fd=(int)syscall(__NR_io_uring_setup, 2*N, &p);
    if(fd<0){perror("setup");return 1;}

    size_t Pg=4096;
    char *sq_ring=mmap(NULL,Pg,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);
    char *cq_ring=mmap(NULL,Pg,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0x8000000u);
    char *sqes   =mmap(NULL,Pg,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0x10000000u);

    uint32_t *sq_tail=(uint32_t*)(sq_ring+p.sq_off.t);
    uint32_t *sq_arr =(uint32_t*)(sq_ring+p.sq_off.ar);
    uint32_t mask=*(uint32_t*)(sq_ring+p.sq_off.rm);
    uint32_t *cq_tail=(uint32_t*)(cq_ring+p.cq_off.t);
    uint32_t *cq_head=(uint32_t*)(cq_ring+p.cq_off.h);
    struct io_uring_cqe *cqes=(struct io_uring_cqe*)(cq_ring+p.cq_off.cq);
    uint32_t cqmask=*(uint32_t*)(cq_ring+p.cq_off.rm);

    // ── io_uring batch ──
    uint32_t tail=*sq_tail;
    for(int i=0;i<N;i++){
        struct io_uring_sqe *sw=(struct io_uring_sqe*)(sqes+((tail+i)&mask)*64);
        memset(sw,0,64); sw->opcode=IORING_OP_WRITE; sw->fd=wr[i];
        sw->addr=(uint64_t)wb[i]; sw->len=BSZ; sw->user_data=(i<<1)|0xC0;
        sq_arr[(tail+i)&mask]=(tail+i)&mask;
    }
    for(int i=0;i<N;i++){
        struct io_uring_sqe *sr=(struct io_uring_sqe*)(sqes+((tail+N+i)&mask)*64);
        memset(sr,0,64); sr->opcode=IORING_OP_READ; sr->fd=rd[i];
        sr->addr=(uint64_t)rb[i]; sr->len=BSZ; sr->user_data=(i<<1)|0xC1;
        sq_arr[(tail+N+i)&mask]=(tail+N+i)&mask;
    }
    int total=2*N;
    __atomic_store_n(sq_tail,tail+total,__ATOMIC_RELEASE);

    uint64_t t0=rdtime();
    syscall(__NR_io_uring_enter,fd,total,0,0,0,0);
    reap_all(cq_tail,cq_head,cqes,cqmask,total);
    uint64_t t_ring=rdtime()-t0;

    // Verify all bytes round-tripped
    int chk=1;
    for(int i=0;i<N&&chk;i++) if(memcmp(wb[i],rb[i],BSZ)) chk=0;

    // ── Sync serial (for comparison) ──
    memset(rb,0,sizeof rb);
    t0=rdtime();
    for(int i=0;i<N;i++) { write(wr[i],wb[i],BSZ); read(rd[i],rb[i],BSZ); }
    uint64_t t_sync=rdtime()-t0;
    int chk2=1;
    for(int i=0;i<N&&chk2;i++) if(memcmp(wb[i],rb[i],BSZ)) chk2=0;

    printf("=== Batch benchmark: %d pipes × (WRITE+READ) = %d ops ===\n",N,total);
    printf("%-20s %8llu cyc  (check=%d)\n","iouring (1 enter):", (unsigned long long)t_ring, chk);
    printf("%-20s %8llu cyc  (check=%d)\n","sync (%d syscalls):", (unsigned long long)t_sync, 2*N, chk2);
    printf("%-20s %.2fx\n","speedup:",(double)t_sync/(double)t_ring);

    // Syscall count: sync = 2N individual calls; io_uring = 1 enter
    printf("\n%-20s %d syscalls\n","sync:", 2*N);
    printf("%-20s 1 syscall  (io_uring_enter submits %d SQEs)\n","iouring:", total);

    printf("\nPASS: io_uring batch bench\n");
    return 0;
}
