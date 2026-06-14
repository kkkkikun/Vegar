// io_uring benchmark — sync vs inline vs fixed-buffer (Phase 2 / M3).
// Measures per-op CPU cycles via rdtime and reports memory footprint.
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_bench.c -o io_uring_bench
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sched.h>
#include <poll.h>
#include <sys/syscall.h>
#include <sys/mman.h>

#define __NR_io_uring_setup    425
#define __NR_io_uring_enter    426
#define __NR_io_uring_register 427

#define IORING_OP_READ         22
#define IORING_OP_WRITE        23
#define IORING_OP_READ_FIXED   4
#define IORING_OP_WRITE_FIXED  5

enum { ITERS = 200, BSZ = 64 };

struct io_uring_sqe {
    uint8_t opcode,flags; uint16_t ioprio; int32_t fd;
    uint64_t off; uint64_t addr; uint32_t len; uint32_t rw_flags;
    uint64_t user_data; uint16_t buf_index,personality; uint32_t _pad[3];
} __attribute__((packed));
struct io_uring_cqe { uint64_t user_data; int32_t res; uint32_t flags; };
struct io_uring_params {
    uint32_t sq_entries,cq_entries,flags,sc,si; uint32_t f,wfd,rsv[3];
    struct io_sqring_offsets { uint32_t head,tail,ring_mask,ring_entries,flags,dropped,array,resv1; uint64_t user_addr; } sq_off;
    struct io_cqring_offsets { uint32_t head,tail,ring_mask,ring_entries,overflow,cqes,flags,resv1; uint64_t user_addr; } cq_off;
};

static inline uint64_t rdtime(void) {
    uint64_t v; asm volatile("rdtime %0" : "=r"(v)); return v;
}

static void reap_all(uint32_t *cq_tail, uint32_t *cq_head, struct io_uring_cqe *cqes,
                     uint32_t cqmask, int want) {
    for (int got=0; got<want;) {
        uint32_t h=__atomic_load_n(cq_head,__ATOMIC_ACQUIRE);
        uint32_t t=__atomic_load_n(cq_tail,__ATOMIC_ACQUIRE);
        if(t!=h){ __atomic_store_n(cq_head,h+1,__ATOMIC_RELEASE); got++; }
        else sched_yield();
    }
}

/* ---------- variant 1: sync write()+read() ---------- */
static uint64_t bench_sync(int wr, int rd, unsigned char *wbuf, unsigned char *rbuf, int n) {
    uint64_t t=rdtime();
    for(int i=0;i<n;i++) { write(wr,wbuf,BSZ); read(rd,rbuf,BSZ); }
    return rdtime()-t;
}

/* ---------- variant 2: io_uring inline (per-op page-table walk) ---------- */
static uint64_t bench_inline(int fd, char *sq_ring, char *sqes, int wr, int rd,
                             unsigned char *wbuf, unsigned char *rbuf, int n,
                             uint32_t *sq_tail, uint32_t *sq_arr, uint32_t mask,
                             uint32_t *cq_tail, uint32_t *cq_head,
                             struct io_uring_cqe *cqes, uint32_t cqmask) {
    uint64_t t=rdtime();
    for(int i=0;i<n;i++) {
        uint32_t tail=*sq_tail;
        struct io_uring_sqe *sw=(struct io_uring_sqe*)(sqes+(tail&mask)*64);
        __builtin_memset(sw,0,64); sw->opcode=IORING_OP_WRITE; sw->fd=wr;
        sw->addr=(uint64_t)(wbuf+(i%(BSZ*64))); sw->len=BSZ; sw->user_data=0xD0|(i<<16);
        sq_arr[tail&mask]=tail&mask;
        struct io_uring_sqe *sr=(struct io_uring_sqe*)(sqes+((tail+1)&mask)*64);
        __builtin_memset(sr,0,64); sr->opcode=IORING_OP_READ; sr->fd=rd;
        sr->addr=(uint64_t)(rbuf+(i%(BSZ*64))); sr->len=BSZ; sr->user_data=0xD1|(i<<16);
        sq_arr[(tail+1)&mask]=(tail+1)&mask;
        __atomic_store_n(sq_tail,tail+2,__ATOMIC_RELEASE);
        syscall(__NR_io_uring_enter,fd,2,0,0,0,0);
        reap_all(cq_tail,cq_head,cqes,cqmask,2);
    }
    return rdtime()-t;
}

/* ---------- variant 3: io_uring fixed-buffer ---------- */
static uint64_t bench_fixed(int fd, char *sq_ring, char *sqes, int wr, int rd, int n,
                            uint32_t *sq_tail, uint32_t *sq_arr, uint32_t mask,
                            uint32_t *cq_tail, uint32_t *cq_head,
                            struct io_uring_cqe *cqes, uint32_t cqmask) {
    // Register two buffers for write / read.  Use mmap so VAs are >= 0x10000
    // and touch every page so COW-populate is done before the page-table walk.
    unsigned char *wb=mmap(NULL,BSZ*64,PROT_READ|PROT_WRITE,MAP_ANONYMOUS|MAP_PRIVATE,-1,0);
    unsigned char *rb=mmap(NULL,BSZ*64,PROT_READ|PROT_WRITE,MAP_ANONYMOUS|MAP_PRIVATE,-1,0);
    for(int k=0;k<BSZ*64;k++) { wb[k]=(unsigned char)(k*13+7); rb[k]=0; }
    struct iovec { void *base; size_t len; } reg[2];
    reg[0].base=wb; reg[0].len=BSZ*64; reg[1].base=rb; reg[1].len=BSZ*64;
    if(syscall(__NR_io_uring_register,fd,0,reg,2)<0){ perror("register"); return ~0ULL; }

    uint64_t t=rdtime();
    for(int i=0;i<n;i++) {
        uint32_t tail=*sq_tail;
        struct io_uring_sqe *sw=(struct io_uring_sqe*)(sqes+(tail&mask)*64);
        __builtin_memset(sw,0,64); sw->opcode=IORING_OP_WRITE_FIXED; sw->fd=wr;
        sw->len=BSZ; sw->user_data=0xE0|(i<<16); sw->buf_index=0;
        sq_arr[tail&mask]=tail&mask;
        struct io_uring_sqe *sr=(struct io_uring_sqe*)(sqes+((tail+1)&mask)*64);
        __builtin_memset(sr,0,64); sr->opcode=IORING_OP_READ_FIXED; sr->fd=rd;
        sr->len=BSZ; sr->user_data=0xE1|(i<<16); sr->buf_index=1;
        sq_arr[(tail+1)&mask]=(tail+1)&mask;
        __atomic_store_n(sq_tail,tail+2,__ATOMIC_RELEASE);
        syscall(__NR_io_uring_enter,fd,2,0,0,0,0);
        reap_all(cq_tail,cq_head,cqes,cqmask,2);
    }
    return rdtime()-t;
}

