#!/bin/sh
# Custom test groups mode
# Edit TEST_GROUPS below to choose which tests to run
# Available: basic, busybox, lua, libctest, libcbench, cyclictest, unixbench, iozone, lmbench, iperf, netperf, ltp

# =========================================================================
# EDIT THIS: Choose test groups to run
# =========================================================================
TEST_GROUPS=""

# =========================================================================
# Setup (same as init_oscomp.sh)
# =========================================================================

echo @@@@@@@@@@ setup @@@@@@@@@@

/musl/busybox mkdir -p /bin
/musl/busybox --install -s /bin
export PATH=/bin

# Verify install worked
echo "basename test: $(basename /foo/bar.txt)"
ls /bin | head -5

/musl/busybox mkdir -p /lib
/musl/busybox mkdir -p /lib/modules/10.0.0

ln -s /glibc/lib/libc.so.6 /lib/libc.so.6 2>/dev/null
ln -s /glibc/lib/libgcc_s.so.1 /lib/libgcc_s.so.1 2>/dev/null
ln -s /glibc/lib/libm.so.6 /lib/libm.so.6 2>/dev/null
ln -s /lib/libc.so.6 /lib/libc.so 2>/dev/null
ln -s /lib/libm.so.6 /lib/libm.so 2>/dev/null
if [ -f /glibc/lib/ld-linux-loongarch-lp64d.so.1 ]; then
    ln -s /musl/lib/libc.so /lib/ld-musl-loongarch-lp64d.so.1 2>/dev/null
    ln -s /glibc/lib/ld-linux-loongarch-lp64d.so.1 /lib/ld-linux-loongarch-lp64d.so.1 2>/dev/null
else
    ln -s /musl/lib/libc.so /lib/ld-musl-riscv64.so.1 2>/dev/null
    ln -s /musl/lib/libc.so /lib/ld-musl-riscv64-sf.so.1 2>/dev/null
    ln -s /glibc/lib/ld-linux-riscv64-lp64d.so.1 /lib/ld-linux-riscv64-lp64d.so.1 2>/dev/null
fi
ln -s /lib /lib64 2>/dev/null

/musl/busybox mkdir -p /usr
ln -s /lib /usr/lib64 2>/dev/null

/musl/busybox mkdir -p /boot
/musl/busybox mkdir -p /var/tmp
/musl/busybox mkdir -p /tmp
/musl/busybox mkdir -p /etc
/musl/busybox mkdir -p /sys/kernel/debug/hwpoison 2>/dev/null

echo "CONFIG_MEMORY_FAILURE=y" > /boot/config-10.0.0 2>/dev/null
echo "CONFIG_MEMORY_FAILURE=y" > /lib/modules/10.0.0/config 2>/dev/null
echo "kernel/drivers/hwpoison_inject.ko" > /lib/modules/10.0.0/modules.builtin 2>/dev/null
echo "kernel/drivers/hwpoison_inject.ko:" > /lib/modules/10.0.0/modules.dep 2>/dev/null

echo "root:x:0:0:root:/root:/bin/bash" > /etc/passwd 2>/dev/null
echo "nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin" >> /etc/passwd 2>/dev/null
echo "root:x:0:" > /etc/group 2>/dev/null
echo "users:x:100:" >> /etc/group 2>/dev/null
echo "daemon:x:2:" >> /etc/group 2>/dev/null

echo @@@@@@@@@@ setup done @@@@@@@@@@

# =========================================================================
# io_uring tests (Phase 2). Add more binaries here as M1/M2 land.
# =========================================================================
run_iouring() {
    name="$1"
    if [ -f "/$name" ]; then
        /musl/busybox cp "/$name" "/tmp/t"
        /musl/busybox chmod +x /tmp/t
        echo "===== [io_uring] running $name ====="
        /tmp/t
        echo "===== [io_uring] $name exit=$? ====="
    else
        echo "===== [io_uring] /$name NOT FOUND on disk ====="
    fi
}
run_iouring io_uring_nop
run_iouring io_uring_pipe
run_iouring io_uring_poll
run_iouring io_uring_file
run_iouring io_uring_bench
run_iouring io_uring_batch
run_iouring io_uring_scale
run_iouring io_uring_getevents
run_iouring io_uring_readv
run_iouring io_uring_shim_test
run_iouring io_uring_send_recv
run_iouring io_uring_accept
run_iouring io_uring_iodepth

