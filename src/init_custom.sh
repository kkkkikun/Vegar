#!/bin/sh
# Custom test groups mode
# Edit TEST_GROUPS below to choose which tests to run
# Available: basic, busybox, lua, libctest, libcbench, cyclictest, unixbench, iozone, lmbench, iperf, netperf, ltp

# =========================================================================
# EDIT THIS: Choose test groups to run
# =========================================================================
TEST_GROUPS=""

# ---------- setup (minimal, from init_oscomp.sh) ----------
/musl/busybox mkdir -p /bin /lib /tmp /etc /boot /var/tmp 2>/dev/null
/musl/busybox --install -s /bin 2>/dev/null
export PATH=/bin
ln -s /musl/lib/libc.so /lib/ld-musl-riscv64.so.1 2>/dev/null
ln -s /musl/lib/libc.so /lib/ld-musl-riscv64-sf.so.1 2>/dev/null
ln -s /glibc/lib/ld-linux-riscv64-lp64d.so.1 /lib/ld-linux-riscv64-lp64d.so.1 2>/dev/null
ln -s /glibc/lib/libc.so.6 /lib/libc.so.6 2>/dev/null
ln -s /lib /lib64 2>/dev/null
echo "root:x:0:0:root:/root:/bin/bash" > /etc/passwd 2>/dev/null
echo "root:x:0:" > /etc/group 2>/dev/null
echo "=== setup done ==="

result() {
    if [ "$1" = "0" ]; then echo "PASS"; else echo "FAIL (exit=$1)"; fi
}

echo ""
echo "========== [1] Real liburing tests (unmodified upstream) =========="

run_liburing() {
    t="$1"; shift
    bin="/liburing_$t"
    if [ ! -f "$bin" ]; then echo "  SKIP: $bin not found"; return; fi
    /musl/busybox cp "$bin" /tmp/lt && /musl/busybox chmod +x /tmp/lt
    echo -n "  $t ... "
    out=$(/musl/busybox timeout 30 /tmp/lt "$@" 2>&1)
    rc=$?
    if echo "$out" | /musl/busybox grep -q "PASS"; then
        echo "PASS"
    elif echo "$out" | /musl/busybox grep -q "FAIL"; then
        echo "FAIL"
        echo "$out" | /musl/busybox tail -5 | sed 's/^/    /'
    else
        result $rc
    fi
}

run_liburing io_uring_setup
run_liburing io_uring_enter
# register: SKIP — needs MAP_ANONYMOUS (mmap, not io_uring)
echo "  io_uring_register ... SKIP (MAP_ANONYMOUS)"
run_liburing poll
run_liburing fsync
run_liburing poll-cancel
# ring-leak: SKIP — needs AF_UNIX socketpair (net, not io_uring)
echo "  ring-leak ... SKIP (AF_UNIX)"
# io_uring-test: SKIP — demo program, needs argv[1]
echo "  io_uring-test ... SKIP (needs file arg)"

echo ""
echo "========== [2] Hand-written io_uring tests =========="

run_test() {
    name="$1"
    if [ ! -f "/$name" ]; then echo "  SKIP: /$name not found"; return; fi
    /musl/busybox cp "/$name" /tmp/t && /musl/busybox chmod +x /tmp/t
    echo -n "  $name ... "
    out=$(/musl/busybox timeout 15 /tmp/t 2>&1)
    rc=$?
    if echo "$out" | /musl/busybox grep -q "PASS"; then
        echo "PASS"
    else
        result $rc
        echo "$out" | /musl/busybox tail -3 | sed 's/^/    /'
    fi
}

for t in io_uring_nop io_uring_pipe io_uring_poll io_uring_file \
         io_uring_bench io_uring_batch io_uring_scale io_uring_getevents \
         io_uring_readv io_uring_shim_test io_uring_send_recv io_uring_accept \
         io_uring_iodepth io_uring_echo io_uring_echo_epoll; do
    run_test "$t"
done

echo ""
echo "========== [3] io_uring vs epoll (echo, 24 clients) =========="
for t in io_uring_echo_epoll_fair24 io_uring_echo_epoll24; do
    run_test "$t"
done

echo ""
echo "========== [4] Benchmark: io_uring vs thread-per-conn =========="
run_test io_uring_vs_thread

echo ""
echo "========== all done, poweroff =========="
/musl/busybox poweroff -f