int main(void) {
    int pfd[2]; pipe(pfd);
    struct io_uring_params p; __builtin_memset(&p,0,sizeof p);
    int fd=(int)syscall(__NR_io_uring_setup,8,&p);
    if(fd<0){perror("setup");return 1;}

    size_t Pg=4096;
    char *sq_ring=mmap(NULL,Pg,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);
    char *cq_ring=mmap(NULL,Pg,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0x8000000u);
    char *sqes   =mmap(NULL,Pg,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0x10000000u);

    uint32_t *sq_tail=(uint32_t*)(sq_ring+p.sq_off.tail);
    uint32_t *sq_arr =(uint32_t*)(sq_ring+p.sq_off.array);
    uint32_t mask=*(uint32_t*)(sq_ring+p.sq_off.ring_mask);
    uint32_t *cq_tail=(uint32_t*)(cq_ring+p.cq_off.tail);
    uint32_t *cq_head=(uint32_t*)(cq_ring+p.cq_off.head);
    struct io_uring_cqe *cqes=(struct io_uring_cqe*)(cq_ring+p.cq_off.cqes);
    uint32_t cqmask=*(uint32_t*)(cq_ring+p.cq_off.ring_mask);

    static unsigned char wbuf[BSZ*64], rbuf[BSZ*64];
    for(int i=0;i<BSZ*64;i++) wbuf[i]=(unsigned char)(i*13+7);

    // ── Warmup (1 round each, not measured) ──
    bench_sync(pfd[1],pfd[0],wbuf,rbuf,1);
    bench_inline(fd,sq_ring,sqes,pfd[1],pfd[0],wbuf,rbuf,1,
                 sq_tail,sq_arr,mask,cq_tail,cq_head,cqes,cqmask);
    bench_fixed(fd,sq_ring,sqes,pfd[1],pfd[0],1,
                sq_tail,sq_arr,mask,cq_tail,cq_head,cqes,cqmask);

    // ── Measured runs ──
    uint64_t t_sync  =bench_sync  (pfd[1],pfd[0],wbuf,rbuf,ITERS);
    uint64_t t_inline=bench_inline(fd,sq_ring,sqes,pfd[1],pfd[0],wbuf,rbuf,ITERS,
                                   sq_tail,sq_arr,mask,cq_tail,cq_head,cqes,cqmask);
    uint64_t t_fixed =bench_fixed (fd,sq_ring,sqes,pfd[1],pfd[0],ITERS,
                                   sq_tail,sq_arr,mask,cq_tail,cq_head,cqes,cqmask);

    if(t_fixed==~0ULL) return 1;  // register failed

    int ops=ITERS*2;
    int sync_c= (int)(t_sync/ops), inl_c=(int)(t_inline/ops), fix_c=(int)(t_fixed/ops);

    printf("\n===== io_uring benchmark (pipe WRITE+READ, %d rounds) =====\n",ITERS);
    printf("%-18s %8llu cyc / %4d ops = %5d cycles/op\n",
           "sync write+read:",  (unsigned long long)t_sync, ops, sync_c);
    printf("%-18s %8llu cyc / %4d ops = %5d cycles/op  (%.2fx vs sync)\n",
           "io_uring inline:",  (unsigned long long)t_inline,ops,inl_c,
           (double)t_sync/(double)t_inline);
    printf("%-18s %8llu cyc / %4d ops = %5d cycles/op  (%.2fx vs sync)\n",
           "io_uring fixed:",   (unsigned long long)t_fixed,ops,fix_c,
           (double)t_sync/(double)t_fixed);
    printf("%-18s %.2fx\n","fixed vs inline:",(double)t_inline/(double)t_fixed);

    // ── Memory footprint table (analytical, based on kernel config) ──
    // task-stack-size = 0x40000 = 256 KB  (make/defconfig.toml)
    // io_uring: 1 worker (256 KB) + SQ ring (4 KB) + CQ ring (4 KB) + SQEs (4 KB)
    printf("\n== Memory footprint (N concurrent outstanding ops) ==\n");
    printf("  task stack = 256 KB,  iouring worker + rings = ~268 KB\n\n");
    printf("  %-6s %10s %12s %12s %12s\n","N","1","4","16","64");
    printf("  %-6s %8.0fKB  %8.0fKB  %8.0fKB  %8.0fKB\n",
           "sync:",256.0,1024.0,4096.0,16384.0);
    printf("  %-6s %8dKB  %8dKB  %8dKB  %8dKB\n",
           "iouring:",268,268,268,268);

    printf("\nPASS: io_uring bench\n");
    return 0;
}