# =========================================================================
# Helper: run one test with timeout
# =========================================================================
run_test() {
    runtime="$1"
    test_name="$2"
    timeout_secs="$3"
    test_script="/$runtime/${test_name}_testcode.sh"

    if [ ! -f "$test_script" ]; then
        return
    fi

    echo ">>> Running: $test_script (timeout ${timeout_secs}s)"

    cd "/$runtime"

    /musl/busybox timeout $timeout_secs /musl/busybox sh "$test_script" 2>&1
    result=$?

    cd /

    if [ $result -eq 143 ]; then
        echo "#### OS COMP TEST GROUP END ${test_name}-${runtime} ####"
        echo "<<< Timeout: $test_script"
    else
        echo "<<< Finished: $test_script (exit: $result)"
    fi

    /musl/busybox killall -9 iperf3 netserver hackbench lmbench_all 2>/dev/null
}

# =========================================================================
# LTP Test Runner
# =========================================================================
run_ltp() {
    runtime="$1"
    echo "#### OS COMP TEST GROUP START ltp-$runtime ####"
    cd "/$runtime/ltp/testcases/bin" 2>/dev/null || return

    # Blacklist: tests that are known to fail or hang
    BLACKLIST="mincore01 mprotect02"

    # Whitelist: curated list of basic syscall tests
    WHITELIST="
        abort01 access01 alarm02 alarm03 alarm05 alarm06 alarm07 \
        bind01 bind05 \
        chdir01 chdir04 chmod01 chown01 \
        clock_getres01 clock_gettime02 clock_nanosleep01 \
        clone01 clone03 clone06 \
        close01 close02 \
        creat01 \
        dup01 dup02 dup03 dup04 dup07 dup201 dup202 dup203 dup204 \
        exit_group01 exit02 \
        faccessat01 faccessat02 \
        fchdir01 fchdir02 fchmod01 fchmod02 \
        fchown01 fchown02 fchown03 \
        fcntl02 fcntl03 fcntl04 fcntl05 fcntl08 \
        fdatasync01 fdatasync02 \
        fork01 fork03 fork07 fork08 fork10 \
        fstat02 fstat03 fstatfs01 fstatfs02 \
        fsync01 ftruncate01 \
        futex_wait01 futex_wait02 futex_wake01 futex_wake02 \
        getcwd01 getcwd03 \
        getdents01 getdents02 \
        getegid01 geteuid01 geteuid02 \
        gethostname01 getpagesize01 \
        getpgid01 getpgid02 getpgrp01 \
        getpid01 getpid02 getppid01 getppid02 \
        getpriority01 getpriority02 \
        getrandom01 getrandom02 getrandom03 getrandom04 \
        getrlimit01 getrlimit02 \
        getrusage01 getrusage02 \
        getsid01 getsid02 gettid01 gettid02 \
        gettimeofday01 getuid01 getuid03 \
        kill03 kill06 kill07 kill08 kill09 \
        link02 link04 link05 \
        lseek01 lseek02 lseek07 lseek11 \
        lstat01 lstat02 \
        madvise01 madvise02 madvise03 madvise05 \
        memcmp01 memcpy01 memset01 \
        mkdir05 mkdirat02 \
        mmap02 mmap05 mmap06 mmap09 \
        nanosleep04 \
        open01 open02 open03 open06 open07 open08 open09 open10 open11 \
        openat01 \
        pathconf01 \
        pipe01 pipe02 pipe03 pipe06 pipe07 pipe08 pipe10 pipe11 pipe12 \
        poll01 \
        pread01 pread02 pwrite01 pwrite02 pwrite03 pwrite04 \
        read01 read02 read03 read04 \
        readdir01 readlink01 readlinkat01 \
        rename01 renameat01 \
        rmdir01 rmdir02 rmdir03 \
        rt_sigaction03 rt_sigprocmask01 rt_sigprocmask02 \
        sbrk01 sbrk02 \
        select03 \
        sendfile02 sendfile04 sendfile05 sendfile06 \
        setitimer01 setitimer02 \
        setpgid02 \
        setrlimit02 setrlimit03 setrlimit04 setrlimit05 \
        setuid01 \
        sigaltstack02 signal01 signal02 signal03 signal04 signal05 \
        sigpending02 \
        socket01 socket02 \
        stat01 stat02 stat03 \
        statvfs02 statx01 statx02 statx03 \
        symlink02 symlink04 \
        syscall01 \
        tgkill03 tkill01 tkill02 \
        truncate02 \
        uname01 uname02 uname04 \
        unlink05 unlink07 unlink08 unlink09 unlinkat01 \
        utime06 utime07 utimes01 \
        utsname01 utsname04 \
        wait01 wait02 wait401 wait402 wait403 \
        waitpid01 waitpid03 waitpid04 waitpid06 waitpid07 waitpid08 \
        waitpid10 waitpid11 waitpid12 waitpid13 \
        write01 write02 write03 write04 write05 write06 \
        writev07 \
        splice01 splice03 \
        confstr01 \
        dirtypipe \
        dup3_01 dup3_02 \
        llseek01 llseek02 llseek03 \
        ppoll01 \
        pselect02 pselect03 \
        preadv01 pwritev01 \
        readv01 readv02 \
        shmat01 shmat02 shmat03 shmat04 \
        shmctl01 shmctl03 \
        shmdt01 shmdt02 \
        shmget02 shmget03 shmget04 shmget05 shmget06 \
        fpathconf01 \
        accept01 accept03 \
        fchmodat01 fchmodat02 \
        fchmod03 fchmod04 fchmod05 fchmod06 \
        ioctl04 ioctl05 ioctl06 \
        lstat01_64 lstat02_64 \
        fcntl02_64 fcntl03_64 fcntl04_64 fcntl05_64 fcntl08_64 \
        fstat02_64 fstat03_64 \
        fstatfs01_64 fstatfs02_64 \
        sendfile02_64 sendfile04_64 sendfile05_64 sendfile06_64 \
        stat01_64 stat02_64 stat03_64 \
        truncate02_64 \
        pread01_64 pread02_64 pwrite01_64 pwrite02_64 \
        preadv01_64 pwritev01_64 \
        getdomainname01 \
        execl \
        recvmsg sendmsg \
        vfork01 vfork02 \
        mincore02 \
        mknod01 mknodat02 \
        mount01 mount02 mount03 mount04 \
        pivot_root01 \
        prctl01 prctl02 \
        process_vm_readv01 process_vm_writev01 \
        ptrace01 ptrace03 \
        quotactl01 quotactl02 quotactl03 quotactl04 quotactl05 quotactl06 \
        readdir21 \
        remap_file_pages01 remap_file_pages02 \
        rename02 rename03 rename04 rename05 rename06 rename07 rename08 rename09 rename10 rename11 rename12 rename13 rename14 \
        setdomainname01 setdomainname02 \
        setgid01 setgid02 setgid03 \
        groups01 \
        getgroups01 setgroups01 \
        setregid01 setregid02 \
        setresgid01 setresgid02 setresgid03 \
        setreuid01 setreuid02 setreuid03 setreuid04 setreuid05 setreuid06 setreuid07 \
        setresuid01 setresuid02 setresuid03 setresuid04 setresuid05 \
        mlock01 mlock02 mlock03 mlock201 mlock202 mlock203 mlock04 mlock05 \
        munlock01 munlock02 \
        mprotect01 mprotect02 mprotect03 mprotect04 \
        msync01 msync02 msync03 msync04 \
        brk01 \
        procpcilocator
    "

    for case in $WHITELIST; do
        # Check blacklist
        is_blacklisted=false
        for bl in $BLACKLIST; do
            if [ "$case" = "$bl" ]; then
                is_blacklisted=true
                break
            fi
        done

        if [ "$is_blacklisted" = "true" ]; then
            echo "SKIP LTP CASE $case (blacklisted)"
            continue
        fi

        if [ -f "$case" ]; then
            echo "RUN LTP CASE $case"
            ./$case
            ret=$?
            if [ $ret -eq 0 ]; then
                echo "PASS LTP CASE $case"
            else
                echo "FAIL LTP CASE $case : $ret"
            fi
        fi
    done

    cd /
    echo "#### OS COMP TEST GROUP END ltp-$runtime ####"
}

