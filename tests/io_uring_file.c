// io_uring READ on /dev/zero — generality proof (Phase 2 / M2).
// The worker drives ANY Pollable fd via the same file.read() path. We read 4KB
// from /dev/zero and verify every byte is 0x00.
//   riscv64-linux-musl-gcc -static -O2 tests/io_uring_file.c -o io_uring_file
#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <sched.h>
#include <sys/syscall.h>
#include <sys/mman.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426
#define IORING_OP_READ 22

struct io_uring_sqe {
    uint8_t opcode,flags; uint16_t ioprio; int32_t fd;
    uint64_t off; uint64_t addr; uint32_t len; uint32_t rw_flags;
    uint64_t user_data; uint16_t buf_index,personality; uint32_t _pad[3];
} __attribute__((packed));
struct io_uring_cqe { uint64_t user_data; int32_t res; uint32_t flags; };
struct io_sqring_offsets { uint32_t head,tail,ring_mask,ring_entries,flags,dropped,array,resv1; uint64_t user_addr; };
struct io_cqring_offsets { uint32_t head,tail,ring_mask,ring_entries,overflow,cqes,flags,resv1; uint64_t user_addr; };
struct io_uring_params {
    uint32_t sq_entries,cq_entries,flags,sc,si; uint32_t f,wfd,rsv[3];
    struct io_sqring_offsets sq_off; struct io_cqring_offsets cq_off;
};

int main(void) {
    int zfd = open("/dev/zero", O_RDONLY);
    if (zfd < 0) { perror("open /dev/zero"); return 1; }

    struct io_uring_params p; memset(&p,0,sizeof p);
    int fd = (int)syscall(__NR_io_uring_setup,8,&p); if(fd<0){perror("setup");return 1;}

    size_t P=4096;
    char *sq_ring=mmap(NULL,P,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);
    char *cq_ring=mmap(NULL,P,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0x8000000u);
    char *sqes=   mmap(NULL,P,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0x10000000u);

    uint32_t *sq_tail=(uint32_t*)(sq_ring+p.sq_off.tail);
    uint32_t *sq_arr=(uint32_t*)(sq_ring+p.sq_off.array);
    uint32_t mask=*(uint32_t*)(sq_ring+p.sq_off.ring_mask);
    uint32_t *cq_tail=(uint32_t*)(cq_ring+p.cq_off.tail);
    uint32_t *cq_head=(uint32_t*)(cq_ring+p.cq_off.head);
    struct io_uring_cqe *cqes=(struct io_uring_cqe*)(cq_ring+p.cq_off.cqes);
    uint32_t cqmask=*(uint32_t*)(cq_ring+p.cq_off.ring_mask);

    static unsigned char buf[4096];
    memset(buf,0xFF,4096);

    uint32_t t = *sq_tail;
    struct io_uring_sqe *s = (struct io_uring_sqe*)(sqes+(t&mask)*64);
    memset(s,0,64); s->opcode=IORING_OP_READ; s->fd=zfd;
    s->addr=(uint64_t)buf; s->len=4096; s->user_data=0xC1;
    sq_arr[t&mask]=t&mask;
    __atomic_store_n(sq_tail,t+1,__ATOMIC_RELEASE);
    if(syscall(__NR_io_uring_enter,fd,1,0,0,0,0)<0){perror("enter");return 1;}

    int got=-1;
    for(;;){uint32_t h=__atomic_load_n(cq_head,__ATOMIC_ACQUIRE),tt=__atomic_load_n(cq_tail,__ATOMIC_ACQUIRE);
        if(tt!=h){struct io_uring_cqe*c=&cqes[h&cqmask];got=c->res;
            __atomic_store_n(cq_head,h+1,__ATOMIC_RELEASE);break;}sched_yield();}

    int ok=0;
    if(got>0){
        ok=1; for(int i=0;i<got;i++) if(buf[i]!=0){ok=0;break;}
    }
    if(ok){printf("PASS: io_uring READ /dev/zero (%dB, all zero)\n",got);return 0;}
    printf("FAIL: res=%d ok=%d\n",got,ok);return 1;
}