# =========================================================================
# Run selected test groups
# =========================================================================

echo "=== Custom test mode: running groups: $TEST_GROUPS ==="

for runtime in musl glibc; do
    if [ ! -d "/$runtime" ]; then
        continue
    fi

    echo "=== Running tests for $runtime ==="

    if [ "$runtime" = "glibc" ]; then
        export LD_LIBRARY_PATH=/glibc/lib
    fi

    # Check each requested test group
    for group in $TEST_GROUPS; do
        case "$group" in
            basic)      run_test "$runtime" basic       60 ;;
            busybox)    run_test "$runtime" busybox     60 ;;
            lua)        run_test "$runtime" lua         60 ;;
            libctest)   run_test "$runtime" libctest    90 ;;
            libcbench)  run_test "$runtime" libcbench   90 ;;
            cyclictest) run_test "$runtime" cyclictest  90 ;;
            unixbench)  run_test "$runtime" unixbench   120 ;;
            iozone)     run_test "$runtime" iozone      120 ;;
            lmbench)    run_test "$runtime" lmbench     60 ;;
            iperf)      run_test "$runtime" iperf       120 ;;
            netperf)    run_test "$runtime" netperf     120 ;;
            ltp)        run_ltp "$runtime" ;;
            *)          echo "Warning: unknown test group '$group'" ;;
        esac
    done

    if [ "$runtime" = "glibc" ]; then
        unset LD_LIBRARY_PATH
    fi
done

# =========================================================================
# Shutdown
# =========================================================================
echo "=== Selected tests completed. Shutting down... ==="
/musl/busybox poweroff -f
